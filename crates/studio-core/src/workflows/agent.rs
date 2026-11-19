//! The story assistant: one agent that works on the story in small steps —
//! reading, making an exact edit, checking what the compiler says about the
//! whole story, and editing again — until it is done or out of steps.
//!
//! Every edit lands in the writer's draft as it is made, through the host's
//! [`StoryDraft`]; reviewing what changed is the host's. The request's
//! [`Scope`] is enforced by the tools, so a passage request cannot change
//! anything outside its passage whatever the model decides.
mod check;
mod moves;
mod prompt;
mod scope;
#[cfg(test)]
mod tests;
mod tools;

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
};

use agentkit_workflow::{
    Agent, Runner, SharedStreamingModel, StepRegistry, ToolSource, Value, WorkflowEventKind,
    WorkflowRunStatus,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use storyfella_compiler::SourceWorkspace;
use storyfella_language::Span;

pub use scope::{Passage, Scope};

use super::{execute, AssistantSettings, Cancellation, Feedback, RunStore};
use crate::{authoring::AssetCatalog, planner::PlannerBackend, registry::ArtifactRegistry};

/// The tool source's name inside the agent's workflow.
const TOOLS: &str = "story";

/// The story as the writer's editor holds it, unsaved changes included.
#[async_trait::async_trait]
pub trait StoryDraft: Send + Sync {
    /// Every file of the story as it is now.
    async fn workspace(&self) -> Result<SourceWorkspace, String>;

    /// Replaces each of `texts`, creating a file that does not exist yet, as
    /// one change. Each file must still say what it says in `base`; otherwise
    /// nothing is written and the error says which file moved on.
    async fn write(
        &self,
        base: SourceWorkspace,
        texts: BTreeMap<String, String>,
    ) -> Result<(), String>;

    /// Renames `scene` to `to` everywhere the story names it, as one change,
    /// keeping whatever the host ties to the scene — its artwork — with it.
    /// The story must still be as it is in `base`. A draft that cannot do this
    /// says so.
    async fn rename_scene(
        &self,
        base: SourceWorkspace,
        scene: String,
        to: String,
    ) -> Result<(), String> {
        let _ = (base, scene, to);
        Err("This story cannot rename scenes; edit each place that names it.".into())
    }
}

/// Where the writer is in the editor, so that "this beat" or "the selection"
/// means something whatever the scope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorContext {
    pub file: String,
    pub caret: usize,
    /// The selected text's range; `None` when nothing is selected.
    pub selection: Option<Span>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRequest {
    pub instructions: String,
    pub preserve_facts: bool,
    pub scope: Scope,
    pub editor: Option<EditorContext>,
    /// What the previous run said when it ran out of steps, when this run
    /// carries on from it.
    pub continuing: Option<String>,
}

/// How a request ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentOutcome {
    pub run_id: String,
    /// The agent's last word: what it changed, or its answer.
    pub reply: String,
    /// The files it changed, already in the draft.
    pub changed: BTreeSet<String>,
    /// It used every step it was allowed, so the work may be unfinished.
    pub out_of_steps: bool,
    /// The scope as it stands after the run: a passage moved by the edits
    /// made in and around it. Carrying on uses this.
    pub scope: Scope,
}

/// The story a request works on: the writer's draft and the asset catalog
/// its names resolve against.
#[derive(Clone)]
pub struct Story {
    pub draft: Arc<dyn StoryDraft>,
    pub registry: ArtifactRegistry,
}

/// Runs a request against the selected text provider. A managed provider's
/// server answers only on the port the inference service allocated for it, so
/// the allocation lives until the run finishes.
pub async fn run_with_backend(
    request: AgentRequest,
    settings: AssistantSettings,
    backend: &PlannerBackend,
    story: Story,
    store: &RunStore,
    cancel: Cancellation,
    feedback: impl Fn(Feedback) + Send + Sync + 'static,
) -> Result<Option<AgentOutcome>> {
    feedback(Feedback::Status("Starting the text model".into()));
    if let Some(described) = backend.describe() {
        feedback(Feedback::Detail(described));
    }
    let Some(allocation) = super::provider::wait_for_allocation(backend, &cancel).await? else {
        return Ok(None);
    };
    run_with_model(
        request,
        settings,
        super::provider::chat_model(allocation.config()),
        story,
        store,
        cancel,
        feedback,
    )
    .await
}

pub async fn run_with_model(
    request: AgentRequest,
    settings: AssistantSettings,
    model: SharedStreamingModel,
    Story { draft, registry }: Story,
    store: &RunStore,
    cancel: Cancellation,
    feedback: impl Fn(Feedback) + Send + Sync + 'static,
) -> Result<Option<AgentOutcome>> {
    let feedback: tools::Sink = Arc::new(feedback);
    let settings = settings.bounded();
    let workspace = draft.workspace().await.map_err(anyhow::Error::msg)?;
    let before = check::problems(&workspace, &AssetCatalog::from_registry(&registry));
    let task = prompt::request(&request, &workspace, &registry, &before);
    let tools = Arc::new(tools::StoryTools::new(
        draft,
        registry,
        request.scope.clone(),
        &workspace,
        before,
        feedback.clone(),
    ));

    let mut agent = Agent::new("storyfella-assistant")
        .system(prompt::SYSTEM)
        .tools(ToolSource::named(TOOLS))
        .max_iterations(settings.max_steps)
        .takes_conversation("history")
        .answers_as("reply");
    if let Some(thinking) = settings.thinking {
        agent = agent.thinking(thinking);
    }
    if let Some(tokens) = settings.max_output_tokens {
        agent = agent.max_output_tokens(tokens);
    }
    let workflow = agent
        .build(&StepRegistry::builtin())
        .map_err(|error| anyhow::anyhow!("Assistant workflow: {error:?}"))?;
    let model_node = workflow
        .nodes
        .iter()
        .find(|node| node.kind == "language_model")
        .context("The assistant workflow has no model")?
        .id;

    let state = store.create(&workflow, &serde_json::json!({ "request": request }))?;
    let run_id = state.identity.run_id.clone();
    let runner = Runner::with_streaming_model(workflow, model)
        .with_tool_source(TOOLS, tools.clone())
        .with_workflow_inputs(BTreeMap::from([("history".to_owned(), Value::text(task))]))
        .map_err(|error| anyhow::anyhow!("Assistant request: {error:?}"))?;

    feedback(Feedback::Status("Thinking".into()));
    let asked = Arc::new(AtomicU32::new(0));
    let counted = asked.clone();
    let sink = feedback.clone();
    let state = execute(runner, state, store, cancel.clone(), move |event| {
        if let Feedback::Event(event) = &event {
            if matches!(event.event, WorkflowEventKind::NodeStarted { node } if node == model_node)
            {
                counted.fetch_add(1, Ordering::Relaxed);
            }
        }
        sink(event)
    })
    .await?;
    if state.status == WorkflowRunStatus::Cancelled || cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let reply = match state
        .outputs
        .get(&model_node)
        .and_then(|ports| ports.get("reply"))
    {
        Some(Value::Text(text)) => super::reply::without_thinking(text).trim().to_owned(),
        _ => String::new(),
    };
    Ok(Some(AgentOutcome {
        run_id,
        reply,
        changed: tools.changed().await,
        scope: tools.scope().await,
        out_of_steps: asked.load(Ordering::Relaxed) >= settings.max_steps,
    }))
}

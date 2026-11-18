//! What the assistant column asks of the writing workflow: which passage a
//! request is about, and running it off the UI thread. Nothing here draws.
use std::{ops::Range, path::PathBuf};

use storyfella_language::{Source, Span};
use storyfella_studio_core::{
    planner::PlannerBackend,
    registry::ArtifactRegistry,
    workflows::{
        framework::SharedStreamingModel,
        writing::{self, WritingAction, WritingProposal, WritingRequest, WritingScope},
        Cancellation, Feedback, RunStore,
    },
};

use crate::{state::AppState, writing as files};

/// The passage a request is about.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    Selection,
    #[default]
    Beat,
    Scene,
    File,
}

impl Scope {
    pub const ALL: [Self; 4] = [Self::Selection, Self::Beat, Self::Scene, Self::File];

    pub fn label(self) -> &'static str {
        match self {
            Self::Selection => "Selection",
            Self::Beat => "Beat",
            Self::Scene => "Scene",
            Self::File => "File",
        }
    }
}

/// Where the story's workflow runs and conversation are kept.
pub fn store_path(state: &AppState) -> Option<PathBuf> {
    state
        .project_path
        .as_ref()
        .map(|project| project.with_extension("workflows.sqlite3"))
}

/// "Beat · lines 4–9 of chapter.sf", how a request's passage reads.
pub fn describe(scope: Scope, file: &str, source: &str, range: Span) -> String {
    let text = Source::new(source);
    let line = |offset: usize| text.position(offset).map_or(1, |at| at.line + 1);
    let first = line(range.start);
    // A range ending at a line break belongs to the line before it.
    let last = line(range.end.saturating_sub(1).max(range.start));
    if first == last {
        format!("{} · line {first} of {file}", scope.label())
    } else {
        format!("{} · lines {first}–{last} of {file}", scope.label())
    }
}

/// A rewrite of the passage of `file` that `scope` names around the caret,
/// following `instructions`.
pub fn capture(
    state: &AppState,
    file: &str,
    scope: Scope,
    selection: Range<usize>,
    instructions: &str,
    keep_facts: bool,
) -> Result<WritingRequest, String> {
    let project = state.project_path.as_deref().ok_or("Open a story first.")?;
    let folder = state
        .story
        .path()
        .and_then(|entry| entry.parent())
        .ok_or("Save the story first.")?;
    let source = files::source(state, file).ok_or("Open a story file first.")?;
    if source.trim().is_empty() {
        return Err(format!(
            "{file} is empty. The assistant edits passages that exist, so write a first line, \
             then ask."
        ));
    }
    let scope = match scope {
        Scope::Selection if selection.is_empty() => {
            return Err("Select the passage to work on first.".into())
        }
        Scope::Selection => WritingScope::Selection(Span::new(selection.start, selection.end)),
        Scope::Beat => WritingScope::Beat(selection.start),
        Scope::Scene => WritingScope::Scene(selection.start),
        Scope::File => WritingScope::File,
    };
    let mut request =
        WritingRequest::capture(project, &folder.join(file), source, scope, &state.registry)
            .map_err(|error| error.to_string())?;
    request.action = WritingAction::Rewrite;
    request.instructions = instructions.trim().to_owned();
    request.preserve_facts = keep_facts;
    Ok(request)
}

/// What writes the suggestion: the provider chosen in Settings, or a model
/// handed in directly, as tests do.
#[derive(Clone)]
pub enum Engine {
    Provider(PlannerBackend),
    Model(SharedStreamingModel),
}

/// Runs `request` to a proposal on its own runtime; meant for a worker
/// thread. `Ok(None)` means it was cancelled.
pub fn suggest(
    request: WritingRequest,
    engine: Engine,
    registry: ArtifactRegistry,
    store: PathBuf,
    cancel: Cancellation,
    feedback: impl Fn(Feedback) + Send + Sync + 'static,
) -> anyhow::Result<Option<WritingProposal>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let store = RunStore::open(&store)?;
        match engine {
            Engine::Provider(backend) => {
                writing::run_with_backend(request, &backend, &registry, &store, cancel, feedback)
                    .await
            }
            Engine::Model(model) => {
                writing::run_with_model(request, model, &registry, &store, cancel, feedback).await
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_passage_reads_as_its_scope_and_lines() {
        let source = "scene harbor\n  narrate: Dusk.\n  end\n";
        assert_eq!(
            describe(Scope::File, "story.sf", source, Span::new(0, source.len())),
            "File · lines 1–3 of story.sf"
        );
        assert_eq!(
            describe(Scope::Selection, "story.sf", source, Span::new(15, 20)),
            "Selection · line 2 of story.sf"
        );
    }
}

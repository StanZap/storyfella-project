//! Nonblocking, connection-scoped assistant jobs for the live Studio owner.
//! A job runs the same story agent as the assistant column, on its own
//! thread; its reads and edits of the story come back here and are applied
//! to Studio's draft, each edit one undoable change, and wait in Studio's
//! review like the column's own.
use super::{owned, owner, Grant, StateOwner};
use serde_json::json;
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use storyfella_studio_core::{
    config::AppConfig,
    desktop_bridge::{AssistantCommand, Failure, Reply, Request},
    planner::PlannerBackend,
    workflows::{
        agent::{AgentOutcome, AgentRequest, Scope},
        AssistantSettings,
    },
};
use uuid::Uuid;

use crate::assistant::{self, DraftCall, Engine, Heard};

#[derive(Default)]
pub(super) struct Jobs {
    jobs: Rc<RefCell<HashMap<Uuid, Job>>>,
    #[cfg(test)]
    model: Option<storyfella_studio_core::workflows::framework::SharedStreamingModel>,
}

struct Job {
    session: Uuid,
    epoch: Uuid,
    cancel: Arc<AtomicBool>,
    result: Option<Result<Option<AgentOutcome>, String>>,
}

impl Jobs {
    pub(super) async fn dispatch(
        &mut self,
        state: &(impl StateOwner + Clone + 'static),
        grant: &Grant,
        request: Request<AssistantCommand>,
        session: Uuid,
        epoch: Option<Uuid>,
    ) -> (Option<Uuid>, Reply) {
        let result = async {
            request.check_project(&grant.folder)?;
            let supplied = match &request.command {
                AssistantCommand::Start { session, .. }
                | AssistantCommand::Poll { session, .. }
                | AssistantCommand::Cancel { session, .. } => *session,
            };
            if Some(supplied) != Some(session) || epoch.is_none() {
                return Err(Failure::new(
                    "session_changed",
                    "Inspect the project on this MCP connection first.",
                ));
            }
            if !grant.edit {
                return Err(Failure::new(
                    "scope_denied",
                    "Enable draft edits when opening the Studio channel.",
                ));
            }
            let folder = grant.folder.clone();
            let current = owned(state, move |state| owner(state, &folder, epoch)).await?;
            match request.command {
                AssistantCommand::Start { instructions, preserve_facts, .. } => {
                    if instructions.len() > 8192 { return Err(Failure::new("invalid_arguments", "Use at most 8192 bytes of instructions.")); }
                    let (store, registry) = owned(state, |app| {
                        if app.operation_in_progress() || app.catalog_conflict.is_some() { return Err(Failure::new("owner_busy", "Finish the current operation or resolve the catalog conflict.")); }
                        app.authoring_lease.as_ref().expect("owner checked lease").verify().map_err(|_| super::moved())?;
                        let store = assistant::store_path(app).ok_or_else(super::moved)?;
                        Ok((store, app.registry.clone()))
                    }).await?;
                    let (engine, settings) = self.engine()?;
                    let mut jobs = self.jobs.borrow_mut();
                    jobs.retain(|_, job| {
                        if job.epoch != current { job.cancel.store(true, Ordering::Relaxed); return false; }
                        !matches!(job.result, Some(Ok(None)) | Some(Err(_)))
                    });
                    if jobs.len() >= 8 { return Err(Failure::new("owner_busy", "Finish or cancel an existing assistant job.")); }
                    let id = Uuid::new_v4();
                    let cancel = Arc::new(AtomicBool::new(false));
                    jobs.insert(id, Job { session, epoch: current, cancel: cancel.clone(), result: None });
                    drop(jobs);
                    let request = AgentRequest { instructions, preserve_facts, scope: Scope::Project, editor: None, continuing: None };
                    let (heard, mut hearing) = tokio::sync::mpsc::unbounded_channel();
                    let worker = assistant::spawn(request, settings, engine, registry, store, cancel, heard)
                        .map_err(|error| Failure::new("owner_busy", &format!("The assistant could not start: {error}")))?;
                    let jobs = self.jobs.clone();
                    let state = state.clone();
                    tokio::task::spawn_local(async move {
                        while let Some(heard) = hearing.recv().await {
                            if let Heard::Draft(call) = heard {
                                serve(&state, call).await;
                            }
                        }
                        let result = match worker.await {
                            Ok(result) => result.map_err(|error| format!("{error:#}")),
                            Err(_) => Err("The assistant stopped unexpectedly.".into()),
                        };
                        if let Some(job) = jobs.borrow_mut().get_mut(&id) { job.result = Some(result); }
                    });
                    Ok(json!({"job":id,"status":"running"}))
                }
                AssistantCommand::Poll { job, .. } => {
                    let jobs = self.jobs.borrow();
                    let job = jobs.get(&job).filter(|job| job.session == session && job.epoch == current).ok_or_else(|| Failure::new("unknown_job", "Start an assistant job on this connection."))?;
                    Ok(match &job.result {
                        None => json!({"status":"running"}),
                        Some(Ok(Some(outcome))) => json!({"status":"done", "message":outcome.reply, "files":outcome.changed, "out_of_steps":outcome.out_of_steps, "run_id":outcome.run_id}),
                        Some(Ok(None)) => json!({"status":"cancelled"}),
                        Some(Err(error)) => json!({"status":"failed", "message":error}),
                    })
                }
                AssistantCommand::Cancel { job, .. } => {
                    let mut jobs = self.jobs.borrow_mut();
                    let job = jobs.get_mut(&job).filter(|job| job.session == session && job.epoch == current).ok_or_else(|| Failure::new("unknown_job", "Start an assistant job on this connection."))?;
                    job.cancel.store(true, Ordering::Relaxed);
                    let finished = job.result.is_some();
                    if finished { job.result = Some(Ok(None)); }
                    Ok(json!({"status":if finished { "cancelled" } else { "cancelling" }}))
                }
            }.map(|value| (current, value))
        }.await;
        match result {
            Ok((current, value)) => (Some(current), Ok(value)),
            Err(error) => (epoch, Err(error)),
        }
    }

    /// The model a job writes with, and how hard it may try.
    fn engine(&self) -> Result<(Engine, AssistantSettings), Failure> {
        #[cfg(test)]
        if let Some(model) = &self.model {
            return Ok((Engine::Model(model.clone()), AssistantSettings::default()));
        }
        let config = AppConfig::load()
            .map_err(|error| Failure::new("provider_unavailable", &error.to_string()))?;
        let backend = PlannerBackend::new(&config);
        if !backend.has_llm() {
            return Err(Failure::new(
                "provider_unavailable",
                "Choose a text provider in Studio Settings.",
            ));
        }
        Ok((Engine::Provider(Box::new(backend)), config.assistant))
    }
}

/// Answers a running job's call on the story from Studio's draft. Once
/// Studio has closed the call goes unanswered, which the job reports.
async fn serve(state: &impl StateOwner, call: DraftCall) {
    state.with(move |app| call.answer(app)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use std::path::PathBuf;
    use storyfella_studio_core::{
        desktop_bridge::{ProjectIdentity, SCHEMA},
        project::ProjectFolder,
        registry::ArtifactRegistry,
    };

    async fn running_probe(root: PathBuf) {
        let state = Rc::new(RefCell::new(AppState::default()));
        let folder = ProjectFolder::create(
            &root.join("project"),
            "Novel",
            &ArtifactRegistry::default(),
            "scene harbor\n  narrate: old\n  end\n",
        )
        .unwrap();
        crate::commands::open_project(folder.root(), &mut state.borrow_mut()).unwrap();
        let grant = Grant {
            folder: folder.clone(),
            directory: root.join("channel"),
            edit: true,
        };
        let session = Uuid::new_v4();
        let epoch = state.borrow().authoring_lease.as_ref().unwrap().epoch();
        let edit = serde_json::json!({
            "path": "story.sf",
            "old_text": "narrate: old",
            "new_text": "narrate: new"
        });
        let model = agentkit_ai::providers::fake::FakeChatModel::new_streaming([
            vec![
                agentkit_ai::ChatStreamEvent::ToolCallDelta {
                    index: 0,
                    id: Some("call_edit".into()),
                    name: Some("edit_file".into()),
                    arguments: Some(edit.to_string()),
                },
                agentkit_ai::ChatStreamEvent::Done,
            ],
            vec![
                agentkit_ai::ChatStreamEvent::TextDelta {
                    content: "I renewed the narration.".into(),
                },
                agentkit_ai::ChatStreamEvent::Done,
            ],
        ]);
        let mut jobs = Jobs {
            model: Some(Arc::new(model)),
            ..Default::default()
        };
        let request = |command| Request {
            schema: SCHEMA,
            project: ProjectIdentity::from(&folder),
            command,
        };
        let (_, started) = jobs
            .dispatch(
                &state,
                &grant,
                request(AssistantCommand::Start {
                    session,
                    instructions: "revise".into(),
                    preserve_facts: true,
                }),
                session,
                Some(epoch),
            )
            .await;
        let id = started.unwrap()["job"].as_str().unwrap().parse().unwrap();
        loop {
            let (_, reply) = jobs
                .dispatch(
                    &state,
                    &grant,
                    request(AssistantCommand::Poll { session, job: id }),
                    session,
                    Some(epoch),
                )
                .await;
            let reply = reply.unwrap();
            if reply["status"] == "done" {
                assert_eq!(reply["message"], "I renewed the narration.");
                assert_eq!(reply["files"], serde_json::json!(["story.sf"]));
                assert_eq!(reply["out_of_steps"], false);
                break;
            }
            assert_eq!(reply["status"], "running", "{reply}");
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(state.borrow().story.source().contains("new"));
        assert_eq!(
            state.borrow().assistant_changes,
            std::collections::BTreeMap::from([(
                "story.sf".to_owned(),
                Some("scene harbor\n  narrate: old\n  end\n".to_owned())
            )]),
            "the edit waits for a decision in Studio"
        );
        assert!(state.borrow_mut().undo());
        assert!(state.borrow().story.source().contains("old"));
    }

    #[tokio::test]
    async fn a_live_job_edits_the_draft_as_it_runs_and_each_edit_is_undoable() {
        let scratch = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
        std::fs::create_dir_all(&scratch).unwrap();
        let dir = tempfile::tempdir_in(scratch).unwrap();
        // Started jobs run on the bridge's local task set.
        let local = tokio::task::LocalSet::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            local.run_until(running_probe(dir.path().to_owned())),
        )
        .await
        .unwrap();
    }
}

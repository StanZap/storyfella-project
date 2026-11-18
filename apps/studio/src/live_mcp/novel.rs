//! Nonblocking, connection-scoped novel-writing jobs for the live Studio owner.
use super::{owner, Grant};
use crate::state::AppState;
use dioxus::prelude::*;
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
    desktop_bridge::{Failure, NovelCommand, Reply, Request},
    planner::PlannerBackend,
    workflows::{
        novel::{self, NovelProposal, NovelRequest},
        writing::WritingAction,
        RunStore,
    },
};
use uuid::Uuid;

#[derive(Default)]
pub(super) struct Jobs {
    jobs: Rc<RefCell<HashMap<Uuid, Job>>>,
    #[cfg(test)]
    model: Option<Arc<dyn agentkit_ai::StreamingChatModel + Send + Sync>>,
}

struct Job {
    session: Uuid,
    epoch: Uuid,
    cancel: Arc<AtomicBool>,
    result: Option<Result<Option<NovelProposal>, String>>,
}

impl Jobs {
    pub(super) fn dispatch(
        &mut self,
        mut state: Signal<AppState>,
        grant: &Grant,
        request: Request<NovelCommand>,
        session: Uuid,
        epoch: Option<Uuid>,
    ) -> (Option<Uuid>, Reply) {
        let result = (|| {
            request.check_project(&grant.folder)?;
            let supplied = match &request.command {
                NovelCommand::Start { session, .. }
                | NovelCommand::Poll { session, .. }
                | NovelCommand::Apply { session, .. }
                | NovelCommand::Cancel { session, .. } => *session,
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
            let current = owner(&state.peek(), grant, epoch)?;
            match request.command {
                NovelCommand::Start { instructions, expand, preserve_facts, .. } => {
                    if instructions.len() > 8192 { return Err(Failure::new("invalid_arguments", "Use at most 8192 bytes of instructions.")); }
                    let app = state.peek();
                    if app.operation_in_progress() || app.catalog_conflict.is_some() { return Err(Failure::new("owner_busy", "Finish the current operation or resolve the catalog conflict.")); }
                    app.authoring_lease.as_ref().expect("owner checked lease").verify().map_err(|_| super::moved())?;
                    let project = app.project_path.clone().ok_or_else(super::moved)?;
                    let workspace = app.workspace_snapshot().map_err(|error| Failure::new("source_unavailable", &error))?;
                    let registry = app.registry.clone();
                    let catalog_digest = novel::catalog_digest(&registry).map_err(|error| Failure::new("source_unavailable", &error.to_string()))?;
                    drop(app);
                    #[cfg(test)]
                    let model = self.model.clone();
                    #[cfg(not(test))]
                    let model: Option<Arc<dyn agentkit_ai::StreamingChatModel + Send + Sync>> = None;
                    let backend = if model.is_none() {
                        let config = AppConfig::load().map_err(|error| Failure::new("provider_unavailable", &error.to_string()))?;
                        let backend = PlannerBackend::new(&config);
                        if !backend.has_llm() { return Err(Failure::new("provider_unavailable", "Choose a text provider in Studio Settings.")); }
                        Some(backend)
                    } else { None };
                    let mut jobs = self.jobs.borrow_mut();
                    jobs.retain(|_, job| {
                        if job.epoch != current { job.cancel.store(true, Ordering::Relaxed); return false; }
                        !matches!(job.result, Some(Ok(None)) | Some(Err(_)))
                    });
                    if jobs.len() >= 8 { return Err(Failure::new("owner_busy", "Finish or cancel an existing novel-writing job.")); }
                    let id = Uuid::new_v4();
                    let cancel = Arc::new(AtomicBool::new(false));
                    jobs.insert(id, Job { session, epoch: current, cancel: cancel.clone(), result: None });
                    drop(jobs);
                    let jobs = self.jobs.clone();
                    spawn(async move {
                        let request = NovelRequest { project: project.clone(), workspace, action: if expand { WritingAction::Expand } else { WritingAction::Rewrite }, instructions, preserve_facts, catalog_digest };
                        let result = match RunStore::open(&project.with_extension("workflows.sqlite3")) {
                            Ok(store) => match model {
                                Some(model) => novel::run_with_model(request, model, &registry, &store, cancel, |_| {}).await,
                                None => novel::run_with_backend(request, backend.as_ref().expect("configured backend"), &registry, &store, cancel, |_| {}).await,
                            }.map_err(|error| error.to_string()),
                            Err(error) => Err(error.to_string()),
                        };
                        if let Some(job) = jobs.borrow_mut().get_mut(&id) { job.result = Some(result); }
                    });
                    Ok(json!({"job":id,"status":"running"}))
                }
                NovelCommand::Poll { job, .. } => {
                    let jobs = self.jobs.borrow();
                    let job = jobs.get(&job).filter(|job| job.session == session && job.epoch == current).ok_or_else(|| Failure::new("unknown_job", "Start a novel-writing job on this connection."))?;
                    Ok(match &job.result {
                        None => json!({"status":"running"}),
                        Some(Ok(Some(proposal))) => json!({"status":"ready", "files":proposal.files, "run_id":proposal.run_id}),
                        Some(Ok(None)) => json!({"status":"cancelled"}),
                        Some(Err(error)) => json!({"status":"failed", "message":error}),
                    })
                }
                NovelCommand::Cancel { job, .. } => {
                    let mut jobs = self.jobs.borrow_mut();
                    let job = jobs.get_mut(&job).filter(|job| job.session == session && job.epoch == current).ok_or_else(|| Failure::new("unknown_job", "Start a novel-writing job on this connection."))?;
                    job.cancel.store(true, Ordering::Relaxed);
                    let finished = job.result.is_some();
                    if finished { job.result = Some(Ok(None)); }
                    Ok(json!({"status":if finished { "cancelled" } else { "cancelling" }}))
                }
                NovelCommand::Apply { job, .. } => {
                    let proposal = {
                        let jobs = self.jobs.borrow();
                        let job = jobs.get(&job).filter(|job| job.session == session && job.epoch == current).ok_or_else(|| Failure::new("unknown_job", "Start a novel-writing job on this connection."))?;
                        match &job.result { Some(Ok(Some(proposal))) => proposal.clone(), _ => return Err(Failure::new("job_not_ready", "Wait for a completed suggestion.")) }
                    };
                    state.write().apply_novel_proposal(&proposal).map_err(|error| Failure::new("stale_or_invalid", &error))?;
                    self.jobs.borrow_mut().remove(&job);
                    Ok(json!({"applied":true,"saved":false,"files":proposal.files.keys().collect::<Vec<_>>() }))
                }
            }.map(|value| (current, value))
        })();
        match result {
            Ok((current, value)) => (Some(current), Ok(value)),
            Err(error) => (epoch, Err(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, path::PathBuf};
    use storyfella_studio_core::{
        desktop_bridge::{ProjectIdentity, SCHEMA},
        project::ProjectFolder,
        registry::ArtifactRegistry,
    };

    #[component]
    fn Probe(root: PathBuf, finished: Rc<Cell<bool>>) -> Element {
        let mut state = use_signal(AppState::default);
        use_hook(move || {
            let folder = ProjectFolder::create(
                &root.join("project"),
                "Novel",
                &ArtifactRegistry::default(),
                "include \"chapter.sf\"\n",
            )
            .unwrap();
            std::fs::write(
                folder.root().join("assets/chapter.sf"),
                "scene harbor\n  narrate: old\n  end\n",
            )
            .unwrap();
            crate::commands::open_project(folder.root(), state).unwrap();
            let grant = Grant {
                folder: folder.clone(),
                directory: root.join("channel"),
                edit: true,
            };
            let session = Uuid::new_v4();
            let epoch = state.read().authoring_lease.as_ref().unwrap().epoch();
            let workspace = state.read().workspace_snapshot().unwrap();
            let request = NovelRequest {
                project: state.read().project_path.clone().unwrap(),
                workspace,
                action: WritingAction::Rewrite,
                instructions: "change it".into(),
                preserve_facts: true,
                catalog_digest: novel::catalog_digest(&state.read().registry).unwrap(),
            };
            let proposal = NovelProposal {
                run_id: "fixture".into(),
                request,
                files: std::collections::BTreeMap::from([(
                    "chapter.sf".into(),
                    "scene harbor\n  narrate: new\n  end\n".into(),
                )]),
            };
            let id = Uuid::new_v4();
            let mut jobs = Jobs::default();
            jobs.jobs.borrow_mut().insert(
                id,
                Job {
                    session,
                    epoch,
                    cancel: Arc::new(AtomicBool::new(false)),
                    result: Some(Ok(Some(proposal))),
                },
            );
            let request = |command| Request {
                schema: SCHEMA,
                project: ProjectIdentity::from(&folder),
                command,
            };
            let poll = NovelCommand::Poll { session, job: id };
            let (_, result) = jobs.dispatch(state, &grant, request(poll), session, Some(epoch));
            assert_eq!(result.unwrap()["status"], "ready");
            assert!(state.read().workspace_snapshot().unwrap().files["chapter.sf"].contains("old"));
            let apply = NovelCommand::Apply { session, job: id };
            let (_, result) = jobs.dispatch(state, &grant, request(apply), session, Some(epoch));
            assert_eq!(result.unwrap()["applied"], true);
            assert!(state.read().workspace_snapshot().unwrap().files["chapter.sf"].contains("new"));
            assert!(state.write().undo());
            assert!(state.read().workspace_snapshot().unwrap().files["chapter.sf"].contains("old"));
            finished.set(true);
        });
        rsx! {}
    }

    #[test]
    fn live_job_review_applies_only_after_explicit_request_and_is_undoable() {
        let scratch = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
        std::fs::create_dir_all(&scratch).unwrap();
        let dir = tempfile::tempdir_in(scratch).unwrap();
        let finished = Rc::new(Cell::new(false));
        let mut dom = VirtualDom::new_with_props(
            Probe,
            ProbeProps {
                root: dir.path().to_owned(),
                finished: finished.clone(),
            },
        );
        dom.rebuild_in_place();
        assert!(finished.get());
    }

    #[component]
    fn RunningProbe(root: PathBuf, finished: Rc<Cell<bool>>) -> Element {
        let state = use_signal(AppState::default);
        use_future(move || {
            let root = root.clone();
            let finished = finished.clone();
            async move {
                let folder = ProjectFolder::create(
                    &root.join("project"),
                    "Novel",
                    &ArtifactRegistry::default(),
                    "scene harbor\n  narrate: old\n  end\n",
                )
                .unwrap();
                crate::commands::open_project(folder.root(), state).unwrap();
                let grant = Grant {
                    folder: folder.clone(),
                    directory: root.join("channel"),
                    edit: true,
                };
                let session = Uuid::new_v4();
                let epoch = state.read().authoring_lease.as_ref().unwrap().epoch();
                let model = agentkit_ai::providers::fake::FakeChatModel::new_streaming([vec![
                    agentkit_ai::ChatStreamEvent::TextDelta {
                        content:
                            r#"{"files":{"story.sf":"scene harbor\n  narrate: new\n  end\n"}}"#
                                .into(),
                    },
                    agentkit_ai::ChatStreamEvent::Done,
                ]]);
                let mut jobs = Jobs {
                    model: Some(Arc::new(model)),
                    ..Default::default()
                };
                let request = |command| Request {
                    schema: SCHEMA,
                    project: ProjectIdentity::from(&folder),
                    command,
                };
                let (_, started) = jobs.dispatch(
                    state,
                    &grant,
                    request(NovelCommand::Start {
                        session,
                        instructions: "revise".into(),
                        expand: false,
                        preserve_facts: true,
                    }),
                    session,
                    Some(epoch),
                );
                let id = started.unwrap()["job"].as_str().unwrap().parse().unwrap();
                loop {
                    let (_, reply) = jobs.dispatch(
                        state,
                        &grant,
                        request(NovelCommand::Poll { session, job: id }),
                        session,
                        Some(epoch),
                    );
                    let reply = reply.unwrap();
                    if reply["status"] == "ready" {
                        assert!(reply["files"]["story.sf"].as_str().unwrap().contains("new"));
                        break;
                    }
                    assert_eq!(reply["status"], "running", "{reply}");
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
                assert!(state.read().story.source().contains("old"));
                let (_, applied) = jobs.dispatch(
                    state,
                    &grant,
                    request(NovelCommand::Apply { session, job: id }),
                    session,
                    Some(epoch),
                );
                assert_eq!(applied.unwrap()["applied"], true);
                assert!(state.read().story.source().contains("new"));
                finished.set(true);
            }
        });
        rsx! {}
    }

    #[tokio::test]
    async fn live_job_runs_the_workflow_before_review_and_apply() {
        let scratch = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
        std::fs::create_dir_all(&scratch).unwrap();
        let dir = tempfile::tempdir_in(scratch).unwrap();
        let finished = Rc::new(Cell::new(false));
        let mut dom = VirtualDom::new_with_props(
            RunningProbe,
            RunningProbeProps {
                root: dir.path().to_owned(),
                finished: finished.clone(),
            },
        );
        dom.rebuild_in_place();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !finished.get() {
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_millis(5)) => {},
                    _ = dom.wait_for_work() => dom.render_immediate(&mut dioxus::core::NoOpMutations),
                }
            }
        }).await.unwrap();
    }
}

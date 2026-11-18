//! What the assistant column asks of the writing workflows: which passage —
//! or the whole project — a request is about, and running it off the UI
//! thread. Nothing here draws.
use std::{ops::Range, path::PathBuf};

use storyfella_language::{Source, Span};
use storyfella_studio_core::{
    planner::PlannerBackend,
    registry::ArtifactRegistry,
    workflows::{
        assistant::{Place, Suggestion},
        framework::SharedStreamingModel,
        novel::{self, NovelRequest, NovelResult},
        writing::{self, WritingAction, WritingRequest, WritingResult, WritingScope},
        AssistantSettings, Cancellation, Feedback, RunStore,
    },
};

use crate::{state::AppState, writing as files};

/// What a request is about: a passage of the open file, or every file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    Selection,
    #[default]
    Beat,
    Scene,
    File,
    Project,
}

impl Scope {
    pub const ALL: [Self; 5] = [
        Self::Selection,
        Self::Beat,
        Self::Scene,
        Self::File,
        Self::Project,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Selection => "Selection",
            Self::Beat => "Beat",
            Self::Scene => "Scene",
            Self::File => "File",
            Self::Project => "Project",
        }
    }
}

/// A request as sent: one passage of a file, or the whole project.
#[derive(Clone, Debug)]
pub enum Request {
    Passage {
        file: String,
        request: WritingRequest,
    },
    Project(NovelRequest),
}

impl Request {
    /// How the request's reach reads under it, such as "Beat · lines 4–9 of
    /// chapter.sf" or "Project · 3 files".
    pub fn describe(&self, scope: Scope) -> String {
        match self {
            Self::Passage { file, request } => {
                describe(scope, file, &request.source, request.range)
            }
            Self::Project(request) => match request.workspace.files.len() {
                1 => "Project · 1 file".into(),
                count => format!("Project · {count} files"),
            },
        }
    }

    /// Where the passage begins; none for the whole project.
    pub fn place(&self) -> Option<Place> {
        match self {
            Self::Passage { file, request } => Some(Place {
                file: file.clone(),
                offset: request.range.start,
            }),
            Self::Project(_) => None,
        }
    }
}

/// How a request was answered.
#[derive(Clone, Debug)]
pub enum Answered {
    Edit(Box<Suggestion>),
    Reply(String),
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

/// A request about what `scope` names: the passage of `file` around the
/// caret, or the whole project, following `instructions`.
pub fn capture(
    state: &AppState,
    file: &str,
    scope: Scope,
    selection: Range<usize>,
    instructions: &str,
    keep_facts: bool,
) -> Result<Request, String> {
    if scope == Scope::Project {
        return capture_project(state, instructions, keep_facts).map(Request::Project);
    }
    capture_passage(state, file, scope, selection, instructions, keep_facts).map(|request| {
        Request::Passage {
            file: file.to_owned(),
            request,
        }
    })
}

/// Every file of the story as it stands in the editor, drafts included.
pub fn capture_project(
    state: &AppState,
    instructions: &str,
    keep_facts: bool,
) -> Result<NovelRequest, String> {
    let project = state.project_path.clone().ok_or("Open a story first.")?;
    let workspace = state.workspace_snapshot()?;
    Ok(NovelRequest {
        project,
        workspace,
        action: WritingAction::Rewrite,
        instructions: instructions.trim().to_owned(),
        preserve_facts: keep_facts,
        catalog_digest: novel::catalog_digest(&state.registry)
            .map_err(|error| error.to_string())?,
    })
}

/// A rewrite of the passage of `file` that `scope` names around the caret.
pub fn capture_passage(
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
        Scope::File | Scope::Project => WritingScope::File,
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

/// The stack of the thread a request runs on. The workflow, its HTTP client
/// and logging nest deeply enough to overflow an executor thread's stack,
/// which is 512 KiB on macOS.
const WORKER_STACK: usize = 16 * 1024 * 1024;

/// What a request ended in: `Ok(None)` when it was cancelled.
pub type Finished = anyhow::Result<Option<Answered>>;

/// Runs `request` to a suggestion or a reply on a thread of its own, whose
/// answer arrives on the returned channel.
pub fn spawn(
    request: Request,
    settings: AssistantSettings,
    engine: Engine,
    registry: ArtifactRegistry,
    store: PathBuf,
    cancel: Cancellation,
    feedback: impl Fn(Feedback) + Send + Sync + 'static,
) -> std::io::Result<tokio::sync::oneshot::Receiver<Finished>> {
    let (send, receive) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("assistant".into())
        .stack_size(WORKER_STACK)
        .spawn(move || {
            let _ = send.send(suggest(
                request, settings, engine, registry, store, cancel, feedback,
            ));
        })?;
    Ok(receive)
}

fn suggest(
    request: Request,
    settings: AssistantSettings,
    engine: Engine,
    registry: ArtifactRegistry,
    store: PathBuf,
    cancel: Cancellation,
    feedback: impl Fn(Feedback) + Send + Sync + 'static,
) -> Finished {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let store = RunStore::open(&store)?;
        Ok(match request {
            Request::Passage { file, request } => {
                let result = match engine {
                    Engine::Provider(backend) => {
                        writing::run_with_backend(
                            request, settings, &backend, &registry, &store, cancel, feedback,
                        )
                        .await?
                    }
                    Engine::Model(model) => {
                        writing::run_with_model(
                            request, settings, model, &registry, &store, cancel, feedback,
                        )
                        .await?
                    }
                };
                result.map(|result| match result {
                    WritingResult::Proposal(proposal) => {
                        Answered::Edit(Box::new(Suggestion::Passage { file, proposal }))
                    }
                    WritingResult::Reply { text, .. } => Answered::Reply(text),
                })
            }
            Request::Project(request) => {
                let result = match engine {
                    Engine::Provider(backend) => {
                        novel::run_with_backend(
                            request, settings, &backend, &registry, &store, cancel, feedback,
                        )
                        .await?
                    }
                    Engine::Model(model) => {
                        novel::run_with_model(
                            request, settings, model, &registry, &store, cancel, feedback,
                        )
                        .await?
                    }
                };
                result.map(|result| match result {
                    NovelResult::Proposal(proposal) => {
                        Answered::Edit(Box::new(Suggestion::Project(proposal)))
                    }
                    NovelResult::Reply { text, .. } => Answered::Reply(text),
                })
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::{atomic::AtomicBool, Arc},
        time::Duration,
    };

    use agentkit_ai::providers::openai::OpenAiCompatibleChatModel;

    use super::*;

    /// Answers one chat completion with `reply`, streamed.
    fn serve(listener: TcpListener, reply: &'static str) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(&stream);
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some((key, value)) = line.split_once(':') {
                    if key.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap();
                    }
                }
            }
            reader.read_exact(&mut vec![0; length]).unwrap();
            let delta = serde_json::json!({"id": "reply", "choices": [{"index": 0, "delta": {"role": "assistant", "content": reply}, "finish_reason": null}]});
            let done = serde_json::json!({"id": "reply", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]});
            let body = format!("data: {delta}\n\ndata: {done}\n\ndata: [DONE]\n\n");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        })
    }

    #[test]
    fn a_request_is_answered_over_http_on_its_own_thread() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = serve(listener, "Hello! Which scene should change?");
        let directory = tempfile::tempdir().unwrap();
        let source = "scene harbor\n  narrate: Dusk.\n  end\n";
        let registry = ArtifactRegistry::new();
        let request = Request::Passage {
            file: "story.sf".into(),
            request: WritingRequest::capture(
                &directory.path().join("story.db"),
                &directory.path().join("story.sf"),
                source,
                WritingScope::File,
                &registry,
            )
            .unwrap(),
        };
        let model = OpenAiCompatibleChatModel::new(endpoint, "writing-model")
            .with_timeout(Duration::from_secs(5));
        let answer = spawn(
            request,
            AssistantSettings::default(),
            Engine::Model(Arc::new(model)),
            registry,
            directory.path().join("runs.sqlite3"),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap()
        .blocking_recv()
        .unwrap();
        server.join().unwrap();
        assert!(
            matches!(answer, Ok(Some(Answered::Reply(ref text))) if text == "Hello! Which scene should change?"),
            "{answer:?}"
        );
    }

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

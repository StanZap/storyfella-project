//! What the assistant column asks of the story agent: what a request may
//! change, where the writer is, and running it off the UI thread. The agent's
//! edits come back to the UI thread as [`DraftCall`]s, which the column
//! applies to the story the editor holds. Nothing here draws.
use std::{collections::BTreeMap, ops::Range, path::PathBuf, sync::Arc};

use storyfella_compiler::SourceWorkspace;
use storyfella_language::{Source, Span};
use storyfella_studio_core::{
    planner::PlannerBackend,
    registry::ArtifactRegistry,
    workflows::{
        agent::{self, AgentOutcome, AgentRequest, EditorContext, Passage, Story, StoryDraft},
        assistant::Place,
        framework::SharedStreamingModel,
        AssistantSettings, Cancellation, Feedback, RunStore,
    },
};
use tokio::sync::{mpsc, oneshot};

use crate::{state::AppState, writing as files};

/// What a request may change: a passage of the open file, the file, or
/// everything.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    Selection,
    Beat,
    Scene,
    File,
    #[default]
    Project,
}

impl Scope {
    pub const ALL: [Self; 5] = [
        Self::Project,
        Self::File,
        Self::Scene,
        Self::Beat,
        Self::Selection,
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

/// A request as sent, with how its reach reads under it and where its
/// passage began.
#[derive(Clone, Debug)]
pub struct Request {
    pub agent: AgentRequest,
    /// "Beat · lines 4–9 of chapter.sf" or "Project · 3 files".
    pub describe: String,
    pub place: Option<Place>,
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

/// A request about what `scope` names around the caret in `file`,
/// following `instructions`.
pub fn capture(
    state: &AppState,
    file: &str,
    scope: Scope,
    selection: Range<usize>,
    instructions: &str,
    keep_facts: bool,
) -> Result<Request, String> {
    state.project_path.as_ref().ok_or("Open a story first.")?;
    let source = files::source(state, file).ok_or("Open a story file first.")?;
    let (agent_scope, describe, place) = match scope {
        Scope::Project => {
            let count = state.workspace_snapshot()?.files.len();
            let describe = match count {
                1 => "Project · 1 file".to_owned(),
                count => format!("Project · {count} files"),
            };
            (agent::Scope::Project, describe, None)
        }
        Scope::File => (
            agent::Scope::File {
                file: file.to_owned(),
            },
            describe(scope, file, source, Span::new(0, source.len())),
            Some(Place {
                file: file.to_owned(),
                offset: 0,
            }),
        ),
        Scope::Selection | Scope::Beat | Scope::Scene => {
            let passage = match scope {
                Scope::Selection => Passage::Selection,
                Scope::Beat => Passage::Beat,
                _ => Passage::Scene,
            };
            let agent_scope = agent::Scope::passage(passage, file, source, selection.clone())
                .map_err(|error| error.to_string())?;
            let agent::Scope::Passage { span, .. } = &agent_scope else {
                return Err("The passage could not be found.".into());
            };
            let (describe, place) = (
                describe(scope, file, source, *span),
                Some(Place {
                    file: file.to_owned(),
                    offset: span.start,
                }),
            );
            (agent_scope, describe, place)
        }
    };
    Ok(Request {
        agent: AgentRequest {
            instructions: instructions.trim().to_owned(),
            preserve_facts: keep_facts,
            scope: agent_scope,
            editor: Some(EditorContext {
                file: file.to_owned(),
                caret: selection.start,
                selection: (selection.start < selection.end)
                    .then(|| Span::new(selection.start, selection.end)),
            }),
            continuing: None,
        },
        describe,
        place,
    })
}

/// What writes the story: the provider chosen in Settings, or a model handed
/// in directly, as tests do.
#[derive(Clone)]
pub enum Engine {
    Provider(PlannerBackend),
    Model(SharedStreamingModel),
}

/// What the running agent asks of the UI thread.
pub enum DraftCall {
    /// The story as the editor holds it.
    Workspace(oneshot::Sender<Result<SourceWorkspace, String>>),
    /// Replace these files, each still as in `base`, as one undoable change.
    Write {
        base: SourceWorkspace,
        texts: BTreeMap<String, String>,
        done: oneshot::Sender<Result<(), String>>,
    },
}

/// Everything a running request sends the UI thread, in the order it
/// happened.
pub enum Heard {
    Feedback(Feedback),
    Draft(DraftCall),
}

/// The editor's story, reached from the agent's thread.
struct Bridge(mpsc::UnboundedSender<Heard>);

const CLOSED: &str = "Studio closed the story.";

#[async_trait::async_trait]
impl StoryDraft for Bridge {
    async fn workspace(&self) -> Result<SourceWorkspace, String> {
        let (send, receive) = oneshot::channel();
        self.0
            .send(Heard::Draft(DraftCall::Workspace(send)))
            .map_err(|_| CLOSED.to_owned())?;
        receive.await.map_err(|_| CLOSED.to_owned())?
    }

    async fn write(
        &self,
        base: SourceWorkspace,
        texts: BTreeMap<String, String>,
    ) -> Result<(), String> {
        let (done, receive) = oneshot::channel();
        self.0
            .send(Heard::Draft(DraftCall::Write { base, texts, done }))
            .map_err(|_| CLOSED.to_owned())?;
        receive.await.map_err(|_| CLOSED.to_owned())?
    }
}

/// The stack of the thread a request runs on. The workflow, its HTTP client
/// and logging nest deeply enough to overflow an executor thread's stack,
/// which is 512 KiB on macOS.
const WORKER_STACK: usize = 16 * 1024 * 1024;

/// What a request ended in: `Ok(None)` when it was cancelled.
pub type Finished = anyhow::Result<Option<AgentOutcome>>;

/// Runs `request` on a thread of its own. Its progress and its calls on the
/// story arrive on `heard`; its outcome on the returned channel.
pub fn spawn(
    request: AgentRequest,
    settings: AssistantSettings,
    engine: Engine,
    registry: ArtifactRegistry,
    store: PathBuf,
    cancel: Cancellation,
    heard: mpsc::UnboundedSender<Heard>,
) -> std::io::Result<oneshot::Receiver<Finished>> {
    let (send, receive) = oneshot::channel();
    std::thread::Builder::new()
        .name("assistant".into())
        .stack_size(WORKER_STACK)
        .spawn(move || {
            let _ = send.send(run(
                request, settings, engine, registry, store, cancel, heard,
            ));
        })?;
    Ok(receive)
}

fn run(
    request: AgentRequest,
    settings: AssistantSettings,
    engine: Engine,
    registry: ArtifactRegistry,
    store: PathBuf,
    cancel: Cancellation,
    heard: mpsc::UnboundedSender<Heard>,
) -> Finished {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let store = RunStore::open(&store)?;
        let story = Story {
            draft: Arc::new(Bridge(heard.clone())),
            registry,
        };
        let feedback = move |event| {
            let _ = heard.send(Heard::Feedback(event));
        };
        match engine {
            Engine::Provider(backend) => {
                agent::run_with_backend(
                    request, settings, &backend, story, &store, cancel, feedback,
                )
                .await
            }
            Engine::Model(model) => {
                agent::run_with_model(request, settings, model, story, &store, cancel, feedback)
                    .await
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::atomic::AtomicBool,
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
    fn a_request_reads_the_story_through_the_ui_and_is_answered_over_http() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = serve(listener, "Hello! Which scene should change?");
        let directory = tempfile::tempdir().unwrap();
        let model = OpenAiCompatibleChatModel::new(endpoint, "writing-model")
            .with_timeout(Duration::from_secs(5));
        let (heard, mut hearing) = mpsc::unbounded_channel();
        let request = AgentRequest {
            instructions: "Hello".into(),
            preserve_facts: true,
            scope: agent::Scope::Project,
            editor: None,
            continuing: None,
        };
        let answer = spawn(
            request,
            AssistantSettings::default(),
            Engine::Model(Arc::new(model)),
            ArtifactRegistry::new(),
            directory.path().join("runs.sqlite3"),
            Arc::new(AtomicBool::new(false)),
            heard,
        )
        .unwrap();
        // The UI thread's part: answer the agent's reads of the story.
        let mut reads = 0;
        while let Some(heard) = hearing.blocking_recv() {
            if let Heard::Draft(DraftCall::Workspace(reply)) = heard {
                reads += 1;
                let _ = reply.send(Ok(SourceWorkspace {
                    entry: "story.sf".into(),
                    files: BTreeMap::from([(
                        "story.sf".into(),
                        "scene harbor\n  narrate: Dusk.\n  end\n".into(),
                    )]),
                }));
            }
        }
        let answer = answer.blocking_recv().unwrap();
        server.join().unwrap();
        assert_eq!(reads, 1);
        let outcome = answer.unwrap().unwrap();
        assert_eq!(outcome.reply, "Hello! Which scene should change?");
        assert!(outcome.changed.is_empty());
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

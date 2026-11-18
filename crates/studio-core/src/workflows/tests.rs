use super::*;
use agentkit_ai::{providers::fake::FakeChatModel, ChatStreamEvent};
use agentkit_workflow::{
    DataType, RunError, RunErrorCode, StepDescriptor, StepExecutionContext, StepExecutionRegistry,
    StepFuture, StepInputs, StepOutputs, StepRegistry, Value,
};
use writing::{WritingProposal, WritingRequest, WritingResult, WritingScope};

trait Proposed {
    fn proposal(self) -> WritingProposal;
}

impl Proposed for WritingResult {
    fn proposal(self) -> WritingProposal {
        match self {
            WritingResult::Proposal(proposal) => proposal,
            other => panic!("expected an edit, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn writing_uses_the_bound_provider_and_streaming_http_contract() {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let request = request();
    let replacement = request.original().replace("quiet", "empty");
    let reply = replacement.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(&stream);
        let mut length = 0;
        let mut headers = String::new();
        loop {
            let mut line = String::new();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            if let Some((key, value)) = line.split_once(':') {
                if key.eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse().unwrap();
                }
            }
            headers.push_str(&line);
        }
        assert!(headers.starts_with("POST /v1/chat/completions"));
        assert!(headers
            .to_lowercase()
            .contains("authorization: bearer workflow-test-secret"));
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["model"], "writing-model");
        assert_eq!(body["stream"], true);
        assert_eq!(body["messages"][0]["role"], "system");
        assert!(body["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("replace_only_this_source"));
        let delta = serde_json::json!({"id":"reply", "choices":[{"index":0,"delta":{"role":"assistant","content":reply},"finish_reason":null}]});
        let done = serde_json::json!({"id":"reply", "choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
        let body = format!("data: {delta}\n\ndata: {done}\n\ndata: [DONE]\n\n");
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    let directory = tempfile::tempdir().unwrap();
    let store = RunStore::open(&directory.path().join("runs.sqlite3")).unwrap();
    let model =
        agentkit_ai::providers::openai::OpenAiCompatibleChatModel::new(&endpoint, "writing-model")
            .with_timeout(Duration::from_secs(5))
            .with_api_key("workflow-test-secret");
    let proposal = writing::run_with_model(
        request,
        Arc::new(model),
        &crate::registry::ArtifactRegistry::new(),
        &store,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .await
    .unwrap()
    .unwrap()
    .proposal();
    assert_eq!(proposal.replacement, replacement);
    let state = store.load(&proposal.run_id).unwrap().unwrap();
    assert!(!state.to_json().unwrap().contains("workflow-test-secret"));
    assert!(!store
        .context(&proposal.run_id)
        .unwrap()
        .unwrap()
        .to_string()
        .contains("workflow-test-secret"));
    server.join().unwrap();
}

const SOURCE: &str =
    "scene harbor\n  beat arrival\n    narrate: The café is quiet.\n  beat departure\n    end\n";

fn request() -> WritingRequest {
    WritingRequest::capture(
        Path::new("project.db"),
        Path::new("project.sf"),
        SOURCE,
        WritingScope::Beat(40),
        &crate::registry::ArtifactRegistry::new(),
    )
    .unwrap()
}

fn writing_reply(text: &str) -> Vec<ChatStreamEvent> {
    vec![
        ChatStreamEvent::TextDelta {
            content: text.into(),
        },
        ChatStreamEvent::Done,
    ]
}

#[tokio::test]
async fn invalid_writing_is_repaired_before_publication_and_keeps_original_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let store = RunStore::open(&directory.path().join("runs.sqlite3")).unwrap();
    let request = request();
    let replacement = request.original().replace("quiet", "restless");
    let model = Arc::new(FakeChatModel::new_streaming([
        writing_reply("  beat\n"),
        writing_reply(&replacement),
    ]));
    let registry = crate::registry::ArtifactRegistry::new();
    let proposal = writing::run_with_model(
        request.clone(),
        model,
        &registry,
        &store,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .await
    .unwrap()
    .unwrap()
    .proposal();
    assert_eq!(proposal.request, request);
    assert_eq!(proposal.replacement, replacement);
    let context = store.context(&proposal.run_id).unwrap().unwrap();
    let parent = context["repair_of"].as_str().unwrap();
    assert_eq!(context["attempt"], 1);
    assert_eq!(context["request"], serde_json::to_value(&request).unwrap());
    assert!(store.load(parent).unwrap().is_some());
    let definition: String = store
        .0
        .lock()
        .unwrap()
        .query_row(
            "SELECT definition FROM workflow_runs WHERE id=?1",
            [&proposal.run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(definition.contains("compiler_errors_in_complete_document"));
    assert!(definition.contains("invalid_replacement"));
    let graph: serde_json::Value = serde_json::from_str(&definition).unwrap();
    let repair: serde_json::Value =
        serde_json::from_str(graph["nodes"][2]["config"]["text"].as_str().unwrap()).unwrap();
    assert!(repair["original_request"].is_object());
    assert!(repair["compiler_errors_in_complete_document"]
        .as_array()
        .unwrap()
        .iter()
        .any(|error| error.as_str().unwrap().contains("(line ")));
    assert!(proposal
        .apply(
            Path::new("project.db"),
            Path::new("project.sf"),
            &format!("{SOURCE}\n")
        )
        .is_err());
}

#[tokio::test]
async fn invalid_writing_exhausts_a_bounded_repair_budget_without_a_proposal() {
    let directory = tempfile::tempdir().unwrap();
    let store = RunStore::open(&directory.path().join("runs.sqlite3")).unwrap();
    let model = Arc::new(FakeChatModel::new_streaming([
        writing_reply(""),
        writing_reply("  beat\n"),
        writing_reply("  beat\n"),
    ]));
    let error = writing::run_with_model(
        request(),
        model,
        &crate::registry::ArtifactRegistry::new(),
        &store,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("after 2 attempts"));
    let count: i64 = store
        .0
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM workflow_runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn cancellation_during_repair_does_not_publish_or_submit_another_model_call() {
    let directory = tempfile::tempdir().unwrap();
    let store = RunStore::open(&directory.path().join("runs.sqlite3")).unwrap();
    let model = Arc::new(FakeChatModel::new_streaming([writing_reply("  beat\n")]));
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let result = writing::run_with_model(
        request(),
        model,
        &crate::registry::ArtifactRegistry::new(),
        &store,
        cancel,
        move |event| {
            if matches!(event, Feedback::Status(ref text) if text.starts_with("Fixing the edit ("))
            {
                signal.store(true, Ordering::Relaxed);
            }
        },
    )
    .await
    .unwrap();
    assert!(result.is_none());
}

/// Runs `request` against `replies`, returning the result and every line of
/// detail the run reported.
async fn run_writing_heard(
    request: WritingRequest,
    replies: Vec<Vec<ChatStreamEvent>>,
) -> (Result<Option<WritingResult>>, Vec<String>) {
    let directory = tempfile::tempdir().unwrap();
    let store = RunStore::open(&directory.path().join("runs.sqlite3")).unwrap();
    let details = Arc::new(std::sync::Mutex::new(Vec::new()));
    let heard = details.clone();
    let result = writing::run_with_model(
        request,
        Arc::new(FakeChatModel::new_streaming(replies)),
        &crate::registry::ArtifactRegistry::new(),
        &store,
        Arc::new(AtomicBool::new(false)),
        move |event| {
            if let Feedback::Detail(text) = event {
                heard.lock().unwrap().push(text);
            }
        },
    )
    .await;
    let details = details.lock().unwrap().clone();
    (result, details)
}

#[tokio::test]
async fn a_draft_with_errors_is_still_edited_and_an_edit_may_leave_them() {
    let mut request = request();
    request.source.push_str("scene\n");
    let edit = request.original().replace("quiet", "empty");
    let (result, details) = run_writing_heard(
        request,
        vec![writing_reply(&format!("```storyfella\n{edit}```"))],
    )
    .await;
    assert_eq!(result.unwrap().unwrap().proposal().replacement, edit);
    assert_eq!(details.last().unwrap(), "No new problems");
}

#[tokio::test]
async fn an_edit_that_fixes_a_broken_draft_says_so() {
    let source = format!("{SOURCE}scene\n");
    let request = WritingRequest::capture(
        Path::new("project.db"),
        Path::new("project.sf"),
        &source,
        WritingScope::File,
        &crate::registry::ArtifactRegistry::new(),
    )
    .unwrap();
    let (result, details) = run_writing_heard(
        request,
        vec![writing_reply(&format!("```storyfella\n{SOURCE}```"))],
    )
    .await;
    assert_eq!(result.unwrap().unwrap().proposal().replacement, SOURCE);
    assert_eq!(details.last().unwrap(), "The story compiles now");
}

#[tokio::test]
async fn an_edit_of_a_broken_draft_that_adds_an_error_is_sent_back() {
    let mut request = request();
    request.source.push_str("scene\n");
    let broken = request
        .original()
        .replace("narrate: The café is quiet.", "goto nowhere");
    let fixed = request.original().replace("quiet", "empty");
    let (result, details) = run_writing_heard(
        request,
        vec![
            writing_reply(&format!("```storyfella\n{broken}```")),
            writing_reply(&format!("```storyfella\n{fixed}```")),
        ],
    )
    .await;
    assert_eq!(result.unwrap().unwrap().proposal().replacement, fixed);
    assert!(
        details.iter().any(|detail| detail.starts_with("1 problem")),
        "{details:?}"
    );
}

#[tokio::test]
async fn cancellation_before_publication_discards_a_valid_result() {
    let directory = tempfile::tempdir().unwrap();
    let store = RunStore::open(&directory.path().join("runs.sqlite3")).unwrap();
    let request = request();
    let model = Arc::new(FakeChatModel::new_streaming([writing_reply(
        request.original(),
    )]));
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let result = writing::run_with_model(
        request,
        model,
        &crate::registry::ArtifactRegistry::new(),
        &store,
        cancel,
        move |event| {
            if matches!(event, Feedback::Status(ref text) if text == "Checking the edit") {
                signal.store(true, Ordering::Relaxed);
            }
        },
    )
    .await
    .unwrap();
    assert!(result.is_none());
}

#[test]
fn scopes_are_exact_and_proposals_refuse_other_snapshots_or_projects() {
    let request = request();
    assert!(request.original().starts_with("  beat arrival"));
    assert!(!request.original().contains("departure"));
    assert!(request.context.contains("departure"));
    let proposal = WritingProposal {
        run_id: "test".into(),
        replacement: request.original().replace("quiet", "empty"),
        note: None,
        request,
    };
    let next = proposal
        .apply(Path::new("project.db"), Path::new("project.sf"), SOURCE)
        .unwrap();
    assert!(next.contains("empty"));
    assert!(next.contains("  beat departure\n    end"));
    assert!(proposal
        .apply(Path::new("other.db"), Path::new("project.sf"), SOURCE)
        .is_err());
    assert!(proposal
        .apply(Path::new("project.db"), Path::new("other.sf"), SOURCE)
        .is_err());
    assert!(proposal
        .apply(
            Path::new("project.db"),
            Path::new("project.sf"),
            &format!("{SOURCE}\n")
        )
        .is_err());
    let split = SOURCE.find('é').unwrap() + 1;
    assert!(WritingRequest::capture(
        Path::new("project.db"),
        Path::new("project.sf"),
        SOURCE,
        WritingScope::Selection(storyfella_language::Span::new(split, split + 1)),
        &crate::registry::ArtifactRegistry::new()
    )
    .is_err());
}

#[tokio::test]
async fn entire_file_rewrite_and_expand_cover_all_scenes_and_exact_boundaries() {
    let directory = tempfile::tempdir().unwrap();
    let store = RunStore::open(&directory.path().join("runs.sqlite3")).unwrap();
    let registry = crate::registry::ArtifactRegistry::new();
    let source = "# Café 🌊\r\n\r\nscene harbor\r\n  narrate: The sea is quiet.\r\n  end\r\n\r\nscene forest\r\n  narrate: Mara waits.\r\n  end\r\n\r\n";
    for action in [
        writing::WritingAction::Rewrite,
        writing::WritingAction::Expand,
    ] {
        let mut request = WritingRequest::capture(
            Path::new("project.db"),
            Path::new("project.sf"),
            source,
            WritingScope::File,
            &registry,
        )
        .unwrap();
        request.action = action;
        assert_eq!(
            request.range,
            storyfella_language::Span::new(0, source.len())
        );
        assert_eq!(request.original(), source);
        assert_eq!(request.context, source);
        let prompt: serde_json::Value = serde_json::from_str(&request.prompt()).unwrap();
        assert_eq!(prompt["scope"], "entire_file");
        assert_eq!(prompt["replace_only_this_source"], source);
        let mut replacement = source
            .replace("quiet", "restless")
            .replace("waits", "speaks");
        if action == writing::WritingAction::Expand {
            replacement =
                replacement.replace("  end", "  narrate: The day is only beginning.\r\n  end");
        }
        let model = Arc::new(FakeChatModel::new_streaming([writing_reply(&replacement)]));
        let proposal = writing::run_with_model(
            request,
            model,
            &registry,
            &store,
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await
        .unwrap()
        .unwrap()
        .proposal();
        let next = proposal
            .apply(Path::new("project.db"), Path::new("project.sf"), source)
            .unwrap();
        assert_eq!(next, replacement);
        assert!(next.starts_with("# Café 🌊\r\n\r\n"));
        assert!(next.ends_with("\r\n\r\n"));
    }
}

#[tokio::test]
async fn trimmed_model_output_keeps_block_boundaries_without_a_repair_call() {
    let directory = tempfile::tempdir().unwrap();
    let store = RunStore::open(&directory.path().join("runs.sqlite3")).unwrap();
    let request = request();
    let expected = request.original().replace("quiet", "restless");
    // A single queued response proves formatting repair needs no second call.
    let model = Arc::new(FakeChatModel::new_streaming([writing_reply(
        expected.trim(),
    )]));
    let registry = crate::registry::ArtifactRegistry::new();
    let proposal = writing::run_with_model(
        request,
        model,
        &registry,
        &store,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .await
    .unwrap()
    .unwrap()
    .proposal();
    assert_eq!(proposal.replacement, expected);
}

#[test]
fn model_boundary_normalization_preserves_crlf_blank_lines_and_partial_selections() {
    let registry = crate::registry::ArtifactRegistry::new();
    let source = "scene harbor\r\n  beat arrival\r\n    narrate: Café is quiet.\r\n\r\n  beat departure\r\n    end\r\n";
    let request = WritingRequest::capture(
        Path::new("project.db"),
        Path::new("project.sf"),
        source,
        WritingScope::Beat(source.find("Café").unwrap()),
        &registry,
    )
    .unwrap();
    for response in [
        "beat arrival\n    narrate: Café is restless.",
        "beat arrival\n  narrate: Café is restless.",
    ] {
        assert_eq!(
            request.normalize_replacement(response),
            "  beat arrival\r\n    narrate: Café is restless.\r\n\r\n"
        );
    }
    let start = source.find("quiet").unwrap();
    let partial = WritingRequest::capture(
        Path::new("project.db"),
        Path::new("project.sf"),
        source,
        WritingScope::Selection(storyfella_language::Span::new(start, start + 5)),
        &registry,
    )
    .unwrap();
    assert_eq!(partial.normalize_replacement("still"), "still");
}

#[tokio::test]
async fn writing_streams_and_commits_a_reopenable_run_without_model_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("runs.sqlite3");
    let request = request();
    let replacement = request.original().replace("quiet", "empty");
    let model = Arc::new(FakeChatModel::new_streaming([vec![
        ChatStreamEvent::TextDelta {
            content: replacement.clone(),
        },
        ChatStreamEvent::Done,
    ]]));
    let store = RunStore::open(&path).unwrap();
    let deltas = Arc::new(Mutex::new(String::new()));
    let receive = deltas.clone();
    let proposal = writing::run_with_model(
        request.clone(),
        model,
        &crate::registry::ArtifactRegistry::new(),
        &store,
        Arc::new(AtomicBool::new(false)),
        move |feedback| {
            if let Feedback::Text(text) = feedback {
                receive.lock().unwrap().push_str(&text);
            }
        },
    )
    .await
    .unwrap()
    .unwrap()
    .proposal();
    assert_eq!(proposal.replacement, replacement);
    assert_eq!(*deltas.lock().unwrap(), replacement);
    drop(store);
    let store = RunStore::open(&path).unwrap();
    let state = store.load(&proposal.run_id).unwrap().unwrap();
    assert_eq!(state.status, WorkflowRunStatus::Completed);
    assert_eq!(
        store.context(&proposal.run_id).unwrap().unwrap(),
        serde_json::to_value(request).unwrap()
    );
    let events = store.events(&proposal.run_id).unwrap();
    assert_eq!(events.len() as u64, state.event_sequence);
    assert!(events.iter().any(|event| matches!(
        event.event,
        agentkit_workflow::WorkflowEventKind::RunCompleted
    )));
    let mut stale = state.clone();
    stale.revision += 1;
    assert!(store.commit(0, &stale, &[]).is_err());
    assert_eq!(store.load(&proposal.run_id).unwrap().unwrap(), state);
}

#[tokio::test]
async fn a_commit_waits_for_another_connection_writing_the_same_store() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("runs.sqlite3");
    let request = request();
    let model = Arc::new(FakeChatModel::new_streaming([vec![
        ChatStreamEvent::TextDelta {
            content: request.original().replace("quiet", "empty"),
        },
        ChatStreamEvent::Done,
    ]]));
    let store = RunStore::open(&path).unwrap();
    let proposal = writing::run_with_model(
        request,
        model,
        &crate::registry::ArtifactRegistry::new(),
        &store,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .await
    .unwrap()
    .unwrap()
    .proposal();
    let state = store.load(&proposal.run_id).unwrap().unwrap();

    // Another connection, such as the one saving the conversation, is
    // writing when the run commits, and finishes shortly after.
    let other = rusqlite::Connection::open(&path).unwrap();
    let (locked, wait) = std::sync::mpsc::channel();
    let finishing = std::thread::spawn(move || {
        other
            .execute_batch(
                "BEGIN IMMEDIATE;
                 INSERT INTO assistant_conversations (project, conversation) VALUES ('p', '{}');",
            )
            .unwrap();
        locked.send(()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        other.execute_batch("COMMIT").unwrap();
    });
    wait.recv().unwrap();
    let mut next = state.clone();
    next.revision += 1;
    store.commit(state.revision, &next, &[]).unwrap();
    finishing.join().unwrap();
    assert_eq!(store.load(&proposal.run_id).unwrap().unwrap(), next);
}

#[tokio::test]
async fn cancellation_interrupts_a_stalled_step_and_retains_request_context() {
    let directory = tempfile::tempdir().unwrap();
    let store = RunStore::open(&directory.path().join("runs.sqlite3")).unwrap();
    let mut registry = StepRegistry::builtin();
    registry
        .register(StepDescriptor::new("delayed", "Delayed reply").output("reply", DataType::Text));
    let mut graph = Workflow::builder("test-cancel");
    graph.add("delayed");
    let graph = graph.build(&registry).unwrap();
    let state = store
        .create(&graph, &serde_json::json!({"request":"keep this"}))
        .unwrap();
    let mut executors = StepExecutionRegistry::default();
    executors.register_function(
        "delayed",
        |_: StepInputs, _: StepExecutionContext| -> StepFuture<'static> {
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok(StepOutputs::from([(
                    "reply".into(),
                    Value::Text("late".into()),
                )]))
            })
        },
    );
    let runner = Runner::new(graph, None, vec![])
        .with_registry(registry)
        .with_step_executors(executors);
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let trigger = async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        signal.store(true, Ordering::Relaxed);
    };
    let run = execute(runner, state, &store, cancel, |_| {});
    let (result, ()) =
        tokio::time::timeout(Duration::from_secs(2), async { tokio::join!(run, trigger) })
            .await
            .unwrap();
    let state = result.unwrap();
    assert_eq!(state.status, WorkflowRunStatus::Cancelled);
    assert!(state.outputs.is_empty());
    assert_eq!(
        store.context(&state.identity.run_id).unwrap().unwrap()["request"],
        "keep this"
    );
}

#[tokio::test]
async fn provider_failure_is_durable_and_never_publishes_a_proposal() {
    let directory = tempfile::tempdir().unwrap();
    let store = RunStore::open(&directory.path().join("runs.sqlite3")).unwrap();
    let mut registry = StepRegistry::builtin();
    registry.register(StepDescriptor::new("fail", "Failure"));
    let mut graph = Workflow::builder("test-fail");
    graph.add("fail");
    let graph = graph.build(&registry).unwrap();
    let state = store.create(&graph, &serde_json::Value::Null).unwrap();
    let id = state.identity.run_id.clone();
    let mut executors = StepExecutionRegistry::default();
    executors.register_function(
        "fail",
        |_: StepInputs, _: StepExecutionContext| -> StepFuture<'static> {
            Box::pin(async {
                Err(RunError::new(
                    RunErrorCode::ModelCall,
                    "provider unavailable",
                ))
            })
        },
    );
    assert!(execute(
        Runner::new(graph, None, vec![])
            .with_registry(registry)
            .with_step_executors(executors),
        state,
        &store,
        Arc::new(AtomicBool::new(false)),
        |_| {}
    )
    .await
    .is_err());
    assert_eq!(
        store.load(&id).unwrap().unwrap().status,
        WorkflowRunStatus::Failed
    );
}

async fn run_writing(
    request: WritingRequest,
    replies: Vec<Vec<ChatStreamEvent>>,
) -> (Result<Option<WritingResult>>, i64) {
    let directory = tempfile::tempdir().unwrap();
    let store = RunStore::open(&directory.path().join("runs.sqlite3")).unwrap();
    let result = writing::run_with_model(
        request,
        Arc::new(FakeChatModel::new_streaming(replies)),
        &crate::registry::ArtifactRegistry::new(),
        &store,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .await;
    let runs: i64 = store
        .0
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM workflow_runs", [], |row| row.get(0))
        .unwrap();
    (result, runs)
}

#[tokio::test]
async fn a_request_that_is_not_an_edit_is_answered_without_repairs() {
    let (result, runs) = run_writing(
        request(),
        vec![writing_reply(
            "<think>A greeting, not an edit.</think>Hello! Which beat should change?",
        )],
    )
    .await;
    match result.unwrap() {
        Some(WritingResult::Reply { text, .. }) => {
            assert_eq!(text, "Hello! Which beat should change?")
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(runs, 1, "a message spends no repair attempts");
}

#[tokio::test]
async fn a_fenced_edit_keeps_its_note_and_a_whole_file_answer_is_cut_to_the_passage() {
    let request = request();
    let passage = request.original().replace("quiet", "empty");
    let whole = format!(
        "{}{}{}",
        &request.source[..request.range.start],
        passage,
        &request.source[request.range.end..]
    );
    let (result, _) = run_writing(
        request.clone(),
        vec![writing_reply(&format!(
            "The café is empty now.\n\n```storyfella\n{whole}```"
        ))],
    )
    .await;
    let proposal = result.unwrap().unwrap().proposal();
    assert_eq!(proposal.replacement, passage);
    assert_eq!(proposal.note.as_deref(), Some("The café is empty now."));
}

#[tokio::test]
async fn an_answer_cut_off_at_the_output_limit_is_refused() {
    let request = request();
    let (result, runs) = run_writing(
        request.clone(),
        vec![vec![
            ChatStreamEvent::TextDelta {
                content: format!("```\n{}", request.original()),
            },
            ChatStreamEvent::OutputLimitReached,
            ChatStreamEvent::Done,
        ]],
    )
    .await;
    let error = result.unwrap_err().to_string();
    assert!(error.contains("output token limit"), "{error}");
    assert_eq!(runs, 1, "asking again would hit the same limit");
}

#[tokio::test]
#[ignore = "requires an OpenAI-compatible local model server"]
async fn live_local_model_answers_a_greeting_in_prose() {
    let model_name = std::env::var("STORYFELLA_LIVE_MODEL").expect("set STORYFELLA_LIVE_MODEL");
    let endpoint =
        std::env::var("STORYFELLA_LIVE_URL").unwrap_or_else(|_| "http://127.0.0.1:1234/v1".into());
    let directory = tempfile::tempdir().unwrap();
    let store = RunStore::open(&directory.path().join("runs.sqlite3")).unwrap();
    let mut request = WritingRequest::capture(
        Path::new("project.db"),
        Path::new("project.sf"),
        SOURCE,
        WritingScope::File,
        &crate::registry::ArtifactRegistry::new(),
    )
    .unwrap();
    request.instructions = "Hello".into();
    let model =
        agentkit_ai::providers::openai::OpenAiCompatibleChatModel::new(endpoint, model_name)
            .with_timeout(std::time::Duration::from_secs(120));
    let result = writing::run_with_model(
        request,
        Arc::new(model),
        &crate::registry::ArtifactRegistry::new(),
        &store,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .await;
    assert!(
        matches!(result.unwrap(), Some(WritingResult::Reply { .. })),
        "a greeting is answered in prose"
    );
}

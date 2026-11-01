use super::*;
use agentkit_ai::{providers::fake::FakeChatModel, ChatStreamEvent};
use agentkit_workflow::{
    DataType, RunError, RunErrorCode, StepDescriptor, StepExecutionContext, StepExecutionRegistry,
    StepFuture, StepInputs, StepOutputs, StepRegistry, Value,
};
use writing::{WritingProposal, WritingRequest, WritingScope};

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
    let proposal = writing::run(
        request,
        crate::llm::LlmConfig {
            base_url: endpoint,
            model: "writing-model".into(),
            api_key: Some("workflow-test-secret".into()),
            timeout: Duration::from_secs(5),
        },
        &crate::registry::ArtifactRegistry::new(),
        &store,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .await
    .unwrap()
    .unwrap();
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
    .unwrap();
    assert_eq!(proposal.request, request);
    assert_eq!(proposal.replacement, replacement);
    assert!(!proposal.review(&registry).unwrap().has_errors);
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
    assert!(error.to_string().contains("after 2 repair attempts"));
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
            if matches!(event, Feedback::Status(ref text) if text == "Refining the suggestion…") {
                signal.store(true, Ordering::Relaxed);
            }
        },
    )
    .await
    .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn invalid_existing_source_is_reported_without_calling_the_model() {
    let directory = tempfile::tempdir().unwrap();
    let store = RunStore::open(&directory.path().join("runs.sqlite3")).unwrap();
    let mut request = request();
    request.source.push_str("scene\n");
    let error = writing::run_with_model(
        request,
        Arc::new(FakeChatModel::new_streaming([])),
        &crate::registry::ArtifactRegistry::new(),
        &store,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .await
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("current draft has source errors"));
    let count: i64 = store
        .0
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM workflow_runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
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
            if matches!(event, Feedback::Status(ref text) if text == "Checking suggestion…") {
                signal.store(true, Ordering::Relaxed);
            }
        },
    )
    .await
    .unwrap();
    assert!(result.is_none());
}

#[test]
fn proposal_review_groups_only_new_notes_and_keeps_source_errors_blocking() {
    let source = "scene harbor\n  location: harbor_dock\n  cast: mara, tam\n  beat arrival\n    shot: wide, dawn\n    mara: \"You're late.\"\n    tam think: She counts the minutes.\n";
    let registry = crate::registry::ArtifactRegistry::new();
    let request = WritingRequest::capture(
        Path::new("project.db"),
        Path::new("project.sf"),
        source,
        WritingScope::Beat(source.find("mara:").unwrap()),
        &registry,
    )
    .unwrap();
    let mut proposal = WritingProposal {
        run_id: "review".into(),
        replacement: format!("{}    mara: \"The tide won't wait.\"\n", request.original()),
        request,
    };
    let review = proposal.review(&registry).unwrap();
    assert!(!review.has_errors);
    assert!(
        review.diagnostics.is_empty(),
        "existing artwork/reference warnings must stay in the Writer Problems panel: {:?}",
        review.diagnostics
    );
    proposal
        .replacement
        .push_str("    ada: \"Hello.\"\n    ada: \"Still here.\"\n");
    let review = proposal.review(&registry).unwrap();
    assert_eq!(
        review
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "SF203" && diagnostic.message.contains("ada"))
            .count(),
        1
    );
    proposal.replacement = "  beat\n".into();
    let review = proposal.review(&registry).unwrap();
    assert!(review.has_errors);
    assert!(review
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == storyfella_language::Severity::Error));
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
    .unwrap();
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

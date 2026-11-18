//! Exercise Studio's writing host through a real inference-service allocation.
//! This binary also serves as the isolated service host and fake llama worker.

#[cfg(unix)]
fn main() {
    agentkit_ai::native::service::run_if_requested();
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--model") {
        worker(&args);
        return;
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run());
}

#[cfg(not(unix))]
fn main() {}

#[cfg(unix)]
fn worker(args: &[String]) {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    let value = |flag: &str| &args[args.iter().position(|arg| arg == flag).unwrap() + 1];
    let model = PathBuf::from(value("--model"));
    let port = value("--port").parse::<u16>().unwrap();
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
    let mut calls = 0;
    for stream in listener.incoming() {
        let mut stream = stream.unwrap();
        let mut reader = BufReader::new(&stream);
        let mut request = String::new();
        reader.read_line(&mut request).unwrap();
        let mut length = 0;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse().unwrap();
                }
            }
        }
        let mut input = vec![0; length];
        reader.read_exact(&mut input).unwrap();
        let (content_type, body) = if request.starts_with("GET /health ") {
            ("application/json", "{\"status\":\"ok\"}".to_owned())
        } else if request.starts_with("GET /props ") {
            (
                "application/json",
                serde_json::json!({"model_path": model, "modalities": {"vision": false}})
                    .to_string(),
            )
        } else {
            assert!(
                request.starts_with("POST /v1/chat/completions "),
                "{request}"
            );
            calls += 1;
            std::fs::write(model.with_extension(format!("request{calls}")), b"waiting").unwrap();
            let release = model.with_extension(format!("release{calls}"));
            let deadline = Instant::now() + Duration::from_secs(20);
            while !release.exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(release.exists(), "test did not release request");
            let reply = if calls == 1 {
                "  beat\n"
            } else {
                "  beat arrival\n    narrate: The café is empty.\n"
            };
            let delta = serde_json::json!({"id":"reply", "choices":[{"index":0,"delta":{"role":"assistant","content":reply},"finish_reason":null}]});
            let done = serde_json::json!({"id":"reply", "choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
            (
                "text/event-stream",
                format!("data: {delta}\n\ndata: {done}\n\ndata: [DONE]\n\n"),
            )
        };
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    }
}

#[cfg(unix)]
async fn run() {
    use agentkit_ai::{
        deployments::{ChatDeployment, ManagedChatServer},
        native::service,
    };
    use std::sync::{atomic::AtomicBool, Arc};
    use std::time::Duration;
    use storyfella_studio_core::{
        config::AppConfig,
        planner::PlannerBackend,
        registry::ArtifactRegistry,
        workflows::{
            writing::{self, WritingRequest, WritingScope},
            RunStore,
        },
    };

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let state = root.join("state");
    let model = root.join("model.gguf");
    std::fs::write(&model, b"fixture").unwrap();
    let mut config = AppConfig::load_from(root.join("studio.toml")).unwrap();
    config.state_dir = state.clone();
    config.text = Some(ChatDeployment {
        base_url: "http://127.0.0.1:1/v1".into(),
        model: "writing-model".into(),
        api_key: None,
        timeout: Duration::from_secs(30),
        server: Some(ManagedChatServer {
            executable: std::env::current_exe().unwrap(),
            model_path: model.clone(),
            mmproj: None,
            host: "127.0.0.1".into(),
            port: 1,
        }),
    });
    let backend = PlannerBackend::new(&config);
    let registry = ArtifactRegistry::new();
    let source = "scene harbor\n  beat arrival\n    narrate: The café is quiet.\n  beat departure\n    end\n";
    let request = WritingRequest::capture(
        &root.join("project.db"),
        &root.join("project.sf"),
        source,
        WritingScope::Beat(40),
        &registry,
    )
    .unwrap();
    let store = RunStore::open(&root.join("runs.sqlite3")).unwrap();
    let service = service::Client::new(&state);
    let run = writing::run_with_backend(
        request,
        &backend,
        &registry,
        &store,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    );
    tokio::pin!(run);
    let check = async {
        for call in 1..=2 {
            let marker = model.with_extension(format!("request{call}"));
            tokio::time::timeout(Duration::from_secs(30), async {
                while !marker.exists() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("worker did not receive writing request");
            let status = service.status().await.unwrap();
            assert_eq!(status.resident.len(), 1);
            assert_eq!(
                status.resident[0].active, 1,
                "allocation must survive each writing attempt"
            );
            std::fs::write(model.with_extension(format!("release{call}")), b"go").unwrap();
        }
    };
    let (result, ()) = tokio::join!(run, check);
    let Some(writing::WritingResult::Proposal(proposal)) = result.unwrap() else {
        panic!("expected an edit");
    };
    assert!(proposal.replacement.contains("empty"));
    service.shutdown().await.unwrap();
    println!("managed writing routing and repair lease: passed");
}

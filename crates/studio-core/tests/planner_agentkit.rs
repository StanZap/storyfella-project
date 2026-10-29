use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};
use storyfella_studio_core::llm::{
    ChatMessage, ChatOptions, LlmClient, LlmConfig, ToolDefinition, ToolFunction,
};

#[tokio::test]
async fn planner_host_preserves_structured_controls_and_round_trip_tool_messages() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for index in 0..3 {
            let end = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < end, "planner never called provider");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
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
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                headers.push_str(&line);
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            assert!(headers
                .to_ascii_lowercase()
                .contains("authorization: bearer test-secret"));
            requests.push(serde_json::from_slice::<Value>(&body).unwrap());
            let message = if index == 1 {
                json!({"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"list_artifacts","arguments":"{}"}}]})
            } else {
                json!({"role":"assistant","content":"{\"answer\":\"Mia\"}"})
            };
            let body=json!({"id":"completion","choices":[{"message":message,"finish_reason":if index==1 {"tool_calls"} else {"stop"}}],"usage":{"prompt_tokens":12,"completion_tokens":3,"total_tokens":15}}).to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
        requests
    });
    let client = LlmClient::new(LlmConfig {
        base_url: url,
        model: "story-model".into(),
        api_key: Some("test-secret".into()),
        timeout: Duration::from_secs(2),
    });
    let source = vec![
        ChatMessage::text("system", "Keep the story facts"),
        ChatMessage::text("user", "Describe Mia"),
    ];
    let format = json!({"type":"json_schema","json_schema":{"name":"answer","schema":{"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"],"additionalProperties":false}}});
    let first = client
        .chat_with_options(
            &source,
            &ChatOptions {
                temperature: Some(0.5),
                max_tokens: Some(321),
                seed: Some(42),
                response_format: Some(format.clone()),
                tools: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(first.usage.unwrap().total_tokens, 15);
    let output = client
        .chat_with_tools(
            &source,
            vec![ToolDefinition {
                r#type: "function",
                function: ToolFunction {
                    name: "list_artifacts".into(),
                    description: "Read the catalog".into(),
                    parameters: json!({"type":"object","properties":{}}),
                },
            }],
            Some(format.clone()),
            |name, arguments| {
                assert_eq!(name, "list_artifacts");
                assert_eq!(arguments, "{}");
                "c:mia character Mia".into()
            },
            3,
        )
        .await
        .unwrap();
    assert_eq!(
        output.choices[0].message.content.as_deref(),
        Some("{\"answer\":\"Mia\"}")
    );
    let requests = server.join().unwrap();
    assert_eq!(requests[0]["seed"], 42);
    assert_eq!(requests[0]["max_tokens"], 321);
    assert_eq!(requests[0]["response_format"], format);
    assert_eq!(
        requests[1]["tools"][0]["function"]["name"],
        "list_artifacts"
    );
    assert_eq!(requests[2]["messages"][2]["tool_calls"][0]["id"], "call-1");
    assert_eq!(requests[2]["messages"][3]["tool_call_id"], "call-1");
    assert_eq!(requests[2]["messages"][3]["content"], "c:mia character Mia");
    assert_eq!(source.len(), 2);
}

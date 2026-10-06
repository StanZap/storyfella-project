use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn executable_emits_only_framed_protocol_and_exits_after_shutdown() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_storyfella-lsp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let messages = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
        json!({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {"textDocument": {"uri": "file:///harbor.sf", "version": 1, "text": include_str!("../../../examples/harbor.sf")}}}),
        json!({"jsonrpc": "2.0", "id": "stop", "method": "shutdown"}),
        json!({"jsonrpc": "2.0", "method": "exit"}),
    ];
    let mut stdin = child.stdin.take().unwrap();
    for message in messages {
        let body = serde_json::to_vec(&message).unwrap();
        write!(stdin, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
        stdin.write_all(&body).unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stderr.is_empty());
    let mut remaining = output.stdout.as_slice();
    let mut replies = Vec::new();
    while !remaining.is_empty() {
        let split = remaining.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let header = std::str::from_utf8(&remaining[..split]).unwrap();
        let length: usize = header
            .strip_prefix("Content-Length: ")
            .unwrap()
            .parse()
            .unwrap();
        let body = &remaining[split + 4..split + 4 + length];
        replies.push(serde_json::from_slice::<Value>(body).unwrap());
        remaining = &remaining[split + 4 + length..];
    }
    assert_eq!(replies.len(), 3);
    assert_eq!(replies[0]["id"], 1);
    assert_eq!(replies[1]["method"], "textDocument/publishDiagnostics");
    assert!(
        replies[1]["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["severity"] == 3),
        "{:?}",
        replies[1]
    );
    assert_eq!(replies[2]["id"], "stop");
    assert!(replies[2]["result"].is_null());
}

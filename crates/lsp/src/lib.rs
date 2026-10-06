//! Synchronous stdio LSP adapter over shared analysis. Full-document sync, UTF-16.
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{self, BufRead, Write},
};
use storyfella_language::{
    analyze, complete, Analysis, CompletionKind, Position, Severity, Source, Span, SymbolKind,
};

const MAX_MESSAGE: usize = 4 * 1024 * 1024;

fn read_frame(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut length = None;
    let mut total = 0;
    loop {
        let mut line = String::new();
        let bytes = std::io::Read::take(&mut *reader, 8193).read_line(&mut line)?;
        if bytes == 0 {
            return if total == 0 {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "incomplete LSP header",
                ))
            };
        }
        total += bytes;
        if total > 8192 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "LSP headers exceed 8 KiB",
            ));
        }
        if line == "\r\n" {
            break;
        }
        let (key, value) = line
            .trim_end()
            .split_once(':')
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid LSP header"))?;
        if key.eq_ignore_ascii_case("Content-Length") {
            if length.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate Content-Length",
                ));
            }
            length = Some(value.trim().parse::<usize>().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid Content-Length")
            })?);
        }
    }
    let length = length.filter(|n| *n <= MAX_MESSAGE).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "missing or excessive Content-Length",
        )
    })?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Some(body))
}

fn write_message(writer: &mut impl Write, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}

#[derive(Deserialize)]
struct Identifier {
    uri: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TextPosition {
    text_document: Identifier,
    position: Position,
}
#[derive(Deserialize)]
struct OpenDocument {
    uri: String,
    version: i64,
    text: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenParams {
    text_document: OpenDocument,
}
#[derive(Deserialize)]
struct VersionedIdentifier {
    uri: String,
    version: i64,
}
#[derive(Deserialize)]
struct Change {
    text: String,
    range: Option<Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangeParams {
    text_document: VersionedIdentifier,
    content_changes: Vec<Change>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DocumentParams {
    text_document: Identifier,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReferencesContext {
    include_declaration: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReferencesParams {
    text_document: Identifier,
    position: Position,
    context: ReferencesContext,
}

struct Document {
    version: i64,
    analysis: Analysis,
}
#[derive(Default)]
struct Server {
    initialized: bool,
    shutdown: bool,
    documents: BTreeMap<String, Document>,
}

struct RpcError {
    code: i32,
    message: &'static str,
}
fn invalid() -> RpcError {
    RpcError {
        code: -32602,
        message: "Invalid params",
    }
}
fn params<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, RpcError> {
    serde_json::from_value(value).map_err(|_| invalid())
}

fn range(source: &Source, span: Span) -> Value {
    json!({"start": source.position(span.start).unwrap(), "end": source.position(span.end).unwrap()})
}
fn publish(uri: &str, doc: &Document) -> Value {
    let source = &doc.analysis.document.source;
    let diagnostics: Vec<_> = doc.analysis.diagnostics.iter().map(|d| json!({
        "range": range(source, d.span), "severity": match d.severity { Severity::Error => 1, Severity::Warning => 2, Severity::Information => 3 },
        "code": d.code, "source": "storyfella", "message": d.message,
    })).collect();
    json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": uri, "version": doc.version, "diagnostics": diagnostics}})
}

impl Server {
    fn document(&self, uri: &str) -> Result<&Document, RpcError> {
        self.documents.get(uri).ok_or_else(invalid)
    }
    fn at(&self, p: &TextPosition) -> Result<(&Analysis, usize), RpcError> {
        let analysis = &self.document(&p.text_document.uri)?.analysis;
        let offset = analysis
            .document
            .source
            .offset(p.position)
            .ok_or_else(invalid)?;
        Ok((analysis, offset))
    }
    fn dispatch(
        &mut self,
        method: &str,
        p: Value,
        notifications: &mut Vec<Value>,
    ) -> Result<Value, RpcError> {
        if method == "initialize" {
            if self.initialized {
                return Err(RpcError {
                    code: -32600,
                    message: "Already initialized",
                });
            }
            if !p.is_object() {
                return Err(invalid());
            }
            self.initialized = true;
            return Ok(json!({"capabilities": {
                "positionEncoding": "utf-16", "textDocumentSync": {"openClose": true, "change": 1},
                "completionProvider": {"triggerCharacters": [":", ","]}, "definitionProvider": true,
                "referencesProvider": true, "documentSymbolProvider": true
            }, "serverInfo": {"name": "storyfella-lsp", "version": env!("CARGO_PKG_VERSION")}}));
        }
        if !self.initialized {
            return Err(RpcError {
                code: -32002,
                message: "Server not initialized",
            });
        }
        if self.shutdown {
            return Err(RpcError {
                code: -32600,
                message: "Server has shut down",
            });
        }
        match method {
            "initialized" | "$/cancelRequest" | "$/setTrace" => Ok(Value::Null),
            "shutdown" => {
                self.shutdown = true;
                Ok(Value::Null)
            }
            "textDocument/didOpen" => {
                let p: OpenParams = params(p)?;
                if p.text_document.text.len() > MAX_MESSAGE
                    || self.documents.contains_key(&p.text_document.uri)
                {
                    return Err(invalid());
                }
                let document = Document {
                    version: p.text_document.version,
                    analysis: analyze(&p.text_document.text, &[]),
                };
                notifications.push(publish(&p.text_document.uri, &document));
                self.documents.insert(p.text_document.uri, document);
                Ok(Value::Null)
            }
            "textDocument/didChange" => {
                let p: ChangeParams = params(p)?;
                let previous = self.document(&p.text_document.uri)?;
                if p.text_document.version <= previous.version {
                    return Ok(Value::Null);
                }
                // We advertise Full sync: reject incremental edits instead of corrupting source.
                if p.content_changes.len() != 1 {
                    return Err(invalid());
                }
                let change = &p.content_changes[0];
                if change.range.is_some() || change.text.len() > MAX_MESSAGE {
                    return Err(invalid());
                }
                let document = Document {
                    version: p.text_document.version,
                    analysis: analyze(&change.text, &[]),
                };
                notifications.push(publish(&p.text_document.uri, &document));
                self.documents.insert(p.text_document.uri, document);
                Ok(Value::Null)
            }
            "textDocument/didClose" => {
                let p: DocumentParams = params(p)?;
                self.documents.remove(&p.text_document.uri);
                notifications.push(json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": p.text_document.uri, "diagnostics": []}}));
                Ok(Value::Null)
            }
            "textDocument/definition" => {
                let p: TextPosition = params(p)?;
                let (analysis, offset) = self.at(&p)?;
                Ok(analysis.definition_at(offset).map_or(Value::Null, |span| json!({"uri": p.text_document.uri, "range": range(&analysis.document.source, span)})))
            }
            "textDocument/references" => {
                let p: ReferencesParams = params(p)?;
                let (analysis, offset) = self.at(&TextPosition {
                    text_document: Identifier {
                        uri: p.text_document.uri.clone(),
                    },
                    position: p.position,
                })?;
                Ok(json!(analysis.references_at(offset, p.context.include_declaration).iter().map(|span| json!({"uri": p.text_document.uri, "range": range(&analysis.document.source, *span)})).collect::<Vec<_>>()))
            }
            "textDocument/completion" => {
                let p: TextPosition = params(p)?;
                let (analysis, offset) = self.at(&p)?;
                Ok(json!(complete(analysis, offset, &[]).iter().map(|c| json!({"label": c.label,
                    "kind": match c.kind { CompletionKind::Keyword => 14, CompletionKind::Scene => 3, CompletionKind::Asset => 6 },
                    "textEdit": {"range": range(&analysis.document.source, c.replace), "newText": c.label}
                })).collect::<Vec<_>>()))
            }
            "textDocument/documentSymbol" => {
                let p: DocumentParams = params(p)?;
                let analysis = &self.document(&p.text_document.uri)?.analysis;
                let source = &analysis.document.source;
                let result: Vec<_> = analysis.symbols.iter().filter(|s| s.kind == SymbolKind::Scene).map(|scene| {
                    let children: Vec<_> = analysis.symbols.iter().filter(|s| s.kind == SymbolKind::Beat && scene.span.start <= s.span.start && s.span.end <= scene.span.end).map(|beat| json!({
                        "name": beat.name.value, "kind": 12, "range": range(source, beat.span), "selectionRange": range(source, beat.name.span)
                    })).collect();
                    json!({"name": scene.name.value, "kind": 3, "range": range(source, scene.span), "selectionRange": range(source, scene.name.span), "children": children})
                }).collect();
                Ok(json!(result))
            }
            _ => Err(RpcError {
                code: -32601,
                message: "Method not found",
            }),
        }
    }
}

/// Serve until exit. Returns 0 only when exit follows shutdown; stdout is protocol-only.
pub fn serve(mut reader: impl BufRead, mut writer: impl Write) -> io::Result<i32> {
    let mut server = Server::default();
    while let Some(body) = read_frame(&mut reader)? {
        let message: Value = match serde_json::from_slice(&body) {
            Ok(message) => message,
            Err(_) => {
                write_message(
                    &mut writer,
                    &json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}}),
                )?;
                continue;
            }
        };
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str);
        if message.get("jsonrpc") != Some(&json!("2.0"))
            || method.is_none()
            || id
                .as_ref()
                .is_some_and(|id| !id.is_null() && !id.is_string() && !id.is_number())
        {
            write_message(
                &mut writer,
                &json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600, "message": "Invalid Request"}}),
            )?;
            continue;
        }
        let method = method.unwrap();
        if method == "exit" && id.is_none() {
            return Ok(if server.shutdown { 0 } else { 1 });
        }
        let mut notifications = Vec::new();
        let result = server.dispatch(
            method,
            message.get("params").cloned().unwrap_or(Value::Null),
            &mut notifications,
        );
        if let Some(id) = id {
            let response = match result {
                Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                Err(error) => {
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": error.code, "message": error.message}})
                }
            };
            write_message(&mut writer, &response)?;
        } else if let Err(error) = result {
            if !method.starts_with("$/") {
                notifications.push(json!({"jsonrpc": "2.0", "method": "window/logMessage", "params": {"type": 2, "message": format!("{method}: {}", error.message)}}));
            }
        }
        for notification in notifications {
            write_message(&mut writer, &notification)?;
        }
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Value>> {
        read_frame(reader)?
            .map(|body| {
                serde_json::from_slice(&body)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
            })
            .transpose()
    }
    fn session(messages: &[Value]) -> (i32, Vec<Value>) {
        let mut input = Vec::new();
        for message in messages {
            write_message(&mut input, message).unwrap();
        }
        let mut output = Vec::new();
        let code = serve(Cursor::new(input), &mut output).unwrap();
        let mut output = Cursor::new(output);
        let mut messages = Vec::new();
        while let Some(message) = read_message(&mut output).unwrap() {
            messages.push(message);
        }
        (code, messages)
    }
    fn request(id: u32, method: &str, p: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": p})
    }
    fn notification(method: &str, p: Value) -> Value {
        json!({"jsonrpc": "2.0", "method": method, "params": p})
    }
    fn finish() -> [Value; 2] {
        [
            request(99, "shutdown", Value::Null),
            notification("exit", Value::Null),
        ]
    }

    #[test]
    fn lifecycle_diagnostics_navigation_completion_and_close() {
        let source = "scene start\n  narrate: 🌊\n  goto finish\nscene finish\n  end\n";
        let at = json!({"textDocument": {"uri": "file:///story.sf"}, "position": {"line": 2, "character": 9}});
        let mut messages = vec![
            request(1, "initialize", json!({})),
            notification("initialized", json!({})),
            notification(
                "textDocument/didOpen",
                json!({"textDocument": {"uri": "file:///story.sf", "languageId": "storyfella", "version": 1, "text": source}}),
            ),
            request(2, "textDocument/definition", at.clone()),
            request(3, "textDocument/completion", at.clone()),
            request(
                4,
                "textDocument/documentSymbol",
                json!({"textDocument": {"uri": "file:///story.sf"}}),
            ),
            request(
                5,
                "textDocument/references",
                json!({"textDocument": {"uri": "file:///story.sf"}, "position": {"line": 3, "character": 8}, "context": {"includeDeclaration": true}}),
            ),
            notification(
                "textDocument/didClose",
                json!({"textDocument": {"uri": "file:///story.sf"}}),
            ),
        ];
        messages.extend(finish());
        let (code, out) = session(&messages);
        assert_eq!(code, 0);
        let reply = |id| out.iter().find(|m| m["id"] == id).unwrap();
        assert_eq!(
            reply(1)["result"]["capabilities"]["textDocumentSync"]["change"],
            1
        );
        assert_eq!(
            reply(2)["result"]["range"]["start"],
            json!({"line": 3, "character": 6})
        );
        assert_eq!(reply(3)["result"][0]["label"], "finish");
        assert_eq!(reply(4)["result"].as_array().unwrap().len(), 2);
        assert_eq!(reply(5)["result"].as_array().unwrap().len(), 2);
        assert_eq!(
            out.iter()
                .filter(|m| m["method"] == "textDocument/publishDiagnostics")
                .count(),
            2
        );
    }

    #[test]
    fn edits_are_versioned_and_bad_incremental_edits_do_not_replace_source() {
        let uri = "file:///story.sf";
        let mut messages = vec![
            request(1, "initialize", json!({})),
            notification(
                "textDocument/didOpen",
                json!({"textDocument": {"uri": uri, "version": 1, "text": "scene start\n  goto absent"}}),
            ),
            notification(
                "textDocument/didChange",
                json!({"textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": "scene finish\n  end"}]}),
            ),
            notification(
                "textDocument/didChange",
                json!({"textDocument": {"uri": uri, "version": 1}, "contentChanges": [{"text": "bad"}]}),
            ),
            notification(
                "textDocument/didChange",
                json!({"textDocument": {"uri": uri, "version": 3}, "contentChanges": [{"text": "bad", "range": {}}]}),
            ),
            request(
                2,
                "textDocument/documentSymbol",
                json!({"textDocument": {"uri": uri}}),
            ),
        ];
        messages.extend(finish());
        let (_, out) = session(&messages);
        let diagnostics: Vec<_> = out
            .iter()
            .filter(|m| m["method"] == "textDocument/publishDiagnostics")
            .collect();
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0]["params"]["diagnostics"][0]["code"], "SF202");
        assert_eq!(diagnostics[1]["params"]["diagnostics"], json!([]));
        assert_eq!(
            out.iter().find(|m| m["id"] == 2).unwrap()["result"][0]["name"],
            "finish"
        );
        assert!(out.iter().any(|m| m["method"] == "window/logMessage"));
    }

    #[test]
    fn protocol_errors_and_abnormal_exit() {
        let (code, out) = session(&[
            request(1, "textDocument/completion", json!({})),
            request(2, "initialize", json!({})),
            request(3, "unknown", json!({})),
            request(4, "textDocument/definition", json!({})),
            notification("exit", Value::Null),
        ]);
        assert_eq!(code, 1);
        assert_eq!(out[0]["error"]["code"], -32002);
        assert_eq!(out[2]["error"]["code"], -32601);
        assert_eq!(out[3]["error"]["code"], -32602);
        assert!(read_message(&mut Cursor::new(b"Content-Length: 999999999\r\n\r\n")).is_err());
        assert!(read_message(&mut Cursor::new(
            b"Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}"
        ))
        .is_err());
        assert!(read_message(&mut Cursor::new(b"Content-Length: 5\r\n\r\n{}")).is_err());
    }

    #[test]
    fn malformed_json_does_not_break_following_frames() {
        let mut input = b"Content-Length: 1\r\n\r\n{".to_vec();
        write_message(&mut input, &request(1, "initialize", json!({}))).unwrap();
        for message in finish() {
            write_message(&mut input, &message).unwrap();
        }
        let mut output = Vec::new();
        assert_eq!(serve(Cursor::new(input), &mut output).unwrap(), 0);
        let mut reader = Cursor::new(output);
        assert_eq!(
            read_message(&mut reader).unwrap().unwrap()["error"]["code"],
            -32700
        );
        assert_eq!(read_message(&mut reader).unwrap().unwrap()["id"], 1);
    }
}

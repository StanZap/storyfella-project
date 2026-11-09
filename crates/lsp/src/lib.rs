//! Synchronous stdio LSP adapter over shared analysis. Incremental sync, UTF-16.
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
};
use storyfella_compiler::{ExpandedWorkspace, SourceWorkspace, WorkspaceError};
use storyfella_language::{
    analyze, complete, format, rename, scene_case_fixes, semantic_tokens, Analysis, CompletionKind,
    Diagnostic, Position, Severity, Source, Span, SymbolKind, SEMANTIC_TOKEN_TYPES,
};
use storyfella_studio_core::authoring::{AssetCatalog, CatalogEntry};

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
    range: Option<ChangeRange>,
}
#[derive(Deserialize)]
struct ChangeRange {
    start: Position,
    end: Position,
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
struct CodeActionContext {
    only: Option<Vec<String>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodeActionParams {
    text_document: Identifier,
    range: ChangeRange,
    context: CodeActionContext,
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
    diagnostics: Vec<storyfella_language::Diagnostic>,
}
impl Document {
    fn new(version: i64, text: &str, catalog: &AssetCatalog) -> Self {
        let analysis = analyze(text, &catalog.assets);
        let diagnostics =
            storyfella_compiler::compile_analysis(&analysis, &catalog.bindings).diagnostics;
        Self {
            version,
            analysis,
            diagnostics,
        }
    }
}
#[derive(Default)]
struct Server {
    initialized: bool,
    shutdown: bool,
    documents: BTreeMap<String, Document>,
    catalog_entries: Vec<CatalogEntry>,
    entry_path: Option<PathBuf>,
    published_workspace: BTreeSet<String>,
}

struct WorkspaceCursor {
    workspace: SourceWorkspace,
    expanded: ExpandedWorkspace,
    analysis: Analysis,
    offset: usize,
}

#[derive(Debug)]
struct RpcError {
    code: i32,
    message: String,
}
fn invalid() -> RpcError {
    RpcError {
        code: -32602,
        message: "Invalid params".into(),
    }
}
fn params<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, RpcError> {
    serde_json::from_value(value).map_err(|_| invalid())
}

fn range(source: &Source, span: Span) -> Value {
    json!({"start": source.position(span.start).unwrap(), "end": source.position(span.end).unwrap()})
}

fn completion_kind(kind: CompletionKind) -> u32 {
    match kind {
        CompletionKind::Keyword => 14,
        CompletionKind::Scene => 3,
        CompletionKind::Flag | CompletionKind::Asset => 6,
    }
}
fn diagnostic_json(d: &Diagnostic, range: Value) -> Value {
    json!({
        "range": range, "severity": match d.severity { Severity::Error => 1, Severity::Warning => 2, Severity::Information => 3 },
        "code": d.code, "source": "storyfella", "message": d.message,
    })
}

fn publish(uri: &str, version: Option<i64>, diagnostics: Vec<Value>) -> Value {
    let mut params = json!({"uri": uri, "diagnostics": diagnostics});
    if let Some(version) = version {
        params["version"] = json!(version);
    }
    json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": params})
}

fn relative_source(root: &Path, uri: &str) -> Option<String> {
    let path = url::Url::parse(uri).ok()?.to_file_path().ok()?;
    let relative = path
        .strip_prefix(root)
        .map(Path::to_path_buf)
        .ok()
        .or_else(|| {
            // Clients can spell the workspace's parent path through a symlink.
            let parent = path.parent()?.canonicalize().ok()?;
            Some(parent.strip_prefix(root).ok()?.join(path.file_name()?))
        })?;
    Some(relative.to_str()?.replace(std::path::MAIN_SEPARATOR, "/"))
}

impl Server {
    fn load_workspace(&self, entry: &Path) -> Result<SourceWorkspace, WorkspaceError> {
        let root = entry
            .parent()
            .expect("the configured entry is an absolute file path");
        let drafts = self
            .documents
            .iter()
            .filter_map(|(uri, document)| {
                let path = relative_source(root, uri)?;
                Some((path, document.analysis.document.source.text().to_owned()))
            })
            .collect();
        storyfella_studio_core::story::load_source_workspace(entry, &drafts)
    }

    fn workspace_uri(&self, path: &str) -> Option<String> {
        let root = self.entry_path.as_ref()?.parent()?;
        let uri = url::Url::from_file_path(root.join(path)).ok()?.to_string();
        if self.documents.contains_key(&uri) {
            return Some(uri);
        }
        Some(
            self.documents
                .keys()
                .find(|uri| relative_source(root, uri).as_deref() == Some(path))
                .cloned()
                .unwrap_or(uri),
        )
    }

    fn publish_diagnostics(&mut self, notifications: &mut Vec<Value>) {
        let mut reports: BTreeMap<String, Vec<Value>> = self
            .documents
            .iter()
            .map(|(uri, doc)| {
                let diagnostics = doc
                    .diagnostics
                    .iter()
                    .map(|d| diagnostic_json(d, range(&doc.analysis.document.source, d.span)))
                    .collect();
                (uri.clone(), diagnostics)
            })
            .collect();
        let mut current_workspace = BTreeSet::new();
        if let Some(entry) = &self.entry_path {
            let catalog = self.catalog();
            let analysis = self.load_workspace(entry).and_then(|workspace| {
                let (compilation, expanded) = storyfella_compiler::compile_workspace_with_catalog(
                    &workspace,
                    &catalog.assets,
                    &catalog.bindings,
                )?;
                Ok((workspace, compilation, expanded))
            });
            match analysis {
                Ok((workspace, compilation, expanded)) => {
                    let mut by_path: BTreeMap<_, _> = workspace
                        .files
                        .iter()
                        .map(|(path, text)| (path.as_str(), (Source::new(text), Vec::new())))
                        .collect();
                    for diagnostic in compilation.diagnostics {
                        let (path, span) =
                            expanded.diagnostic_span(&workspace.entry, diagnostic.span);
                        let (source, diagnostics) = by_path
                            .get_mut(path)
                            .expect("mapped path is in the workspace");
                        diagnostics.push(diagnostic_json(&diagnostic, range(source, span)));
                    }
                    for (path, (_, diagnostics)) in by_path {
                        if let Some(uri) = self.workspace_uri(path) {
                            current_workspace.insert(uri.clone());
                            reports.insert(uri, diagnostics);
                        }
                    }
                }
                Err(error) => {
                    if let Some(uri) = self.workspace_uri(&error.path) {
                        current_workspace.insert(uri.clone());
                        reports.entry(uri).or_default().push(diagnostic_json(
                            &error.diagnostic(),
                            json!({"start": error.start, "end": error.end}),
                        ));
                    }
                }
            }
        }
        for uri in self.published_workspace.difference(&current_workspace) {
            reports.entry(uri.clone()).or_default();
        }
        self.published_workspace = current_workspace;
        notifications.extend(reports.into_iter().map(|(uri, diagnostics)| {
            publish(
                &uri,
                self.documents.get(&uri).map(|doc| doc.version),
                diagnostics,
            )
        }));
    }

    fn workspace_at(
        &self,
        uri: &str,
        position: Position,
    ) -> Result<Option<WorkspaceCursor>, RpcError> {
        let Some(entry) = &self.entry_path else {
            return Ok(None);
        };
        let root = entry.parent().ok_or_else(invalid)?;
        let Some(relative) = relative_source(root, uri) else {
            return Ok(None);
        };
        let workspace = self.load_workspace(entry).map_err(|error| RpcError {
            code: -32602,
            message: error.to_string(),
        })?;
        let expanded = workspace.expand().map_err(|error| RpcError {
            code: -32602,
            message: error.message,
        })?;
        let local = self
            .document(uri)?
            .analysis
            .document
            .source
            .offset(position)
            .ok_or_else(invalid)?;
        let offset = expanded.expanded_offset(&relative, local).or_else(|| {
            local.checked_sub(1).and_then(|previous| {
                expanded
                    .expanded_offset(&relative, previous)
                    .map(|offset| offset + 1)
            })
        });
        let Some(offset) = offset else {
            return Ok(None);
        };
        let analysis = analyze(&expanded.source, &self.catalog().assets);
        Ok(Some(WorkspaceCursor {
            workspace,
            expanded,
            analysis,
            offset,
        }))
    }
    fn workspace_location(
        &self,
        workspace: &storyfella_compiler::SourceWorkspace,
        expanded: &storyfella_compiler::ExpandedWorkspace,
        span: Span,
    ) -> Option<Value> {
        let (path, span) = expanded.original_span(span)?;
        let source = Source::new(workspace.files.get(path)?);
        let uri = self.workspace_uri(path)?;
        Some(json!({"uri": uri, "range": range(&source, span)}))
    }

    fn include_definition(&self, uri: &str, analysis: &Analysis, offset: usize) -> Option<Value> {
        let root = self.entry_path.as_ref()?.parent()?;
        let file = relative_source(root, uri)?;
        let include = analysis
            .document
            .story
            .includes
            .iter()
            .find(|include| include.span.contains(offset))?;
        let path = storyfella_compiler::include_path(&file, &include.value);
        let target = root.join(&path).canonicalize().ok()?;
        if !target.starts_with(root) || !target.is_file() {
            return None;
        }
        let uri = self.workspace_uri(&path)?;
        let start = Position::default();
        Some(json!({"uri": uri, "range": {"start": start, "end": start}}))
    }

    fn code_actions(&self, p: CodeActionParams) -> Result<Value, RpcError> {
        if p.context
            .only
            .is_some_and(|kinds| !kinds.iter().any(|kind| kind == "quickfix"))
        {
            return Ok(json!([]));
        }
        let uri = &p.text_document.uri;
        let doc = self.document(uri)?;
        let source = &doc.analysis.document.source;
        let start = source.offset(p.range.start).ok_or_else(invalid)?;
        let end = source
            .offset(p.range.end)
            .filter(|end| *end >= start)
            .ok_or_else(invalid)?;
        let workspace = self.workspace_at(uri, p.range.start)?;
        let analysis = workspace
            .as_ref()
            .map_or(&doc.analysis, |cursor| &cursor.analysis);
        let actions = scene_case_fixes(analysis)
            .filter_map(|edit| {
                let span = if let Some(cursor) = &workspace {
                    let (path, span) = cursor.expanded.original_span(edit.replace)?;
                    (self.workspace_uri(path).as_ref() == Some(uri)).then_some(span)?
                } else {
                    edit.replace
                };
                let selected = if start == end {
                    span.contains(start)
                } else {
                    span.start < end && start < span.end
                };
                if !selected {
                    return None;
                }
                Some(json!({
                    "title": format!("Use scene name '{}'", edit.new_text),
                    "kind": "quickfix",
                    "edit": {"documentChanges": [{
                        "textDocument": {"uri": uri, "version": doc.version},
                        "edits": [{"range": range(source, span), "newText": edit.new_text}]
                    }]}
                }))
            })
            .collect::<Vec<_>>();
        Ok(json!(actions))
    }
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
                    message: "Already initialized".into(),
                });
            }
            if !p.is_object() {
                return Err(invalid());
            }
            let catalog = p
                .pointer("/initializationOptions/catalogPath")
                .map(load_catalog)
                .transpose()?
                .unwrap_or_default();
            if let Some(path) = p.pointer("/initializationOptions/entryPath") {
                let path = Path::new(path.as_str().ok_or_else(invalid)?);
                if !path.is_absolute() || !path.is_file() {
                    return Err(RpcError {
                        code: -32602,
                        message: "entryPath must be an absolute path to an existing story file"
                            .into(),
                    });
                }
                self.entry_path = Some(path.canonicalize().map_err(|error| RpcError {
                    code: -32602,
                    message: format!("Invalid story entry: {error}"),
                })?);
            }
            self.catalog_entries = catalog;
            self.initialized = true;
            return Ok(json!({"capabilities": {
                "positionEncoding": "utf-16", "textDocumentSync": {"openClose": true, "change": 2, "save": true},
                "completionProvider": {"triggerCharacters": [":", ","]}, "definitionProvider": true,
                "referencesProvider": true, "documentSymbolProvider": true,
                "codeActionProvider": {"codeActionKinds": ["quickfix"]},
                "renameProvider": {"prepareProvider": true}, "documentFormattingProvider": true,
                "semanticTokensProvider": {"legend": {"tokenTypes": SEMANTIC_TOKEN_TYPES, "tokenModifiers": []}, "full": true}
            }, "serverInfo": {"name": "storyfella-lsp", "version": env!("CARGO_PKG_VERSION")}}));
        }
        if !self.initialized {
            return Err(RpcError {
                code: -32002,
                message: "Server not initialized".into(),
            });
        }
        if self.shutdown {
            return Err(RpcError {
                code: -32600,
                message: "Server has shut down".into(),
            });
        }
        match method {
            "workspace/didChangeConfiguration" => {
                let path = p
                    .pointer("/settings/storyfella/catalogPath")
                    .ok_or_else(invalid)?;
                let entries = load_catalog(path)?;
                let catalog =
                    AssetCatalog::from_entries(entries.clone()).map_err(|message| RpcError {
                        code: -32602,
                        message,
                    })?;
                for doc in self.documents.values_mut() {
                    *doc =
                        Document::new(doc.version, doc.analysis.document.source.text(), &catalog);
                }
                self.catalog_entries = entries;
                self.publish_diagnostics(notifications);
                Ok(Value::Null)
            }
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
                let document = Document::new(
                    p.text_document.version,
                    &p.text_document.text,
                    &self.catalog(),
                );
                self.documents.insert(p.text_document.uri, document);
                self.publish_diagnostics(notifications);
                Ok(Value::Null)
            }
            "textDocument/didChange" => {
                let p: ChangeParams = params(p)?;
                let previous = self.document(&p.text_document.uri)?;
                if p.text_document.version <= previous.version {
                    return Ok(Value::Null);
                }
                if p.content_changes.is_empty() {
                    return Err(invalid());
                }
                let mut text = previous.analysis.document.source.text().to_owned();
                for change in &p.content_changes {
                    if let Some(range) = &change.range {
                        let source = Source::new(&text);
                        let start = source.offset(range.start).ok_or_else(invalid)?;
                        let end = source.offset(range.end).ok_or_else(invalid)?;
                        if start > end
                            || source.position(start) != Some(range.start)
                            || source.position(end) != Some(range.end)
                        {
                            return Err(invalid());
                        }
                        text.replace_range(start..end, &change.text);
                    } else {
                        text = change.text.clone();
                    }
                    if text.len() > MAX_MESSAGE {
                        return Err(invalid());
                    }
                }
                let document = Document::new(p.text_document.version, &text, &self.catalog());
                self.documents.insert(p.text_document.uri, document);
                self.publish_diagnostics(notifications);
                Ok(Value::Null)
            }
            "textDocument/didClose" => {
                let p: DocumentParams = params(p)?;
                let uri = p.text_document.uri;
                self.documents.remove(&uri);
                let was_workspace = self.published_workspace.contains(&uri);
                self.publish_diagnostics(notifications);
                if !was_workspace && !self.published_workspace.contains(&uri) {
                    notifications.push(publish(&uri, None, Vec::new()));
                }
                Ok(Value::Null)
            }
            "textDocument/didSave" => {
                let _: DocumentParams = params(p)?;
                self.publish_diagnostics(notifications);
                Ok(Value::Null)
            }
            "textDocument/definition" => {
                let p: TextPosition = params(p)?;
                let (local, offset) = self.at(&p)?;
                if let Some(location) = self.include_definition(&p.text_document.uri, local, offset)
                {
                    return Ok(location);
                }
                if let Some(WorkspaceCursor {
                    workspace,
                    expanded,
                    analysis,
                    offset,
                }) = self.workspace_at(&p.text_document.uri, p.position)?
                {
                    return Ok(analysis
                        .definition_at(offset)
                        .and_then(|span| self.workspace_location(&workspace, &expanded, span))
                        .unwrap_or(Value::Null));
                }
                Ok(local.definition_at(offset).map_or(Value::Null, |span| json!({"uri": p.text_document.uri, "range": range(&local.document.source, span)})))
            }
            "textDocument/codeAction" => self.code_actions(params(p)?),
            "textDocument/references" => {
                let p: ReferencesParams = params(p)?;
                if let Some(WorkspaceCursor {
                    workspace,
                    expanded,
                    analysis,
                    offset,
                }) = self.workspace_at(&p.text_document.uri, p.position)?
                {
                    return Ok(json!(analysis
                        .references_at(offset, p.context.include_declaration)
                        .into_iter()
                        .filter_map(|span| self.workspace_location(&workspace, &expanded, span))
                        .collect::<Vec<_>>()));
                }
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
                if let Some(WorkspaceCursor {
                    workspace,
                    expanded,
                    analysis,
                    offset,
                }) = self.workspace_at(&p.text_document.uri, p.position)?
                {
                    let items = complete(&analysis, offset, &self.catalog().assets).into_iter().filter_map(|c| {
                        let location = self.workspace_location(&workspace, &expanded, c.replace)?;
                        if location["uri"] != p.text_document.uri { return None; }
                        Some(json!({"label": c.label, "kind": completion_kind(c.kind), "textEdit": {"range": location["range"], "newText": c.label}}))
                    }).collect::<Vec<_>>();
                    return Ok(json!(items));
                }
                let (analysis, offset) = self.at(&p)?;
                Ok(json!(complete(analysis, offset, &self.catalog().assets).iter().map(|c| json!({"label": c.label,
                    "kind": completion_kind(c.kind),
                    "textEdit": {"range": range(&analysis.document.source, c.replace), "newText": c.label}
                })).collect::<Vec<_>>()))
            }
            "textDocument/prepareRename" | "textDocument/rename" => {
                let position: TextPosition = params(p.clone())?;
                if let Some(WorkspaceCursor {
                    workspace,
                    expanded,
                    analysis,
                    offset,
                }) = self.workspace_at(&position.text_document.uri, position.position)?
                {
                    let definition = analysis.definition_at(offset).ok_or_else(invalid)?;
                    let current = analysis
                        .document
                        .source
                        .slice(definition)
                        .ok_or_else(invalid)?;
                    let name = if method.ends_with("prepareRename") {
                        current
                    } else {
                        p.get("newName")
                            .and_then(Value::as_str)
                            .ok_or_else(invalid)?
                    };
                    let edits = rename(&analysis, offset, name).map_err(|message| RpcError {
                        code: -32602,
                        message,
                    })?;
                    if method.ends_with("prepareRename") {
                        let target = analysis
                            .references
                            .iter()
                            .find(|r| r.name.span.contains(offset))
                            .map_or(definition, |r| r.name.span);
                        let location = self
                            .workspace_location(&workspace, &expanded, target)
                            .ok_or_else(invalid)?;
                        if location["uri"] != position.text_document.uri {
                            return Err(invalid());
                        }
                        return Ok(json!({"range": location["range"], "placeholder": current}));
                    }
                    let mut by_file: BTreeMap<String, Vec<Value>> = BTreeMap::new();
                    for edit in edits {
                        let location = self
                            .workspace_location(&workspace, &expanded, edit.replace)
                            .ok_or_else(invalid)?;
                        let uri = location["uri"].as_str().ok_or_else(invalid)?.to_owned();
                        by_file
                            .entry(uri)
                            .or_default()
                            .push(json!({"range": location["range"], "newText": edit.new_text}));
                    }
                    let changes = by_file.into_iter().map(|(uri, edits)| {
                        let version = self.documents.get(&uri).map(|document| document.version);
                        json!({"textDocument": {"uri": uri, "version": version}, "edits": edits})
                    }).collect::<Vec<_>>();
                    return Ok(json!({"documentChanges": changes}));
                }
                let (analysis, offset) = self.at(&position)?;
                let definition = analysis.definition_at(offset).ok_or_else(invalid)?;
                let current = analysis
                    .document
                    .source
                    .slice(definition)
                    .ok_or_else(invalid)?;
                let name = if method.ends_with("prepareRename") {
                    current
                } else {
                    p.get("newName")
                        .and_then(Value::as_str)
                        .ok_or_else(invalid)?
                };
                let edits = rename(analysis, offset, name).map_err(|message| RpcError {
                    code: -32602,
                    message,
                })?;
                if method.ends_with("prepareRename") {
                    let target = analysis
                        .references
                        .iter()
                        .find(|r| r.name.span.contains(offset))
                        .map_or(definition, |r| r.name.span);
                    Ok(
                        json!({"range": range(&analysis.document.source, target), "placeholder": current}),
                    )
                } else {
                    let doc = self.document(&position.text_document.uri)?;
                    Ok(
                        json!({"documentChanges": [{"textDocument": {"uri": position.text_document.uri, "version": doc.version}, "edits": edits.iter().map(|e| json!({"range": range(&analysis.document.source, e.replace), "newText": e.new_text})).collect::<Vec<_>>()}]}),
                    )
                }
            }
            "textDocument/formatting" => {
                let p: DocumentParams = params(p)?;
                let analysis = &self.document(&p.text_document.uri)?.analysis;
                let edits = format(analysis).map_err(|message| RpcError {
                    code: -32602,
                    message,
                })?;
                Ok(json!(edits.iter().map(|e| json!({"range": range(&analysis.document.source, e.replace), "newText": e.new_text})).collect::<Vec<_>>()))
            }
            "textDocument/semanticTokens/full" => {
                let p: DocumentParams = params(p)?;
                let analysis = &self.document(&p.text_document.uri)?.analysis;
                let mut data = Vec::<u32>::new();
                let mut previous = Position::default();
                for token in semantic_tokens(analysis) {
                    let start = analysis.document.source.position(token.span.start).unwrap();
                    let end = analysis.document.source.position(token.span.end).unwrap();
                    let delta = start.line - previous.line;
                    data.extend([
                        delta,
                        if delta == 0 {
                            start.character - previous.character
                        } else {
                            start.character
                        },
                        end.character - start.character,
                        token.kind,
                        0,
                    ]);
                    previous = start;
                }
                Ok(json!({"data": data}))
            }
            "textDocument/documentSymbol" => {
                let p: DocumentParams = params(p)?;
                let analysis = &self.document(&p.text_document.uri)?.analysis;
                let source = &analysis.document.source;
                let result: Vec<_> = analysis.symbols.iter().filter(|s| s.kind != SymbolKind::Beat).map(|scene| {
                    if scene.kind == SymbolKind::Flag {
                        return json!({"name": scene.name.value, "kind": 13, "range": range(source, scene.span), "selectionRange": range(source, scene.name.span)});
                    }
                    let children: Vec<_> = analysis.symbols.iter().filter(|s| s.kind == SymbolKind::Beat && scene.span.start <= s.span.start && s.span.end <= scene.span.end).map(|beat| json!({
                        "name": beat.name.value, "kind": 12, "range": range(source, beat.span), "selectionRange": range(source, beat.name.span)
                    })).collect();
                    json!({"name": scene.name.value, "kind": 3, "range": range(source, scene.span), "selectionRange": range(source, scene.name.span), "children": children})
                }).collect();
                Ok(json!(result))
            }
            _ => Err(RpcError {
                code: -32601,
                message: "Method not found".into(),
            }),
        }
    }
}

fn load_catalog(path: &Value) -> Result<Vec<CatalogEntry>, RpcError> {
    let path = path.as_str().ok_or_else(invalid)?;
    if !std::path::Path::new(path).is_absolute() {
        return Err(RpcError {
            code: -32602,
            message: "catalogPath must be absolute".into(),
        });
    }
    let entries: Vec<CatalogEntry> = std::fs::read(path)
        .map_err(|e| RpcError {
            code: -32602,
            message: format!("Could not read catalog {path}: {e}"),
        })
        .and_then(|bytes| {
            serde_json::from_slice(&bytes).map_err(|e| RpcError {
                code: -32602,
                message: format!("Invalid catalog {path}: {e}"),
            })
        })?;
    AssetCatalog::from_entries(entries.clone()).map_err(|message| RpcError {
        code: -32602,
        message,
    })?;
    Ok(entries)
}

impl Server {
    fn catalog(&self) -> AssetCatalog {
        AssetCatalog::from_entries(self.catalog_entries.clone())
            .expect("catalog is validated before installation")
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
    fn catalog_refresh_uses_unsaved_text_and_preserves_last_context_on_failure() {
        let directory =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(format!("lsp-catalog-{}.json", std::process::id()));
        std::fs::write(&path, r#"[{"id":"mara","name":"mara","kind":"Character"}]"#).unwrap();
        let mut server = Server::default();
        let mut notifications = Vec::new();
        server
            .dispatch(
                "initialize",
                json!({"initializationOptions":{"catalogPath":path}}),
                &mut notifications,
            )
            .unwrap();
        server.dispatch("textDocument/didOpen", json!({"textDocument":{"uri":"file:///story.sf","version":8,"text":"scene a\n  mara: Unsaved 🌊\n  end\n  narrate: unreachable\n"}}), &mut notifications).unwrap();
        assert!(server.documents["file:///story.sf"]
            .diagnostics
            .iter()
            .any(|d| d.code == "SF301"));
        server.dispatch("textDocument/didChange", json!({"textDocument":{"uri":"file:///story.sf","version":9},"contentChanges":[{"text":"scene a\n  mara: Unsaved 🌊\n"}]}), &mut notifications).unwrap();
        assert!(server.documents["file:///story.sf"]
            .diagnostics
            .iter()
            .any(|d| d.code == "SF306"));
        std::fs::write(
            &path,
            r#"[{"id":"mara","name":"mara","kind":"Character","revision":"image-v1"}]"#,
        )
        .unwrap();
        server
            .dispatch(
                "workspace/didChangeConfiguration",
                json!({"settings":{"storyfella":{"catalogPath":path}}}),
                &mut notifications,
            )
            .unwrap();
        let doc = &server.documents["file:///story.sf"];
        assert_eq!(doc.version, 9);
        assert!(doc.analysis.document.source.text().contains("Unsaved 🌊"));
        assert!(doc.diagnostics.is_empty());
        std::fs::write(&path, "invalid json").unwrap();
        assert!(server
            .dispatch(
                "workspace/didChangeConfiguration",
                json!({"settings":{"storyfella":{"catalogPath":path}}}),
                &mut notifications
            )
            .is_err());
        assert_eq!(
            server.catalog_entries[0].revision.as_deref(),
            Some("image-v1")
        );
        std::fs::remove_file(path).unwrap();
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
            2
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
    fn workspace_navigation_uses_unsaved_included_drafts() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("lsp-workspace-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let entry = root.join("story.sf");
        let chapter = root.join("chapter.sf");
        std::fs::write(&entry, "include \"chapter.sf\"\nscene end\n  end\n").unwrap();
        std::fs::write(&chapter, "scene old\n  goto end\n").unwrap();
        let entry = entry.canonicalize().unwrap();
        let chapter = chapter.canonicalize().unwrap();
        let entry_uri = url::Url::from_file_path(&entry).unwrap().to_string();
        let chapter_uri = url::Url::from_file_path(&chapter).unwrap().to_string();
        let mut server = Server::default();
        let mut notifications = Vec::new();
        server
            .dispatch(
                "initialize",
                json!({"initializationOptions": {"entryPath": entry}}),
                &mut notifications,
            )
            .unwrap();
        server.dispatch("textDocument/didOpen", json!({"textDocument": {"uri": chapter_uri, "version": 1, "text": "scene changed\n  goto end\n"}}), &mut notifications).unwrap();
        let result = server.dispatch("textDocument/definition", json!({"textDocument": {"uri": chapter_uri}, "position": {"line": 1, "character": 8}}), &mut notifications).unwrap();
        assert_eq!(result["uri"], entry_uri);
        assert_eq!(result["range"]["start"]["line"], 1);
        let result = server.dispatch("textDocument/references", json!({"textDocument": {"uri": chapter_uri}, "position": {"line": 1, "character": 8}, "context": {"includeDeclaration": true}}), &mut notifications).unwrap();
        assert_eq!(result.as_array().unwrap().len(), 2);
        let result = server.dispatch("textDocument/rename", json!({"textDocument": {"uri": chapter_uri}, "position": {"line": 1, "character": 8}, "newName": "finish"}), &mut notifications).unwrap();
        let changes = result["documentChanges"].as_array().unwrap();
        assert_eq!(changes.len(), 2);
        assert!(changes
            .iter()
            .any(|change| change["textDocument"]["uri"] == entry_uri
                && change["textDocument"]["version"].is_null()));
        assert!(changes
            .iter()
            .any(|change| change["textDocument"]["uri"] == chapter_uri
                && change["textDocument"]["version"] == 1));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workspace_diagnostics_and_quick_fixes_follow_drafts_and_closed_files() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("lsp-diagnostics-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let entry = root.join("story.sf");
        let chapter = root.join("chapter.sf");
        let source = "include \"chapter.sf\"\r\nscene finish\r\n  end\r\n";
        let chapter_text = "scene start\r\n  choice \"🌊\"\r\n    option \"🌊\": Finish\r\n    option \"stay\": start\r\n";
        std::fs::write(&entry, source).unwrap();
        std::fs::write(&chapter, chapter_text).unwrap();
        let entry_uri = url::Url::from_file_path(&entry).unwrap().to_string();
        let chapter_uri = url::Url::from_file_path(&chapter).unwrap().to_string();
        let mut server = Server::default();
        let mut out = Vec::new();
        server
            .dispatch(
                "initialize",
                json!({"initializationOptions": {"entryPath": entry}}),
                &mut out,
            )
            .unwrap();
        server
            .dispatch(
                "textDocument/didOpen",
                json!({"textDocument": {"uri": entry_uri, "version": 1, "text": source}}),
                &mut out,
            )
            .unwrap();
        let chapter_report = out
            .iter()
            .find(|report| report["params"]["uri"] == chapter_uri)
            .unwrap();
        let diagnostic = &chapter_report["params"]["diagnostics"][0];
        assert_eq!(diagnostic["code"], "SF202");
        assert_eq!(
            diagnostic["range"]["start"],
            json!({"line": 2, "character": 17})
        );
        assert!(chapter_report["params"].get("version").is_none());
        let target = server.dispatch("textDocument/definition", json!({"textDocument": {"uri": entry_uri}, "position": {"line": 0, "character": 12}}), &mut out).unwrap();
        assert_eq!(target["uri"], chapter_uri);
        out.clear();
        server
            .dispatch(
                "textDocument/didOpen",
                json!({"textDocument": {"uri": chapter_uri, "version": 4, "text": chapter_text}}),
                &mut out,
            )
            .unwrap();
        let actions = server.dispatch("textDocument/codeAction", json!({"textDocument": {"uri": chapter_uri}, "range": diagnostic_range(), "context": {"diagnostics": []}}), &mut out).unwrap();
        let edit = &actions[0]["edit"]["documentChanges"][0];
        assert_eq!(edit["textDocument"]["version"], 4);
        assert_eq!(edit["edits"][0]["newText"], "finish");
        assert_eq!(edit["edits"][0]["range"], diagnostic_range());
        out.clear();
        server.dispatch("textDocument/didChange", json!({"textDocument": {"uri": entry_uri, "version": 2}, "contentChanges": [{"text": source.replace("scene finish", "scene Finish")}]}), &mut out).unwrap();
        let chapter_report = out
            .iter()
            .find(|report| report["params"]["uri"] == chapter_uri)
            .unwrap();
        assert_eq!(chapter_report["params"]["diagnostics"], json!([]));
        assert_eq!(chapter_report["params"]["version"], 4);
        out.clear();
        server.dispatch("textDocument/didChange", json!({"textDocument": {"uri": chapter_uri, "version": 5}, "contentChanges": [{"text": "include \"story.sf\"\n"}]}), &mut out).unwrap();
        let chapter_report = out
            .iter()
            .find(|report| report["params"]["uri"] == chapter_uri)
            .unwrap();
        assert!(chapter_report["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "SF308" && d["message"].as_str().unwrap().contains("cycle")));
        std::fs::remove_dir_all(root).unwrap();

        fn diagnostic_range() -> Value {
            json!({"start": {"line": 2, "character": 17}, "end": {"line": 2, "character": 23}})
        }
    }

    #[test]
    fn incremental_edits_are_atomic_and_use_utf16() {
        let mut server = Server::default();
        let mut notifications = Vec::new();
        server
            .dispatch("initialize", json!({}), &mut notifications)
            .unwrap();
        server.dispatch("textDocument/didOpen", json!({"textDocument": {"uri": "file:///test.sf", "version": 1, "text": "# 😀\nscene old\n"}}), &mut notifications).unwrap();
        server.dispatch("textDocument/didChange", json!({"textDocument": {"uri": "file:///test.sf", "version": 2}, "contentChanges": [
            {"range": {"start": {"line": 1, "character": 6}, "end": {"line": 1, "character": 9}}, "text": "new"},
            {"range": {"start": {"line": 0, "character": 2}, "end": {"line": 0, "character": 4}}, "text": "🌊"}
        ]}), &mut notifications).unwrap();
        assert_eq!(
            server
                .document("file:///test.sf")
                .unwrap()
                .analysis
                .document
                .source
                .text(),
            "# 🌊\nscene new\n"
        );
        assert!(server.dispatch("textDocument/didChange", json!({"textDocument": {"uri": "file:///test.sf", "version": 3}, "contentChanges": [
            {"range": {"start": {"line": 1, "character": 6}, "end": {"line": 1, "character": 9}}, "text": "bad"},
            {"range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 4}}, "text": "x"}
        ]}), &mut notifications).is_err());
        let document = server.document("file:///test.sf").unwrap();
        assert_eq!(document.version, 2);
        assert_eq!(
            document.analysis.document.source.text(),
            "# 🌊\nscene new\n"
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

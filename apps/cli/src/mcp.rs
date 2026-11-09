//! A local stdio session owns one closed shared project's authoring lease.
use std::{collections::BTreeMap, fmt::Write as _, path::PathBuf};

use agentkit_core::ToolResultStatus;
use agentkit_mcp::{
    McpError, McpPermission, McpToolCall, McpToolDescriptor, McpToolRegistry, McpToolResult,
};
use anyhow::{ensure, Context, Result};
use clap::Args;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use storyfella_compiler::{compile_workspace_with_catalog, SourceWorkspace};
use storyfella_language::{apply_edits, Severity, Source, Span, TextEdit};
use storyfella_studio_core::{
    authoring::{reconcile, AssetCatalog},
    catalog,
    persistence::{ProjectDb, StoreError, StoredProject},
    project::{AuthoringLease, ProjectFolder},
    registry::ArtifactRegistry,
    story::{
        included_edit_path, load_bounded_source_workspace, source_path, StoryDocument,
        StoryFileError,
    },
};
use uuid::Uuid;

#[path = "mcp/playtest.rs"]
mod playtest;

const READ_BYTES: usize = 24 * 1024;
const EDIT_BYTES: usize = 64 * 1024;
const MAX_STORY_BYTES: usize = 1024 * 1024;
const MAX_DIAGNOSTICS: usize = 24;
const BRANCHING_EXAMPLE: &str = include_str!("mcp/branching.sf");

#[derive(Args)]
pub struct McpArgs {
    /// Absolute path to the single shared project folder this client may edit.
    #[arg(long)]
    folder: PathBuf,
    /// Initialize the folder with this title if it does not exist.
    #[arg(long, value_name = "TITLE")]
    create_if_missing: Option<String>,
}

pub async fn run(args: McpArgs) -> Result<()> {
    ensure!(
        args.folder.is_absolute(),
        "Choose an absolute project folder path"
    );
    let folder = if args.folder.try_exists()? {
        ensure!(args.folder.is_dir(), "Choose a shared project folder");
        ProjectFolder::locate_for_authoring(&args.folder)?
            .context("Choose a shared project folder")?
    } else {
        let title = args
            .create_if_missing
            .context("Project folder does not exist")?;
        ProjectFolder::create(&args.folder, &title, &ArtifactRegistry::default(), "")?
    };
    let lease = folder.claim_authoring()?;
    lease.verify()?;
    ensure!(
        std::fs::metadata(source_path(&folder.database()))?.len() <= MAX_STORY_BYTES as u64,
        "This MCP session supports up to 1 MiB of included story text"
    );
    folder.load()?;
    agentkit_mcp::run_stdio(StoryTools {
        folder,
        lease,
        session: Uuid::new_v4(),
    })
    .await?;
    Ok(())
}

struct StoryTools {
    folder: ProjectFolder,
    lease: AuthoringLease,
    session: Uuid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    session: Uuid,
    file: Option<String>,
    #[serde(default)]
    offset: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    session: Uuid,
    expected_version: String,
    file: Option<String>,
    start: usize,
    end: usize,
    text: String,
    proposal: Option<String>,
}

#[derive(Serialize)]
struct EditReport {
    saved: bool,
    changed: bool,
    valid: bool,
    ready: bool,
    version: String,
    proposal: Option<String>,
    diagnostics: Vec<Value>,
    diagnostic_count: usize,
}

#[derive(Serialize)]
struct Failure {
    code: &'static str,
    message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_version: Option<String>,
}

type Reply<T = Value> = Result<T, Failure>;

fn fail(code: &'static str, message: &'static str) -> Failure {
    Failure {
        code,
        message,
        current_version: None,
    }
}

fn arguments<T: DeserializeOwned>(value: Value) -> Reply<T> {
    serde_json::from_value(value).map_err(|_| {
        fail(
            "invalid_arguments",
            "Use the tool's declared argument schema.",
        )
    })
}

impl StoryTools {
    fn check_owner(&self) -> Reply<()> {
        self.lease.verify().map_err(|_| {
            fail(
                "project_moved",
                "The project moved or was replaced. Restart this MCP session.",
            )
        })
    }

    fn snapshot(&self) -> Reply<(StoredProject, SourceWorkspace)> {
        self.check_owner()?;
        let source_bytes = std::fs::metadata(source_path(&self.folder.database()))
            .map_err(|_| source_unavailable())?
            .len();
        if source_bytes > MAX_STORY_BYTES as u64 {
            return Err(fail(
                "story_too_large",
                "This MCP session supports up to 1 MiB of included story text.",
            ));
        }
        let stored =
            ProjectDb::read_authoring_snapshot(&self.folder.database()).map_err(store_failure)?;
        if stored.id != self.folder.id() {
            return Err(fail(
                "project_moved",
                "The project identity changed. Restart this MCP session.",
            ));
        }
        let workspace = workspace(&stored.story, None)?;
        Ok((stored, workspace))
    }

    fn check_session(&self, session: Uuid) -> Reply<()> {
        if session == self.session {
            Ok(())
        } else {
            Err(fail(
                "session_changed",
                "Inspect the project again to get the current session.",
            ))
        }
    }

    fn inspect(&self) -> Reply {
        let (stored, workspace) = self.snapshot()?;
        Ok(json!({
            "schema_version": 1,
            "session": self.session,
            "project_id": stored.id,
            "name": stored.name,
            "version": version(&workspace, stored.registry_version),
            "entry": workspace.entry,
            "files": workspace.files.keys().collect::<Vec<_>>(),
            "asset_count": stored.registry.artifacts.len(),
        }))
    }

    fn read(&self, args: Read) -> Reply {
        self.check_session(args.session)?;
        let (stored, workspace) = self.snapshot()?;
        let file = args.file.as_deref().unwrap_or(&workspace.entry);
        let source = workspace.files.get(file).ok_or_else(unknown_file)?;
        if !source.is_char_boundary(args.offset) {
            return Err(fail(
                "invalid_offset",
                "Use a UTF-8 byte boundary within the file.",
            ));
        }
        let mut end = (args.offset + READ_BYTES).min(source.len());
        while !source.is_char_boundary(end) {
            end -= 1;
        }
        Ok(json!({
            "file": file,
            "version": version(&workspace, stored.registry_version),
            "offset": args.offset,
            "bytes": source.len(),
            "next_offset": (end < source.len()).then_some(end),
            "source": &source[args.offset..end],
        }))
    }

    fn edit(&self, args: Edit) -> Reply {
        self.check_session(args.session)?;
        let (mut stored, current) = self.snapshot()?;
        let current_version = version(&current, stored.registry_version);
        if args.expected_version != current_version {
            return Err(stale(current_version));
        }
        if args.text.len() > EDIT_BYTES {
            return Err(fail(
                "edit_too_large",
                "Use replacements of at most 65536 UTF-8 bytes.",
            ));
        }
        let file = args.file.as_deref().unwrap_or(&current.entry);
        let before = current.files.get(file).ok_or_else(unknown_file)?;
        let included_path = if file == current.entry {
            None
        } else {
            let entry = stored.story.path().ok_or_else(source_unavailable)?;
            Some(included_edit_path(entry, file).map_err(|_| {
                fail(
                    "source_not_editable",
                    "Use an included file at its canonical project-relative path.",
                )
            })?)
        };
        let source = apply_edits(
            before,
            &[TextEdit {
                replace: Span::new(args.start, args.end),
                new_text: args.text,
            }],
        )
        .map_err(|_| {
            fail(
                "invalid_range",
                "Use an ordered range on UTF-8 byte boundaries in the selected source file.",
            )
        })?;
        let proposed = workspace(&stored.story, Some((file, &source)))?;
        let expanded = proposed.expand().map_err(|_| source_unavailable())?;
        // Use the same declaration and association rules as Studio authoring.
        let mut registry = stored.registry.clone();
        catalog::reconcile_declarations(&mut registry, &expanded.source, false);
        let catalog = AssetCatalog::from_registry(&registry);
        let (compilation, expanded) =
            compile_workspace_with_catalog(&proposed, &catalog.assets, &catalog.bindings)
                .map_err(|_| source_unavailable())?;
        let valid = compilation.plan.is_some() && !compilation.has_errors();
        let mut diagnostic_sources = BTreeMap::new();
        let diagnostics: Vec<_> = [Severity::Error, Severity::Warning, Severity::Information]
            .into_iter()
            .flat_map(|severity| compilation.diagnostics.iter().filter(move |d| d.severity == severity))
            .take(MAX_DIAGNOSTICS)
            .map(|diagnostic| {
                let (file, span) = expanded.diagnostic_span(&proposed.entry, diagnostic.span);
                let source = diagnostic_sources
                    .entry(file)
                    .or_insert_with(|| Source::new(&proposed.files[file]));
                json!({ "file": file, "start": span.start, "end": span.end, "code": diagnostic.code,
                    "start_position": source.position(span.start),
                    "end_position": source.position(span.end),
                    "severity": diagnostic.severity, "message": bounded_text(&diagnostic.message, 512) })
            })
            .collect();
        let changed = &source != before;
        let mut report = EditReport {
            saved: false,
            changed,
            valid,
            ready: compilation.ready(),
            proposal: (valid && changed).then(|| {
                proposal_version(
                    self.session,
                    &current_version,
                    &proposed,
                    stored.registry_version,
                )
            }),
            version: current_version,
            diagnostics,
            diagnostic_count: compilation.diagnostics.len(),
        };
        if !valid || !changed || args.proposal.is_none() {
            return Ok(json!(report));
        }
        if args.proposal != report.proposal {
            return Err(fail(
                "proposal_changed",
                "Preview this exact edit again before saving it.",
            ));
        }
        // Check the include graph again before the store checks the target,
        // entry baseline and catalog version in its write transaction.
        let latest = workspace(&stored.story, None)?;
        let latest_proposal = workspace(&stored.story, Some((file, &source)))?;
        if latest != current || latest_proposal != proposed {
            return Err(fail(
                "stale_version",
                "A source file changed. Read the story and preview the edit again.",
            ));
        }
        let analysis = storyfella_language::analyze(&expanded.source, &catalog.assets);
        reconcile(&mut registry, &analysis);
        self.check_owner()?;
        let db = ProjectDb::open_existing(self.folder.database()).map_err(store_failure)?;
        let new_version = if let Some(path) = included_path {
            let mut document =
                StoryDocument::open(&path).map_err(|error| store_failure(error.into()))?;
            if document.source() != before {
                return Err(fail(
                    "save_conflict",
                    "Source changed. Read the current file and preview again.",
                ));
            }
            document.set_source(source);
            db.save_included_authoring_checked(
                &registry,
                stored.registry_version,
                &stored.story,
                file,
                &mut document,
            )
        } else {
            stored.story.set_source(source);
            db.save_authoring_checked(&registry, stored.registry_version, &mut stored.story)
        }
        .map_err(store_failure)?;
        report.saved = true;
        report.version = version(&proposed, new_version);
        report.proposal = None;
        Ok(json!(report))
    }
}

fn unknown_file() -> Failure {
    fail(
        "unknown_file",
        "Choose a source listed by project inspection.",
    )
}

fn source_unavailable() -> Failure {
    fail("source_unavailable", "Could not load the entry and its includes. Check that the files exist inside the project and total at most 1 MiB.")
}

fn workspace(story: &StoryDocument, edit: Option<(&str, &str)>) -> Reply<SourceWorkspace> {
    let path = story.path().ok_or_else(source_unavailable)?;
    let entry = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(source_unavailable)?;
    let mut drafts = BTreeMap::from([(entry.to_owned(), story.source().to_owned())]);
    if let Some((file, source)) = edit {
        drafts.insert(file.to_owned(), source.to_owned());
    }
    load_bounded_source_workspace(path, &drafts, MAX_STORY_BYTES).map_err(|_| source_unavailable())
}

fn store_failure(error: StoreError) -> Failure {
    match error {
        StoreError::PendingSource => fail(
            "source_recovery_required",
            "An interrupted source save needs recovery. Restart this MCP session to recover it before continuing.",
        ),
        StoreError::Conflict | StoreError::Story(StoryFileError::Conflict { .. }) => fail(
            "save_conflict",
            "Source or catalog changed. Inspect the current project before retrying.",
        ),
        _ => {
            tracing::warn!(%error, "MCP project storage operation failed");
            fail("storage_error", "The project could not complete the storage operation. Inspect the project before retrying.")
        }
    }
}

fn stale(current_version: String) -> Failure {
    Failure {
        current_version: Some(current_version),
        ..fail(
            "stale_version",
            "The saved story or catalog changed. Read the current source and preview again.",
        )
    }
}

fn version(workspace: &SourceWorkspace, registry_version: i64) -> String {
    let mut hash = Sha256::new();
    hash.update(registry_version.to_le_bytes());
    let mut field = |text: &str| {
        hash.update((text.len() as u64).to_le_bytes());
        hash.update(text.as_bytes());
    };
    field(&workspace.entry);
    for (name, source) in &workspace.files {
        field(name);
        field(source);
    }
    hex_digest(hash)
}

fn proposal_version(
    session: Uuid,
    base: &str,
    workspace: &SourceWorkspace,
    registry_version: i64,
) -> String {
    let mut hash = Sha256::new();
    hash.update(session.as_bytes());
    hash.update(base.as_bytes());
    hash.update(version(workspace, registry_version).as_bytes());
    hex_digest(hash)
}

fn hex_digest(hash: Sha256) -> String {
    let mut output = String::with_capacity(64);
    for byte in hash.finalize() {
        write!(output, "{byte:02x}").expect("writing to a string cannot fail");
    }
    output
}

fn bounded_text(text: &str, chars: usize) -> &str {
    text.char_indices()
        .nth(chars)
        .map_or(text, |(end, _)| &text[..end])
}

impl McpToolRegistry for StoryTools {
    fn list_tools(&self) -> Vec<McpToolDescriptor> {
        let session = json!({"type":"string","format":"uuid"});
        let version = json!({"type":"string","minLength":64,"maxLength":64});
        let offset = json!({"type":"integer","minimum":0});
        vec![
            tool("storyfella_project_inspect", "Inspect the one project authorized for this MCP process and get its session and current version.", McpPermission::ReadOnly, json!({}), &[]),
            tool("storyfella_story_example", "Get a complete, playable Storyfella source example with narration, a choice, a persistent flag, and conditional endings. Use this syntax as a reference before drafting an original branching chapter.", McpPermission::ReadOnly, json!({}), &[]),
            tool("storyfella_story_read", "Read saved Storyfella source. Omit file for the entry. Offsets and lengths are UTF-8 bytes.", McpPermission::ReadOnly, json!({
                "session": session, "file": {"type":"string"}, "offset": offset
            }), &["session"]),
            tool("storyfella_story_edit", "Replace a UTF-8 byte range in one source file. Omit file for the entry, or name an included file from project inspection. Omit proposal to validate without saving. Repeat the exact edit with the returned proposal to save. Check saved and valid in the response. Diagnostic positions are zero-based lines and UTF-16 columns in the named file.", McpPermission::Mutating, json!({
                "session": session, "expected_version": version, "start": offset, "end": offset,
                "file": {"type":"string"}, "text": {"type":"string","maxLength":EDIT_BYTES}, "proposal": version
            }), &["session", "expected_version", "start", "end", "text"]),
            tool("storyfella_story_playtest", "Run a bounded, text-only playtest of saved source with zero-based choice indexes. Omit choices to stop at the first choice.", McpPermission::ReadOnly, json!({
                "session": session, "expected_version": version,
                "choices": {"type":"array", "maxItems":playtest::MAX_STEPS, "items":offset}
            }), &["session", "expected_version"]),
        ]
    }

    fn call_tool(&self, call: McpToolCall) -> Result<McpToolResult, McpError> {
        let result = match call.name.as_str() {
            "storyfella_project_inspect" => {
                arguments::<NoArgs>(call.arguments).and_then(|_| self.inspect())
            }
            "storyfella_story_example" => {
                arguments::<NoArgs>(call.arguments).map(|_| json!({ "source": BRANCHING_EXAMPLE }))
            }
            "storyfella_story_read" => arguments(call.arguments).and_then(|args| self.read(args)),
            "storyfella_story_edit" => arguments(call.arguments).and_then(|args| self.edit(args)),
            "storyfella_story_playtest" => {
                arguments(call.arguments).and_then(|args| self.playtest(args))
            }
            _ => return Err(McpError::ToolNotFound { name: call.name }),
        };
        let (status, content) = match result {
            Ok(value) => (ToolResultStatus::Success, value),
            Err(error) => (ToolResultStatus::Error, json!(error)),
        };
        Ok(McpToolResult {
            call_id: call.call_id,
            tool_name: call.name,
            status,
            content: content.to_string(),
            metadata: Some(json!({ "structured_content": content })),
        })
    }
}

fn tool(
    name: &str,
    description: &str,
    permission: McpPermission,
    properties: Value,
    required: &[&str],
) -> McpToolDescriptor {
    McpToolDescriptor {
        name: name.into(),
        description: description.into(),
        input_schema: json!({ "type":"object", "properties":properties, "required":required, "additionalProperties":false }),
        permission,
        output_schema: None,
        capabilities: vec![],
        metadata: None,
    }
}

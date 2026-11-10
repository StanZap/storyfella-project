//! Route live Writer requests through the desktop state owner.
use std::{collections::BTreeMap, fmt::Write as _, path::PathBuf};

use anyhow::{ensure, Context, Result};
use dioxus::prelude::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use storyfella_compiler::compile_workspace_with_catalog;
use storyfella_language::{apply_edits, Severity, Source, Span, TextEdit};
use storyfella_studio_core::{
    authoring::AssetCatalog,
    catalog,
    desktop_bridge::{
        self, Call, Command, Failure, ProjectIdentity, Reply, Request, EDIT_BYTES, MAX_CONNECTIONS,
        SCHEMA,
    },
    project::{AuthoringLease, ProjectFolder},
    registry::ArtifactRegistry,
    story::{load_bounded_source_workspace, source_path, AdditionalSources, StoryDocument},
};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::state::AppState;

const MAX_STORY_BYTES: usize = 1024 * 1024;
const READ_BYTES: usize = 24 * 1024;

struct Grant {
    folder: ProjectFolder,
    directory: PathBuf,
    edit: bool,
}

impl Grant {
    fn from_environment() -> Result<Option<Self>> {
        let Some(path) = std::env::var_os("STORYFELLA_MCP_FOLDER") else {
            return Ok(None);
        };
        let path = PathBuf::from(path);
        ensure!(
            path.is_absolute() && path.is_dir(),
            "Choose an absolute project folder"
        );
        Ok(Some(Self {
            folder: ProjectFolder::locate_for_authoring(&path)?
                .context("Choose a shared Storyfella project folder")?,
            directory: std::env::var_os("STORYFELLA_MCP_CHANNEL")
                .context("Set STORYFELLA_MCP_CHANNEL to a new private channel directory")?
                .into(),
            edit: std::env::var_os("STORYFELLA_MCP_EDIT").is_some_and(|value| value == "1"),
        }))
    }
}

pub(crate) fn use_bridge(state: Signal<AppState>) {
    use_future(move || async move {
        let result = match Grant::from_environment() {
            Ok(Some(grant)) => run(state, grant).await,
            Ok(None) => Ok(()),
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            tracing::warn!(%error, "Studio MCP channel unavailable");
        }
    });
}

async fn run(state: Signal<AppState>, grant: Grant) -> Result<()> {
    let (sender, mut receiver) = mpsc::channel::<Call<Command>>(MAX_CONNECTIONS);
    let serve = desktop_bridge::serve(&grant.directory, sender);
    let dispatch = async {
        while let Some(call) = receiver.recv().await {
            let Call {
                request,
                session,
                epoch,
                response,
            } = call;
            if !response.is_closed() {
                let (epoch, reply) =
                    dispatch_call(state, &grant, request, session, epoch, &response).await;
                let _ = response.send((epoch, reply));
            }
        }
        Ok(())
    };
    tokio::select! { result = serve => result, result = dispatch => result }
}

#[cfg(test)]
#[path = "live_mcp/tests.rs"]
mod tests;

fn owner(state: &AppState, grant: &Grant, epoch: Option<Uuid>) -> Result<Uuid, Failure> {
    let current = state
        .authoring_lease
        .as_ref()
        .filter(|lease| lease.belongs_to(&grant.folder))
        .map(AuthoringLease::epoch);
    if epoch.is_some() && current != epoch {
        return Err(Failure::new(
            "owner_changed",
            "Reopen the selected project and reconnect this MCP client.",
        ));
    }
    current.ok_or_else(|| Failure::new("owner_unavailable", "Open the selected project in the Studio window that granted access, then inspect again."))
}

#[derive(PartialEq)]
struct Draft {
    entry: StoryDocument,
    includes: BTreeMap<String, String>,
    registry: ArtifactRegistry,
    catalog_version: i64,
    name: String,
}

impl From<&AppState> for Draft {
    fn from(state: &AppState) -> Self {
        Self {
            entry: state.story.clone(),
            includes: state
                .additional_sources
                .as_ref()
                .map_or_else(BTreeMap::new, AdditionalSources::overrides),
            registry: state.registry.clone(),
            catalog_version: state.registry_version,
            name: state.project_name.clone(),
        }
    }
}

async fn dispatch_call(
    mut state: Signal<AppState>,
    grant: &Grant,
    request: Request,
    session: Uuid,
    epoch: Option<Uuid>,
    response: &oneshot::Sender<(Option<Uuid>, Reply)>,
) -> (Option<Uuid>, Reply) {
    let result = async {
        if request.schema != SCHEMA || request.project != ProjectIdentity::from(&grant.folder) {
            return Err(Failure::new("project_mismatch", "Connect to the channel for the selected shared project using a compatible Storyfella CLI."));
        }
        let supplied = match &request.command {
            Command::Inspect {} => None,
            Command::Read { session, .. } | Command::Edit { session, .. } => Some(*session),
        };
        if supplied.is_some() && (supplied != Some(session) || epoch.is_none()) {
            return Err(Failure::new("session_changed", "Inspect the project on this MCP connection to obtain its session."));
        }
        if !grant.edit && matches!(request.command, Command::Edit { apply: true, .. }) {
            return Err(Failure::new("scope_denied", "Studio granted inspection only. Enable draft edits when opening a new channel."));
        }
        let (current, lease, draft) = {
            let state = state.peek();
            let current = owner(&state, grant, epoch)?;
            let lease = state.authoring_lease.as_ref().expect("owner checked the lease").clone();
            (current, lease, Draft::from(&*state))
        };
        let project_id = grant.folder.id();
        let entry = source_path(&grant.folder.database());
        let edit = grant.edit;
        let (draft, prepared) = tokio::task::spawn_blocking(move || {
            let prepared = lease.verify().map_err(|_| moved()).and_then(|()| {
                if entry.canonicalize().ok().as_deref() != draft.entry.path() {
                    return Err(moved());
                }
                prepare(&draft, request.command, session, project_id, edit)
            });
            (draft, prepared)
        }).await.map_err(|_| Failure::new("owner_busy", "Studio could not prepare the request. Inspect the draft before retrying."))?;
        owner(&state.peek(), grant, Some(current))?;
        let (reply, change) = prepared?;
        if let Some(source) = change {
            if response.is_closed() {
                return Err(Failure::new("client_disconnected", "Reconnect and inspect the draft."));
            }
            let app = state.peek();
            if app.operation_in_progress() || app.catalog_conflict.is_some() {
                return Err(Failure::new("owner_busy", "Finish the current Studio operation or resolve the catalog conflict, then inspect again."));
            }
            if draft != Draft::from(&*app) {
                return Err(Failure::new("stale_version", "The draft changed during validation. Read the current draft and retry."));
            }
            app.authoring_lease.as_ref().expect("owner checked the lease").verify().map_err(|_| moved())?;
            drop(app);
            state.write().set_story_source(source);
        }
        Ok((current, reply))
    }.await;
    match result {
        Ok((current, reply)) => (Some(current), Ok(reply)),
        Err(error) => (epoch, Err(error)),
    }
}

fn moved() -> Failure {
    Failure::new(
        "project_moved",
        "The project moved or was replaced. Reopen it in Studio and reconnect.",
    )
}

fn workspace_unavailable() -> Failure {
    Failure::new(
        "source_unavailable",
        "Check the draft's includes and keep the reachable story under 1 MiB.",
    )
}

fn digest(bytes: impl AsRef<[u8]>) -> String {
    let mut hex = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(hex, "{byte:02x}").expect("writing to a string cannot fail");
    }
    hex
}

fn prepare(
    draft: &Draft,
    command: Command,
    session: Uuid,
    project_id: Uuid,
    can_edit: bool,
) -> Result<(serde_json::Value, Option<String>), Failure> {
    let entry = draft.entry.path().ok_or_else(moved)?;
    let name = entry
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(moved)?;
    let mut overrides = draft.includes.clone();
    overrides.insert(name.to_owned(), draft.entry.source().to_owned());
    let workspace = load_bounded_source_workspace(entry, &overrides, MAX_STORY_BYTES);
    let assets = AssetCatalog::from_registry(&draft.registry);
    let asset_identities: Vec<_> = assets
        .assets
        .iter()
        .map(|asset| (&asset.id, &asset.name, asset.kind))
        .collect();
    let version_bytes = serde_json::to_vec(&(
        workspace.as_ref().ok(),
        &overrides,
        draft.entry.saved_source(),
        draft.catalog_version,
        &asset_identities,
        &assets.bindings,
    ))
    .map_err(|_| workspace_unavailable())?;
    let version = digest(version_bytes);
    let source = draft.entry.source();
    let mut reply = json!({
        "version": version,
        "file": name,
        "draft_digest": digest(source),
        "saved_digest": digest(draft.entry.saved_source()),
        "dirty": draft.entry.is_dirty(),
    });
    let mut change = None;
    match command {
        Command::Inspect {} => {
            reply["schema_version"] = json!(SCHEMA);
            reply["session"] = json!(session);
            reply["project_id"] = json!(project_id);
            reply["name"] = json!(draft.name);
            reply["can_edit"] = json!(can_edit);
        }
        Command::Read { offset, .. } => {
            if !source.is_char_boundary(offset) {
                return Err(Failure::new(
                    "invalid_offset",
                    "Use a UTF-8 byte boundary within the entry file.",
                ));
            }
            let mut end = (offset + READ_BYTES).min(source.len());
            while !source.is_char_boundary(end) {
                end -= 1;
            }
            reply["offset"] = json!(offset);
            reply["bytes"] = json!(source.len());
            reply["next_offset"] = json!((end < source.len()).then_some(end));
            reply["source"] = json!(&source[offset..end]);
        }
        Command::Edit {
            expected_version,
            start,
            end,
            text,
            apply,
            ..
        } => {
            if expected_version != version {
                return Err(Failure {
                    current_version: Some(version),
                    ..Failure::new("stale_version", "The draft or catalog changed. Read the current draft and retry the edit against its version.")
                });
            }
            if text.len() > EDIT_BYTES {
                return Err(Failure::new(
                    "edit_too_large",
                    "Use replacements of at most 65536 UTF-8 bytes.",
                ));
            }
            let updated = apply_edits(
                source,
                &[TextEdit {
                    replace: Span::new(start, end),
                    new_text: text,
                }],
            )
            .map_err(|_| {
                Failure::new(
                    "invalid_range",
                    "Use an ordered range on UTF-8 byte boundaries within the entry file.",
                )
            })?;
            overrides.insert(name.to_owned(), updated.clone());
            let proposed = load_bounded_source_workspace(entry, &overrides, MAX_STORY_BYTES)
                .map_err(|_| workspace_unavailable())?;
            let expanded = proposed.expand().map_err(|_| workspace_unavailable())?;
            let mut registry = draft.registry.clone();
            catalog::reconcile_declarations(&mut registry, &expanded.source, true);
            let assets = AssetCatalog::from_registry(&registry);
            let (compilation, expanded) =
                compile_workspace_with_catalog(&proposed, &assets.assets, &assets.bindings)
                    .map_err(|_| workspace_unavailable())?;
            let valid = compilation.plan.is_some() && !compilation.has_errors();
            let diagnostics: Vec<_> = [Severity::Error, Severity::Warning, Severity::Information]
                .into_iter()
                .flat_map(|severity| {
                    compilation
                        .diagnostics
                        .iter()
                        .filter(move |diagnostic| diagnostic.severity == severity)
                })
                .take(24)
                .map(|diagnostic| {
                    let (file, span) = expanded.diagnostic_span(name, diagnostic.span);
                    let source = Source::new(&proposed.files[file]);
                    json!({ "file": file, "code": diagnostic.code, "severity": diagnostic.severity,
                        "position": source.position(span.start),
                        "message": diagnostic.message.chars().take(512).collect::<String>() })
                })
                .collect();
            let changed = updated != source;
            if valid && changed && apply {
                change = Some(updated);
            }
            reply = json!({
                "base_version": version,
                "applied": change.is_some(),
                "changed": changed,
                "valid": valid,
                "ready": compilation.ready(),
                "diagnostics": diagnostics,
                "diagnostic_count": compilation.diagnostics.len(),
            });
        }
    }
    Ok((reply, change))
}

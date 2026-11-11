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
        self, Call, Command, Failure, Reply, Request, WriterCommand, EDIT_BYTES, MAX_CONNECTIONS,
        SCHEMA,
    },
    project::{AuthoringLease, ProjectFolder},
    registry::ArtifactRegistry,
    story::{
        included_edit_path, load_bounded_source_workspace, source_path, AdditionalSources,
        StoryDocument,
    },
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
    let window = dioxus::desktop::use_window();
    use_future(move || {
        let window = window.clone();
        async move {
            let result = match Grant::from_environment() {
                Ok(Some(grant)) => run(state, grant, window).await,
                Ok(None) => Ok(()),
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                tracing::warn!(%error, "Studio MCP channel unavailable");
            }
        }
    });
}

async fn run(
    state: Signal<AppState>,
    grant: Grant,
    window: dioxus::desktop::DesktopContext,
) -> Result<()> {
    let (sender, mut receiver) = mpsc::channel::<Call<Command>>(MAX_CONNECTIONS);
    let serve = desktop_bridge::serve(&grant.directory, sender);
    let dispatch = async {
        let mut previews = preview::Previews::default();
        while let Some(call) = receiver.recv().await {
            let Call {
                request,
                session,
                epoch,
                response,
            } = call;
            if !response.is_closed() {
                let Request {
                    schema,
                    project,
                    command,
                } = request;
                let (epoch, reply) = match command {
                    Command::Preview(command) => (
                        epoch,
                        previews
                            .dispatch(
                                state,
                                &grant,
                                Request {
                                    schema,
                                    project,
                                    command,
                                },
                                session,
                                epoch,
                                &window,
                            )
                            .await,
                    ),
                    Command::Writer(command) => {
                        dispatch_call(
                            state,
                            &grant,
                            Request {
                                schema,
                                project,
                                command,
                            },
                            session,
                            epoch,
                            &response,
                        )
                        .await
                    }
                };
                let _ = response.send((epoch, reply));
            }
        }
        Ok(())
    };
    tokio::select! { result = serve => result, result = dispatch => result }
}

#[path = "live_mcp/preview.rs"]
mod preview;

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
    includes: Option<AdditionalSources>,
    registry: ArtifactRegistry,
    catalog_version: i64,
    name: String,
}

impl From<&AppState> for Draft {
    fn from(state: &AppState) -> Self {
        Self {
            entry: state.story.clone(),
            includes: state.additional_sources.clone(),
            registry: state.registry.clone(),
            catalog_version: state.registry_version,
            name: state.project_name.clone(),
        }
    }
}

impl Draft {
    fn overrides(&self, entry_name: &str) -> BTreeMap<String, String> {
        let mut overrides = self
            .includes
            .as_ref()
            .map_or_else(BTreeMap::new, AdditionalSources::overrides);
        overrides.insert(entry_name.to_owned(), self.entry.source().to_owned());
        overrides
    }
}

async fn dispatch_call(
    mut state: Signal<AppState>,
    grant: &Grant,
    request: Request<WriterCommand>,
    session: Uuid,
    epoch: Option<Uuid>,
    response: &oneshot::Sender<(Option<Uuid>, Reply)>,
) -> (Option<Uuid>, Reply) {
    let result = async {
        request.check_project(&grant.folder)?;
        let supplied = match &request.command {
            WriterCommand::Inspect {} => None,
            WriterCommand::Read { session, .. } | WriterCommand::Edit { session, .. } => Some(*session),
        };
        if supplied.is_some() && (supplied != Some(session) || epoch.is_none()) {
            return Err(Failure::new("session_changed", "Inspect the project on this MCP connection to obtain its session."));
        }
        if !grant.edit && matches!(request.command, WriterCommand::Edit { apply: true, .. }) {
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
        if let Some(change) = change {
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
            if let Change::Include { file, sources, .. } = &change {
                let entry = app.story.path().ok_or_else(moved)?;
                let path = included_edit_path(entry, file).map_err(|_| source_changed())?;
                if sources.get(file).and_then(StoryDocument::path) != Some(path.as_path()) {
                    return Err(source_changed());
                }
            }
            drop(app);
            match change {
                Change::Entry(source) => state.write().set_story_source(source),
                Change::Include { file, sources, source } => state.write()
                    .apply_checked_include(&file, sources, source)
                    .map_err(|_| source_changed())?,
            }
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

fn source_changed() -> Failure {
    Failure::new(
        "source_changed",
        "The included source changed or moved. Reopen it in Studio, then reread the draft.",
    )
}

enum Change {
    Entry(String),
    Include {
        file: String,
        sources: AdditionalSources,
        source: String,
    },
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
    command: WriterCommand,
    session: Uuid,
    project_id: Uuid,
    can_edit: bool,
) -> Result<(serde_json::Value, Option<Change>), Failure> {
    let entry = draft.entry.path().ok_or_else(moved)?;
    let name = entry
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(moved)?;
    let mut overrides = draft.overrides(name);
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
    let file = match &command {
        WriterCommand::Read { file, .. } | WriterCommand::Edit { file, .. } => file.as_str(),
        WriterCommand::Inspect {} => name,
    }
    .to_owned();
    let file = file.as_str();
    let included = file != name;
    let (source, saved_source) = if included {
        let source = workspace
            .as_ref()
            .map_err(|_| workspace_unavailable())?
            .files
            .get(file)
            .ok_or_else(|| {
                Failure::new(
                    "unknown_file",
                    "Use a project-relative file from draft inspection.",
                )
            })?;
        let baseline = draft
            .includes
            .as_ref()
            .and_then(|sources| sources.get(file))
            .filter(|document| document.is_dirty())
            .map_or(source.as_str(), StoryDocument::saved_source);
        (source.as_str(), baseline)
    } else {
        (draft.entry.source(), draft.entry.saved_source())
    };
    let mut reply = json!({
        "version": version,
        "file": file,
        "draft_digest": digest(source),
        "saved_digest": digest(saved_source),
        "dirty": source != saved_source,
    });
    let mut change = None;
    match command {
        WriterCommand::Inspect {} => {
            reply["schema_version"] = json!(SCHEMA);
            reply["session"] = json!(session);
            reply["project_id"] = json!(project_id);
            reply["name"] = json!(draft.name);
            reply["can_edit"] = json!(can_edit);
            reply["files"] = json!(workspace
                .as_ref()
                .ok()
                .map(|workspace| workspace.files.keys().collect::<Vec<_>>()));
        }
        WriterCommand::Read { offset, .. } => {
            if !source.is_char_boundary(offset) {
                return Err(Failure::new(
                    "invalid_offset",
                    "Use a UTF-8 byte boundary within the source file.",
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
        WriterCommand::Edit {
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
                    "Use an ordered range on UTF-8 byte boundaries within the source file.",
                )
            })?;
            overrides.insert(file.to_owned(), updated.clone());
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
                // An editor outside Studio can change a saved include while
                // compilation runs. Keep the approved graph pinned to its bytes.
                let latest =
                    load_bounded_source_workspace(entry, &draft.overrides(name), MAX_STORY_BYTES);
                let latest_proposal =
                    load_bounded_source_workspace(entry, &overrides, MAX_STORY_BYTES)
                        .map_err(|_| source_changed())?;
                if latest.as_ref().ok() != workspace.as_ref().ok() || latest_proposal != proposed {
                    return Err(source_changed());
                }
                change = Some(if included {
                    let path = included_edit_path(entry, file).map_err(|_| source_changed())?;
                    let mut sources = draft
                        .includes
                        .clone()
                        .map_or_else(|| AdditionalSources::new(entry), Ok)
                        .map_err(|_| source_changed())?;
                    if sources
                        .get(file)
                        .is_some_and(|document| document.source() != source)
                    {
                        return Err(source_changed());
                    }
                    let current = sources.open(file).map_err(|_| source_changed())?;
                    if current.path() != Some(path.as_path()) || current.source() != source {
                        return Err(source_changed());
                    }
                    Change::Include {
                        file: file.to_owned(),
                        sources,
                        source: updated,
                    }
                } else {
                    Change::Entry(updated)
                });
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

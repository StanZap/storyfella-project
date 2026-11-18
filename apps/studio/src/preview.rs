//! Compiles the open story, drafts included, into a playback session.
use std::{collections::BTreeMap, path::PathBuf};

use storyfella_compiler::{SourceWorkspace, WorkspaceError};
use storyfella_language::has_includes;
use storyfella_runtime::AssetPolicy;
use storyfella_studio_core::playback::PlaybackSession;

use crate::state::AppState;

/// The story's entry file and every unsaved draft, keyed by path relative
/// to the entry's folder: what [`workspace`] loads, as data a worker can take.
pub fn drafts(state: &AppState) -> Result<(PathBuf, BTreeMap<String, String>), WorkspaceError> {
    let unbound = || {
        WorkspaceError::entry(
            String::new(),
            "Save this story before previewing included files".into(),
        )
    };
    let path = state.story.path().ok_or_else(unbound)?;
    let entry = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            WorkspaceError::entry(String::new(), "Story filename is not UTF-8".into())
        })?;
    let mut drafts = state
        .additional_sources
        .as_ref()
        .map_or_else(Default::default, |sources| sources.overrides());
    drafts.insert(entry.to_owned(), state.story.source().to_owned());
    Ok((path.to_path_buf(), drafts))
}

/// The story's files as they stand in the editor, unsaved drafts included.
pub fn workspace(state: &AppState) -> Result<SourceWorkspace, WorkspaceError> {
    let (entry, drafts) = drafts(state)?;
    storyfella_studio_core::story::load_source_workspace(&entry, &drafts)
}

/// A playback session of the story as it stands in the editor.
pub fn compile(state: &AppState) -> Result<PlaybackSession, String> {
    if has_includes(state.story.source()) {
        workspace(state)
            .map_err(|error| error.to_string())
            .and_then(|sources| {
                PlaybackSession::compile_workspace(&sources, &state.registry, AssetPolicy::Preview)
            })
    } else {
        PlaybackSession::compile_with_policy(
            state.story.source(),
            &state.registry,
            AssetPolicy::Preview,
        )
    }
}

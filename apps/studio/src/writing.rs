//! The story as the Write view edits it: the files it spans, what each says,
//! and what the language makes of them. Nothing here draws; the view and the
//! tests share it.
//!
//! A file is named by its path relative to the entry story's folder, the
//! same key [`SourceWorkspace`] uses, and every position is a byte offset
//! into that file.
use storyfella_compiler::{include_path, SourceWorkspace};
use storyfella_language::{
    analyze, apply_edits, complete, has_includes, semantic_tokens, Analysis, Completion, Span,
};
use storyfella_studio_core::authoring::AssetCatalog;

use crate::state::AppState;

/// What a span of source is, for coloring it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Highlight {
    Keyword,
    Text,
    Comment,
    /// A name that points at a declaration or an asset.
    Reference,
    /// A scene, beat or flag where it is declared, or a jump to one.
    Symbol,
}

/// The entry story's file name.
pub fn entry(state: &AppState) -> Option<String> {
    state
        .story
        .path()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .map(str::to_owned)
}

/// The file the Write view shows: the open included file, or the entry.
pub fn active(state: &AppState) -> Option<String> {
    state.active_source.clone().or_else(|| entry(state))
}

/// The files of the story, entry first, then each included file by path,
/// and the open file even before anything includes it.
pub fn files(state: &AppState, workspace: Option<&SourceWorkspace>) -> Vec<String> {
    let Some(entry) = entry(state) else {
        return Vec::new();
    };
    let mut files = vec![entry.clone()];
    if let Some(workspace) = workspace {
        files.extend(
            workspace
                .files
                .keys()
                .filter(|file| **file != entry)
                .cloned(),
        );
    }
    if let Some(open) = &state.active_source {
        if !files.contains(open) {
            files.push(open.clone());
        }
    }
    files
}

/// What `file` says in the editor, drafts included.
pub fn source<'a>(state: &'a AppState, file: &str) -> Option<&'a str> {
    if entry(state).as_deref() == Some(file) {
        return Some(state.story.source());
    }
    state
        .additional_sources
        .as_ref()?
        .get(file)
        .map(|document| document.source())
}

/// Replaces what `file` says, as one undoable edit.
pub fn set_source(state: &mut AppState, file: &str, text: String) -> Result<(), String> {
    if entry(state).as_deref() == Some(file) {
        state.set_story_source(text);
        return Ok(());
    }
    state
        .set_additional_source(file, text)
        .map_err(|error| error.to_string())
}

/// Replaces what `file` says with what someone typed at `now`; a run of
/// typing is one undo step.
pub fn type_into(
    state: &mut AppState,
    file: &str,
    text: String,
    now: std::time::Instant,
) -> Result<(), String> {
    state.typed(file, now, |state| set_source(state, file, text))
}

/// Shows `file` in the editor, loading an included file from disk.
pub fn open(state: &mut AppState, file: &str) -> Result<(), String> {
    if entry(state).as_deref() == Some(file) {
        state.active_source = None;
        return Ok(());
    }
    if state.active_source.as_deref() == Some(file) {
        return Ok(());
    }
    state
        .open_additional_source(file)
        .map_err(|error| error.to_string())
}

/// The spans of `source` worth coloring, in order.
pub fn highlights(state: &AppState, source: &str) -> Vec<(Span, Highlight)> {
    let catalog = AssetCatalog::from_registry(&state.registry);
    semantic_tokens(&analyze(source, &catalog.assets))
        .into_iter()
        .filter_map(|token| {
            let highlight = match token.kind {
                0 => Highlight::Keyword,
                1 => Highlight::Text,
                2 => Highlight::Comment,
                3 => Highlight::Reference,
                4 => Highlight::Symbol,
                _ => return None,
            };
            Some((token.span, highlight))
        })
        .collect()
}

/// The story's files with `file` reading `text`, as the compiler sees them.
fn workspace_with(state: &AppState, file: &str, text: &str) -> Option<SourceWorkspace> {
    let (entry, mut drafts) = crate::preview::drafts(state).ok()?;
    drafts.insert(file.to_owned(), text.to_owned());
    storyfella_studio_core::story::load_source_workspace(&entry, &drafts).ok()
}

/// What could go at `offset` of `file` while it reads `text`, which may be
/// newer than the story: the editor asks before its edit reaches the state.
pub fn completions(state: &AppState, file: &str, text: &str, offset: usize) -> Vec<Completion> {
    let catalog = AssetCatalog::from_registry(&state.registry);
    let spans_files = entry(state)
        .and_then(|entry| source(state, &entry).map(has_includes))
        .unwrap_or(false);
    if spans_files {
        if let Some(found) = workspace_completions(state, file, text, offset, &catalog) {
            return found;
        }
    }
    complete(&analyze(text, &catalog.assets), offset, &catalog.assets)
}

fn workspace_completions(
    state: &AppState,
    file: &str,
    text: &str,
    offset: usize,
    catalog: &AssetCatalog,
) -> Option<Vec<Completion>> {
    let expanded = workspace_with(state, file, text)?.expand().ok()?;
    // The caret at the end of a file sits past its last segment.
    let mapped = expanded.expanded_offset(file, offset).or_else(|| {
        let previous = offset.checked_sub(1)?;
        expanded
            .expanded_offset(file, previous)
            .map(|mapped| mapped + 1)
    })?;
    let analysis = analyze(&expanded.source, &catalog.assets);
    Some(
        complete(&analysis, mapped, &catalog.assets)
            .into_iter()
            .filter_map(|mut completion| {
                let (owner, span) = expanded.original_span(completion.replace).or_else(|| {
                    if completion.replace.start != completion.replace.end {
                        return None;
                    }
                    let previous = completion.replace.start.checked_sub(1)?;
                    let (owner, span) =
                        expanded.original_span(Span::new(previous, previous + 1))?;
                    Some((owner, Span::new(span.end, span.end)))
                })?;
                (owner == file).then(|| {
                    completion.replace = span;
                    completion
                })
            })
            .collect(),
    )
}

/// A place in the story: a file and a byte offset into it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub file: String,
    pub offset: usize,
}

/// What the symbol under the caret leads to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Navigation {
    pub definition: Option<Place>,
    pub references: Vec<Place>,
    /// The declared name, when the caret is on something that can be renamed.
    pub rename: Option<String>,
}

/// Where the name at `offset` of `file` is declared and used, across files.
/// On an `include` line, the definition is the included file.
pub fn navigation(state: &AppState, file: &str, offset: usize) -> Navigation {
    let Some(text) = source(state, file) else {
        return Navigation::default();
    };
    let catalog = AssetCatalog::from_registry(&state.registry);
    let local = analyze(text, &catalog.assets);
    let spans_files = entry(state)
        .and_then(|entry| source(state, &entry).map(has_includes))
        .unwrap_or(false);
    if spans_files {
        if let Some(include) = local
            .document
            .story
            .includes
            .iter()
            .find(|include| include.span.contains(offset))
        {
            return Navigation {
                definition: Some(Place {
                    file: include_path(file, &include.value),
                    offset: 0,
                }),
                ..Navigation::default()
            };
        }
        if let Some(expanded) = crate::preview::workspace(state)
            .ok()
            .and_then(|workspace| workspace.expand().ok())
        {
            if let Some(mapped) = expanded.expanded_offset(file, offset) {
                let analysis = analyze(&expanded.source, &catalog.assets);
                return locate(&analysis, mapped, |span| {
                    let (owner, span) = expanded.original_span(span)?;
                    Some(Place {
                        file: owner.to_owned(),
                        offset: span.start,
                    })
                });
            }
        }
    }
    locate(&local, offset, |span| {
        Some(Place {
            file: file.to_owned(),
            offset: span.start,
        })
    })
}

fn locate(analysis: &Analysis, offset: usize, place: impl Fn(Span) -> Option<Place>) -> Navigation {
    let definition = analysis.definition_at(offset);
    Navigation {
        definition: definition.and_then(&place),
        references: analysis
            .references_at(offset, true)
            .into_iter()
            .filter_map(&place)
            .collect(),
        rename: analysis
            .symbols
            .iter()
            .find(|symbol| Some(symbol.name.span) == definition)
            .map(|symbol| symbol.name.value.clone()),
    }
}

/// Renames the symbol at `offset` of `file` everywhere it is used.
pub fn rename(state: &mut AppState, file: &str, offset: usize, name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Name the symbol.".into());
    }
    if entry(state).as_deref() == Some(file) {
        state.rename_story_symbol(offset, name)
    } else {
        state.rename_workspace_symbol(file, offset, name)
    }
}

/// Evens out the spacing of `file`; its text, comments and indentation stay.
pub fn format(state: &mut AppState, file: &str) -> Result<(), String> {
    let text = source(state, file).ok_or("Open a story file to format it.")?;
    let catalog = AssetCatalog::from_registry(&state.registry);
    let edits = storyfella_language::format(&analyze(text, &catalog.assets))?;
    if edits.is_empty() {
        return Ok(());
    }
    let formatted = apply_edits(text, &edits)?;
    set_source(state, file, formatted)
}

#[cfg(test)]
mod tests;

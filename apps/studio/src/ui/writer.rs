//! The Writer — the story as text, with the artifacts it names beside it.
//!
//! Left: the `.sf` source, highlighted. Right: the artifacts the cursor's
//! scene references, resolved ones with their picture and pending ones as
//! placeholders that offer to create them. Writing the story is what fills
//! the asset catalog with work.
//!
//! Uses the same tokens, AST and reference analysis as the external LSP.
//! Unresolved assets remain pending, with a Create affordance.

use dioxus::html::HasFileData;
use dioxus::prelude::*;
use storyfella_language::{analyze, Analysis, AssetKind, ReferenceKind, Severity, TokenKind};
use uuid::Uuid;

use storyfella_studio_core::authoring::AssetCatalog;
use storyfella_studio_core::errors::{CodedError, ReportedError};
use storyfella_studio_core::registry::{ArtifactKind, ArtifactRegistry};

use crate::state::AppState;

use super::{
    components::IconButton,
    generated_asset_url,
    icons::{Icon, IconName},
};

#[derive(Clone, Copy)]
struct WriterAssetView(Signal<bool>);

fn project_diagnostics(
    state: &AppState,
) -> Vec<(
    String,
    storyfella_language::Diagnostic,
    storyfella_language::Position,
)> {
    let catalog = AssetCatalog::from_registry(&state.registry);
    let compiled = super::preview::workspace(state).and_then(|sources| {
        let (compilation, expanded) = storyfella_compiler::compile_workspace_with_catalog(
            &sources,
            &catalog.assets,
            &catalog.bindings,
        )?;
        Ok((sources, compilation, expanded))
    });
    let (sources, compilation, expanded) = match compiled {
        Ok(compiled) => compiled,
        Err(error) => return vec![(error.path.clone(), error.diagnostic(), error.start)],
    };
    compilation
        .diagnostics
        .into_iter()
        .filter_map(|mut diagnostic| {
            let (path, span) = expanded.diagnostic_span(&sources.entry, diagnostic.span);
            let position =
                storyfella_language::Source::new(&sources.files[path]).position(span.start)?;
            diagnostic.span = span;
            Some((path.to_owned(), diagnostic, position))
        })
        .collect()
}

type SourcePosition = (String, storyfella_language::Position);

#[derive(Default)]
struct SymbolNavigation {
    definition: Option<SourcePosition>,
    references: Vec<SourcePosition>,
    rename: Option<String>,
}

fn symbol_navigation(
    state: &AppState,
    file: &str,
    local: &Analysis,
    offset: usize,
    catalog: &AssetCatalog,
) -> SymbolNavigation {
    use storyfella_language::{Position, Source, Span};
    let locations = |analysis: &Analysis, offset, map: &dyn Fn(Span) -> Option<SourcePosition>| {
        let definition = analysis.definition_at(offset);
        SymbolNavigation {
            definition: definition.and_then(map),
            references: analysis
                .references_at(offset, true)
                .into_iter()
                .filter_map(map)
                .collect(),
            rename: analysis
                .symbols
                .iter()
                .find(|symbol| Some(symbol.name.span) == definition)
                .map(|symbol| symbol.name.value.clone()),
        }
    };
    if storyfella_language::has_includes(state.story.source()) {
        if let Some(include) = local
            .document
            .story
            .includes
            .iter()
            .find(|include| include.span.contains(offset))
        {
            return SymbolNavigation {
                definition: Some((
                    storyfella_compiler::include_path(file, &include.value),
                    Position::default(),
                )),
                ..Default::default()
            };
        }
        if let Ok(sources) = super::preview::workspace(state) {
            if let Ok(expanded) = sources.expand() {
                if let Some(offset) = expanded.expanded_offset(file, offset) {
                    let analysis = analyze(&expanded.source, &catalog.assets);
                    return locations(&analysis, offset, &|span| {
                        let (file, span) = expanded.original_span(span)?;
                        let position = Source::new(&sources.files[file]).position(span.start)?;
                        Some((file.to_owned(), position))
                    });
                }
            }
        }
    }
    locations(local, offset, &|span| {
        let position = local.document.source.position(span.start)?;
        Some((file.to_owned(), position))
    })
}

pub(super) fn navigate_source(
    state: &mut AppState,
    caret: super::WriterCaret,
    file: &str,
    position: storyfella_language::Position,
) -> Result<(), storyfella_studio_core::story::StoryFileError> {
    let entry = state
        .story
        .path()
        .and_then(std::path::Path::file_name)
        .and_then(|name| name.to_str());
    if file.is_empty() || entry == Some(file) {
        state.active_source = None;
    } else if state.active_source.as_deref() != Some(file) {
        state.open_additional_source(file)?;
    }
    caret.navigate(position);
    Ok(())
}

#[component]
fn ReferenceButtons(
    positions: Vec<SourcePosition>,
    on_navigate: EventHandler<SourcePosition>,
) -> Element {
    rsx! {
        for (file, position) in positions {
            button { class: "mr-2 font-mono text-[11px] text-violet-300",
                title: format!("Open reference · {}:{}", position.line + 1, position.character + 1),
                onclick: { let file = file.clone(); move |_| on_navigate.call((file.clone(), position)) },
                if !file.is_empty() { "{file} · " }
                "{position.line + 1}:{position.character + 1}"
            }
        }
    }
}

fn ancestors_visible(path: &str, collapsed: &std::collections::BTreeSet<String>) -> bool {
    let mut parent = std::path::Path::new(path).parent();
    while let Some(current) = parent {
        if current.as_os_str().is_empty() {
            break;
        }
        if collapsed.contains(
            &current
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/"),
        ) {
            return false;
        }
        parent = current.parent();
    }
    true
}

fn workspace_completions(
    state: &AppState,
    path: &str,
    offset: usize,
) -> Option<Vec<storyfella_language::Completion>> {
    let sources = super::preview::workspace(state).ok()?;
    let expanded = sources.expand().ok()?;
    let offset = expanded.expanded_offset(path, offset).or_else(|| {
        offset.checked_sub(1).and_then(|previous| {
            expanded
                .expanded_offset(path, previous)
                .map(|mapped| mapped + 1)
        })
    })?;
    let catalog = AssetCatalog::from_registry(&state.registry);
    let analysis = analyze(&expanded.source, &catalog.assets);
    Some(
        storyfella_language::complete(&analysis, offset, &catalog.assets)
            .into_iter()
            .filter_map(|mut completion| {
                let (file, span) = expanded.original_span(completion.replace).or_else(|| {
                    if completion.replace.start != completion.replace.end {
                        return None;
                    }
                    let previous = completion.replace.start.checked_sub(1)?;
                    let (file, span) = expanded
                        .original_span(storyfella_language::Span::new(previous, previous + 1))?;
                    Some((file, storyfella_language::Span::new(span.end, span.end)))
                })?;
                if file != path {
                    return None;
                }
                completion.replace = span;
                Some(completion)
            })
            .collect(),
    )
}

fn accept_additional_completion(
    mut state: Signal<AppState>,
    path: &str,
    completion: &storyfella_language::Completion,
    mut navigation: Signal<Option<storyfella_language::Position>>,
) {
    let source = state
        .read()
        .additional_sources
        .as_ref()
        .and_then(|sources| sources.get(path))
        .map(|document| document.source().to_owned());
    let Some(source) = source else {
        return;
    };
    if let Ok(edited) = storyfella_language::apply_edits(
        &source,
        &[storyfella_language::TextEdit {
            replace: completion.replace,
            new_text: completion.label.clone(),
        }],
    ) {
        let position = storyfella_language::Source::new(&edited)
            .position(completion.replace.start + completion.label.len());
        if state
            .write()
            .set_additional_source(path, edited.clone())
            .is_ok()
        {
            if let Some(position) = position {
                update_editor_after_completion("svs-extra-writer", &source, &edited, position);
                navigation.set(None);
            }
        }
    }
}

/// DOM selections use UTF-16; shared Source converts them to UTF-8 spans.
const CARET_BRIDGE: &str = r#"
    const el = document.getElementById("svs-writer");
    if (el && !el.dataset.bridged) {
        el.dataset.bridged = "1";
        const push = () => {
            const caret = el.selectionStart ?? 0;
            const lines = el.value.slice(0, caret).split("\n");
            dioxus.send({line: lines.length - 1, character: lines.at(-1).length});
        };
        el.addEventListener("keydown", event => {
            if (event.key === "Tab" && !event.shiftKey && !event.ctrlKey && !event.altKey && !event.metaKey) event.preventDefault();
        });
        for (const name of ["input", "keyup", "click", "focus", "select"]) {
            el.addEventListener(name, push);
        }
    }
"#;

fn insert_indent(id: &str) {
    document::eval(&format!(
        r#"const el = document.getElementById({}); if (el) {{ el.setRangeText('  ', el.selectionStart, el.selectionEnd, 'end'); el.dispatchEvent(new Event('input', {{ bubbles: true }})); el.focus(); }}"#,
        serde_json::to_string(id).unwrap()
    ));
}

fn focus_position(position: storyfella_language::Position) {
    focus_in("svs-writer", position);
}

fn focus_in(id: &str, position: storyfella_language::Position) {
    document::eval(&format!(
        r#"const el = document.getElementById({});
        if (el) {{ const lines = el.value.split("\n");
          const line = Math.min({}, lines.length - 1);
          const offset = lines.slice(0, line).reduce((n, s) => n + s.length + 1, 0) + Math.min({}, lines[line].length);
          el.focus(); el.setSelectionRange(offset, offset); }}"#,
        serde_json::to_string(id).unwrap(),
        position.line,
        position.character
    ));
}

fn update_editor_after_completion(
    id: &str,
    previous: &str,
    edited: &str,
    position: storyfella_language::Position,
) {
    let previous = serde_json::to_string(&previous.replace("\r\n", "\n")).unwrap();
    let edited = serde_json::to_string(&edited.replace("\r\n", "\n")).unwrap();
    let id = serde_json::to_string(id).unwrap();
    document::eval(&format!(
        r#"const el = document.getElementById({id});
        if (el && el.value === {previous}) {{
            el.value = {edited};
            const lines = el.value.split('\n');
            const line = Math.min({}, lines.length - 1);
            const offset = lines.slice(0, line).reduce((n, s) => n + s.length + 1, 0) + Math.min({}, lines[line].length);
            el.focus(); el.setSelectionRange(offset, offset);
        }}"#,
        position.line, position.character
    ));
}

fn accept_completion(
    mut state: Signal<AppState>,
    caret: super::WriterCaret,
    completion: &storyfella_language::Completion,
) {
    let source = state.read().story.source().to_owned();
    if let Ok(edited) = storyfella_language::apply_edits(
        &source,
        &[storyfella_language::TextEdit {
            replace: completion.replace,
            new_text: completion.label.clone(),
        }],
    ) {
        let position = storyfella_language::Source::new(&edited)
            .position(completion.replace.start + completion.label.len())
            .unwrap();
        state.write().set_story_source(edited.clone());
        update_editor_after_completion("svs-writer", &source, &edited, position);
        let mut caret = caret;
        caret.position.set(position);
    }
}

/// Which scene and beat a line belongs to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scope {
    pub scene: Option<String>,
    pub beat: Option<String>,
}

fn outline_analysis(analysis: &Analysis) -> Vec<Scope> {
    let source = &analysis.document.source;
    (0..source.line_count())
        .map(|line| {
            let span = source.line_span(line).unwrap();
            let offset = if span.start == source.text().len() {
                span.start.saturating_sub(1)
            } else {
                span.start + source.slice(span).unwrap().len()
                    - source.slice(span).unwrap().trim_start().len()
            };
            let (scene, beat) = analysis.scope_at(offset);
            Scope {
                scene: scene.map(str::to_owned),
                beat: beat.map(str::to_owned),
            }
        })
        .collect()
}

#[cfg(test)]
fn outline(source: &str) -> Vec<Scope> {
    outline_analysis(&analyze(source, &[]))
}

/// A name the story references, and whether the registry has it.
#[derive(Clone, Debug, PartialEq)]
pub struct Mention {
    pub name: String,
    pub artifact: Option<Uuid>,
    pub image: Option<String>,
    pub kind: Option<ArtifactKind>,
    pub can_create: bool,
}

impl Mention {
    pub fn pending(&self) -> bool {
        self.artifact.is_none()
    }
}

#[cfg(test)]
fn names_in(line: &str) -> Vec<String> {
    analyze(&format!("scene _preview\n  {}", line.trim_start()), &[])
        .references
        .into_iter()
        .filter(|r| matches!(r.kind, ReferenceKind::Asset(_)))
        .map(|r| r.name.value)
        .collect()
}

fn mentions_in(
    analysis: &Analysis,
    line: usize,
    registry: &ArtifactRegistry,
) -> (Vec<Mention>, Vec<Mention>) {
    let scopes = outline_analysis(analysis);
    let here = scopes.get(line).cloned().unwrap_or_default();
    let mut beat_names = Vec::new();
    let mut scene_names = Vec::new();
    for reference in &analysis.references {
        if !matches!(reference.kind, ReferenceKind::Asset(_))
            || here.scene.as_deref() != Some(&reference.scene)
        {
            continue;
        }
        let target = if here.beat.is_some() && here.beat == reference.beat {
            &mut beat_names
        } else {
            &mut scene_names
        };
        if !target.contains(&reference.name.value) {
            target.push(reference.name.value.clone());
        }
    }
    scene_names.retain(|name| !beat_names.contains(name));
    let resolve = |names: Vec<String>| -> Vec<Mention> {
        names
            .into_iter()
            .map(|name| {
                let reference = analysis.references.iter().find(|r| {
                    matches!(r.kind, ReferenceKind::Asset(_))
                        && r.name.value == name
                        && here.scene.as_deref() == Some(r.scene.as_str())
                });
                let artifact = reference
                    .and_then(|r| r.asset_id.as_ref())
                    .and_then(|id| registry.artifacts.iter().find(|a| a.id.to_string() == *id));
                let expected_kind = reference.and_then(|r| match r.kind {
                    ReferenceKind::Asset(AssetKind::Character) => Some(ArtifactKind::Character),
                    ReferenceKind::Asset(AssetKind::Location) => Some(ArtifactKind::Environment),
                    ReferenceKind::Asset(AssetKind::Prop) => Some(ArtifactKind::Object),
                    ReferenceKind::Asset(AssetKind::Illustration) => {
                        Some(ArtifactKind::Illustration)
                    }
                    ReferenceKind::Scene | ReferenceKind::Flag => None,
                });
                Mention {
                    can_create: reference.is_some_and(|r| {
                        analysis
                            .diagnostics
                            .iter()
                            .any(|d| d.code == "SF203" && d.span == r.name.span)
                    }),
                    image: artifact
                        .and_then(AppState::display_image)
                        .and_then(generated_asset_url),
                    kind: expected_kind,
                    artifact: artifact.map(|a| a.id),
                    name,
                }
            })
            .collect()
    };
    (resolve(beat_names), resolve(scene_names))
}

#[cfg(test)]
fn mentions_at(
    source: &str,
    line: usize,
    registry: &ArtifactRegistry,
) -> (Vec<Mention>, Vec<Mention>) {
    mentions_in(
        &analyze(source, &AssetCatalog::from_registry(registry).assets),
        line,
        registry,
    )
}

/// One coloured run of a line.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub text: String,
    pub class: &'static str,
}

fn highlight_line(analysis: &Analysis, line: usize) -> Vec<Piece> {
    let source = &analysis.document.source;
    let Some(span) = source.line_span(line) else {
        return Vec::new();
    };
    highlight_span(analysis, span)
}

pub(super) fn highlight_span(analysis: &Analysis, span: storyfella_language::Span) -> Vec<Piece> {
    let source = &analysis.document.source;
    analysis.document.tokens.iter().filter(|t| t.span.start < span.end && t.span.end > span.start).map(|token| {
        let reference = analysis.references.iter().find(|r| r.name.span == token.span && matches!(r.kind, ReferenceKind::Asset(_)));
        let class = if let Some(reference) = reference {
            if reference.asset_id.is_some() {
                "rounded-sm bg-sky-400/15 text-sky-200 shadow-[0_0_0_2px_rgba(56,189,248,.15)]"
            } else {
                "rounded-sm bg-amber-400/15 text-amber-300 shadow-[0_0_0_2px_rgba(251,191,36,.15)]"
            }
        } else {
            match token.kind {
                TokenKind::Keyword => "text-sky-300 font-semibold",
                TokenKind::String | TokenKind::Text => "text-emerald-300",
                TokenKind::Comment => "text-zinc-700 italic",
                TokenKind::Whitespace => "",
                _ => "text-zinc-400",
            }
        };
        let visible = storyfella_language::Span::new(token.span.start.max(span.start), token.span.end.min(span.end));
        Piece { text: source.slice(visible).unwrap().to_owned(), class }
    }).collect()
}

#[cfg(test)]
fn highlight(line: &str, known: &[String]) -> Vec<Piece> {
    let indent = line.len() - line.trim_start().len();
    let catalog: Vec<storyfella_language::Asset> = known
        .iter()
        .map(|name| storyfella_language::Asset {
            id: name.clone(),
            name: name.clone(),
            kind: AssetKind::Character,
        })
        .collect();
    let analysis = analyze(
        &format!("scene _preview\n  {}", line.trim_start()),
        &catalog,
    );
    let mut pieces = highlight_line(&analysis, 1);
    if let Some(first) = pieces.first_mut() {
        first.text = line[..indent].to_owned();
    }
    pieces
}

#[component]
fn RenameSymbolDialog(
    app_state: Signal<AppState>,
    offset: usize,
    name: String,
    #[props(default)] file: Option<String>,
    onclose: EventHandler<()>,
) -> Element {
    let mut replacement = use_signal(|| name.clone());
    let mut error = use_signal(|| None::<String>);
    let (id, mount) = super::text_input::use_text_value(replacement(), None);
    rsx! {
        super::dialog::Dialog { title: "Rename symbol", on_close: onclose,
            form { class: "studio-task-form", onsubmit: move |event| {
                event.prevent_default();
                let result = if let Some(file) = &file { app_state.write().rename_workspace_symbol(file, offset, replacement().trim()) } else { app_state.write().rename_story_symbol(offset, replacement().trim()) };
                match result {
                    Ok(()) => onclose.call(()),
                    Err(message) => error.set(Some(message)),
                }
            },
                header { class: "flex items-center gap-3",
                    h2 { class: "flex-1 text-xl font-semibold", "Rename {name}" }
                    IconButton { label: "Close", icon: IconName::Close, onclick: move |_| onclose.call(()) }
                }
                label { class: "studio-field", r#for: id.clone(), span { "New name" }
                    input { id, onmounted: mount, autofocus: true, class: "setting-control font-mono", aria_label: "New symbol name",
                        oninput: move |event| { replacement.set(event.value()); error.set(None); }
                    }
                }
                if let Some(message) = error() { p { role: "alert", class: "studio-inline-error", "{message}" } }
                footer { class: "flex justify-end gap-2",
                    button { r#type: "button", class: "studio-button", title: "Cancel", onclick: move |_| onclose.call(()), "Cancel" }
                    button { r#type: "submit", class: "studio-button studio-button-primary", title: "Rename", disabled: replacement().trim().is_empty(), "Rename" }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ProjectEntryKind {
    StoryFile,
    Folder,
}

#[component]
fn CreateProjectEntry(
    kind: ProjectEntryKind,
    folder: String,
    mut app_state: Signal<AppState>,
    oncreated: EventHandler<()>,
    onclose: EventHandler<()>,
) -> Element {
    let mut name = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let (label, placeholder) = match kind {
        ProjectEntryKind::StoryFile => ("File name", "New file name"),
        ProjectEntryKind::Folder => ("Folder name", "New folder name"),
    };
    rsx! {
        form { class: "studio-task-form studio-explorer-form",
            onsubmit: move |event| {
                event.prevent_default();
                let mut name = name().trim().to_owned();
                if name.is_empty() {
                    error.set(Some(format!("Enter a {}", label.to_lowercase())));
                    return;
                }
                let root = app_state.read().story.path().and_then(|path| path.parent()).map(std::path::Path::to_path_buf);
                let Some(root) = root else { return; };
                let result = match kind {
                    ProjectEntryKind::StoryFile => {
                        if !name.ends_with(".sf") {
                            name.push_str(".sf");
                        }
                        super::project_files::destination(&root, &folder, &name)
                            .map_err(|error| error.to_string())
                            .and_then(|path| app_state.write().create_additional_source(&path).map_err(|error| error.to_string()))
                    }
                    ProjectEntryKind::Folder => {
                        super::project_files::create_folder(&root, &folder, &name).map_err(|error| error.to_string())
                    }
                };
                match result {
                    Ok(()) => oncreated.call(()),
                    Err(message) => error.set(Some(message)),
                }
            },
            super::inputs::DismissibleTextInput {
                label, value: name(), placeholder, autofocus: true,
                oninput: move |value| { name.set(value); error.set(None); },
                ondismiss: move |_| onclose.call(()),
            }
            if let Some(message) = error() {
                p { role: "alert", class: "studio-inline-error", "{message}" }
            }
        }
    }
}

fn explorer_rename_key(key: &Key, macos: bool) -> bool {
    if macos {
        *key == Key::Enter
    } else {
        *key == Key::F2
    }
}

fn explorer_rename_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "Enter"
    } else {
        "F2"
    }
}

#[component]
pub fn Writer(
    mut app_state: Signal<AppState>,
    backend: Signal<storyfella_studio_core::planner::PlannerBackend>,
) -> Element {
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
    let sidebar_visible = use_context::<super::SidebarVisibility>().0;
    let mut files = use_signal(Vec::<super::project_files::Entry>::new);
    let mut folder = use_signal(String::new);
    let mut item_menu = use_signal(|| None::<String>);
    let mut rename_item = use_signal(|| None::<String>);
    let mut rename_name = use_signal(String::new);
    let mut rename_asset = use_signal(|| None::<uuid::Uuid>);
    let mut create_entry = use_signal(|| None::<(ProjectEntryKind, String)>);
    let mut file_preview = use_context::<super::ProjectFilePreview>().0;
    let mut open_asset = use_signal(|| false);
    let novel_proposal =
        use_signal(|| None::<storyfella_studio_core::workflows::novel::NovelProposal>);
    let showing_novel_review = novel_proposal.read().is_some();
    use_context_provider(|| WriterAssetView(open_asset));
    let mut new_asset = use_signal(|| None::<super::actions::ArtAction>);
    let mut collapsed = use_signal(std::collections::BTreeSet::<String>::new);
    let mut scan_error = use_signal(|| None::<String>);
    let mut drop_error = use_signal(|| None::<String>);
    let mut refresh = use_signal(|| 0u64);
    let path = app_state
        .read()
        .story
        .path()
        .map(std::path::Path::to_path_buf);
    use_effect(use_reactive((&path,), move |_| {
        open_asset.set(false);
        file_preview.set(None);
        folder.set(String::new());
        collapsed.set(Default::default());
        create_entry.set(None);
        item_menu.set(None);
        rename_item.set(None);
        rename_asset.set(None);
        drop_error.set(None);
    }));
    let revision = refresh();
    use_effect(use_reactive(
        (&path, &revision),
        move |(path, _)| match path.as_deref().map(super::project_files::list) {
            Some(Ok(found)) => {
                files.set(found);
                scan_error.set(None);
            }
            Some(Err(error)) => {
                files.set(Vec::new());
                scan_error.set(Some(error.to_string()));
            }
            None => {
                files.set(Vec::new());
                scan_error.set(None);
            }
        },
    ));
    let entry = path
        .as_ref()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let active = app_state.read().active_source.clone();
    let included = super::preview::workspace(&app_state.read())
        .ok()
        .map(|workspace| {
            workspace
                .files
                .into_keys()
                .collect::<std::collections::BTreeSet<_>>()
        });
    let mut caret = use_context::<super::WriterCaret>();
    let selected_source = active
        .as_ref()
        .and_then(|path| {
            app_state
                .read()
                .additional_sources
                .as_ref()
                .and_then(|sources| sources.get(path))
                .map(|document| document.source().to_owned())
        })
        .unwrap_or_else(|| app_state.read().story.source().to_owned());
    let selected_analysis = analyze(&selected_source, &[]);
    let selected_offset = selected_analysis
        .document
        .source
        .offset((caret.position)())
        .unwrap_or(0);
    let selected_scene = selected_analysis
        .scope_at(selected_offset)
        .0
        .map(str::to_owned);
    let mut scene_signal = use_context::<Signal<Option<String>>>();
    use_effect(use_reactive((&selected_scene,), move |(scene,)| {
        scene_signal.set(scene)
    }));
    let assets: Vec<_> = app_state
        .read()
        .registry
        .artifacts
        .iter()
        .filter(|item| {
            matches!(
                item.kind,
                ArtifactKind::Character
                    | ArtifactKind::Environment
                    | ArtifactKind::Object
                    | ArtifactKind::Illustration
            )
        })
        .map(|item| (item.id, item.name.clone()))
        .collect();
    let creation = create_entry();
    let creation_form = creation.as_ref().map(|(kind, target)| {
        let kind = *kind;
        let target = target.clone();
        let key = format!("{kind:?}-{target}");
        rsx! {
            CreateProjectEntry { key: "{key}", kind, folder: target.clone(), app_state,
                onclose: move |_| create_entry.set(None),
                oncreated: move |_| {
                    collapsed.write().remove(&target);
                    refresh += 1;
                    create_entry.set(None);
                    if kind == ProjectEntryKind::StoryFile {
                        file_preview.set(None);
                        open_asset.set(false);
                        caret.position.set(storyfella_language::Position::default());
                    }
                },
            }
        }
    });
    rsx! {
        div { class: "flex h-full min-h-0 min-w-0",
            aside { hidden: !sidebar_visible(), class: if sidebar_visible() { "flex w-52 shrink-0 flex-col overflow-y-auto border-r border-white/10 p-3" } else { "hidden" }, aria_label: "Project explorer", ondragover: move |event| event.prevent_default(), ondrop: move |event| {
                event.prevent_default();
                let root = app_state.read().story.path().and_then(|path| path.parent()).map(std::path::Path::to_path_buf);
                if let Some(root) = root { for file in event.files() {
                    if let Err(error) = super::project_files::import_file(&root, &folder(), &file.path()) { drop_error.set(Some(format!("Could not add {}: {error}", file.name()))); } else { refresh += 1; }
                } }
            },
                if let Some(notice) = app_state.read().source_notice.clone() {
                    div { class: "mb-3 text-xs text-amber-300", role: "status", "{notice}"
                        button { class: "ml-1 underline", title: "Dismiss notice", onclick: move |_| app_state.write().source_notice = None, "Dismiss" }
                    }
                }
                div { class: "studio-explorer-header flex items-center gap-2",
                    button { class: "mr-auto text-xs font-semibold text-zinc-400", title: "Select project root", onclick: move |_| folder.set(String::new()), "Project" }
                    super::components::ActionMenu { label: "Add to project", icon: IconName::Add,
                        options: vec![
                            super::components::MenuOption { label: "New story file".into(), icon: IconName::Book, disabled: false },
                            super::components::MenuOption { label: "New folder".into(), icon: IconName::Folder, disabled: false },
                            super::components::MenuOption { label: "New artifact".into(), icon: IconName::Image, disabled: false },
                        ],
                        onselect: move |index| match index {
                            0 | 1 => {
                                let target = folder();
                                collapsed.write().remove(&target);
                                let kind = if index == 0 { ProjectEntryKind::StoryFile } else { ProjectEntryKind::Folder };
                                create_entry.set(Some((kind, target)));
                            },
                            2 => new_asset.set(Some(super::actions::ArtAction::Create(ArtifactKind::Character))),
                            _ => {},
                        },
                    }
                    IconButton { label: "Refresh project files", icon: IconName::Refresh, onclick: move |_| refresh += 1 }
                }
                if let Some(error) = scan_error() { div { class: "text-xs text-red-400", role: "alert", "Couldn't load project files" super::components::ErrorDetailsButton { error: ReportedError::plain(storyfella_studio_core::errors::ErrorCode::AppCommandInvalid, error) } } }
                if let Some(error) = drop_error() { div { class: "text-xs text-red-400", role: "alert", "Couldn't update project files" super::components::ErrorDetailsButton { error: ReportedError::plain(storyfella_studio_core::errors::ErrorCode::AppCommandInvalid, error) } button { class: "ml-1 underline", title: "Dismiss error", onclick: move |_| drop_error.set(None), "Dismiss" } } }
                if creation.as_ref().is_some_and(|(_, target)| target.is_empty()) {
                    {creation_form.clone()}
                }
                for row in files() {
                    {
                        let menu_path = row.path.clone();
                        let rename_path = row.path.clone();
                        let item_path = row.path.clone();
                        let item_name = row.name.clone();
                        let entry_item = item_path == entry;
                        let visible_item = ancestors_visible(&item_path, &collapsed.read());
                        rsx! { if visible_item { div { class: "relative", oncontextmenu: move |event| { event.prevent_default(); event.stop_propagation(); item_menu.set(Some(menu_path.clone())); },
                        if rename_item.read().as_deref() == Some(item_path.as_str()) {
                            form { onsubmit: move |event| {
                                event.prevent_default();
                                let name = rename_name().trim().to_owned();
                                let root = app_state.read().story.path().and_then(|p| p.parent()).map(std::path::Path::to_path_buf);
                                if let Some(root) = root {
                                    match super::project_files::rename(&mut app_state.write(), &root, &rename_path, &name) {
                                        Ok(()) => { rename_item.set(None); refresh += 1; }
                                        Err(error) => drop_error.set(Some(error.to_string())),
                                    }
                                }
                            },
                                input { onmounted: move |_| { document::eval("const input = document.querySelector('.studio-explorer-rename'); if (input) { input.focus(); input.select(); }"); }, class: "studio-explorer-rename w-full bg-zinc-800 text-xs", style: "padding-left: {8 + row.depth * 12}px", value: "{rename_name}", autofocus: true, onblur: move |_| rename_item.set(None), oninput: move |e| rename_name.set(e.value()), onkeydown: move |e| { if e.key() == Key::Escape { e.prevent_default(); e.stop_propagation(); rename_item.set(None); } } }
                            }
                        } else {
                        { if row.folder {
                            let folder_path = row.path.clone();
                            let drop_folder = folder_path.clone();
                            let rename_folder = folder_path.clone();
                            let rename_folder_name = item_name.clone();
                            let is_open = !collapsed.read().contains(&folder_path);
                            let visible = ancestors_visible(&folder_path, &collapsed.read());
                            rsx! { if visible { button { key: "folder-{folder_path}", title: format!("{} {} · Rename ({})", if is_open { "Collapse" } else { "Expand" }, row.name, explorer_rename_hint()), style: "padding-left: {8 + row.depth * 12}px", class: if folder() == folder_path { "flex items-center gap-1 rounded bg-white/10 py-1 text-left text-xs text-zinc-100" } else { "flex items-center gap-1 rounded py-1 text-left text-xs text-zinc-400 hover:bg-white/5" }, aria_expanded: is_open, onkeydown: move |event| { if event.modifiers().is_empty() && explorer_rename_key(&event.key(), cfg!(target_os = "macos")) { event.prevent_default(); event.stop_propagation(); rename_name.set(rename_folder_name.clone()); rename_item.set(Some(rename_folder.clone())); } }, onclick: move |_| { let mut set = collapsed.write(); if !set.remove(&folder_path) { set.insert(folder_path.clone()); } folder.set(folder_path.clone()); }, ondragover: move |event| event.prevent_default(), ondrop: move |event| {
                                event.prevent_default(); event.stop_propagation();
                                let root = app_state.read().story.path().and_then(|path| path.parent()).map(std::path::Path::to_path_buf);
                                if let Some(root) = root { for file in event.files() { if let Err(error) = super::project_files::import_file(&root, &drop_folder, &file.path()) { drop_error.set(Some(format!("Could not add {}: {error}", file.name()))); } else { collapsed.write().remove(&drop_folder); refresh += 1; } } }
                            },
                                Icon { name: if is_open { IconName::ChevronDown } else { IconName::Arrow }, class: "size-3" } Icon { name: IconName::Folder, class: "size-4" } "{row.name}"
                            } } }
                        } else {
                        let file = row.path;
                        let visible = ancestors_visible(&file, &collapsed.read());
                        let is_story = file.ends_with(".sf");
                        let is_entry = file == entry;
                        let selected = match file_preview.read().as_deref() {
                            Some(preview) => path.as_deref().and_then(std::path::Path::parent).is_some_and(|root| preview == root.join(&file)),
                            None => !open_asset() && if is_entry { active.is_none() } else { active.as_deref() == Some(file.as_str()) },
                        };
                        let label = if is_story && !is_entry && included.as_ref().is_some_and(|sources| !sources.contains(&file)) { format!("{} · not included", row.name) } else { row.name };
                        let target = file.clone();
                        let rename_target = file.clone();
                        let rename_label = item_name.clone();
                        rsx! { if visible { button { key: "{file}", style: "padding-left: {8 + row.depth * 12}px", class: if selected { "w-full rounded bg-white/10 py-1 text-left text-xs text-zinc-100" } else { "w-full rounded py-1 text-left text-xs text-zinc-400 hover:bg-white/5" },
                            title: if is_entry { format!("Open {file}") } else { format!("Open {file} · Rename ({})", explorer_rename_hint()) }, onkeydown: move |event| {
                                if !is_entry && event.modifiers().is_empty() && explorer_rename_key(&event.key(), cfg!(target_os = "macos")) {
                                    event.prevent_default();
                                    event.stop_propagation();
                                    rename_name.set(rename_label.clone());
                                    rename_item.set(Some(rename_target.clone()));
                                }
                            }, onclick: move |_| {
                                if !is_story {
                                    let root = app_state.read().story.path().and_then(|path| path.parent()).map(std::path::Path::to_path_buf);
                                    if let Some(root) = root {
                                        match super::project_files::existing(&root, &target) {
                                            Ok(path) => file_preview.set(Some(path)),
                                            Err(error) => { drop_error.set(Some(error.to_string())); return; }
                                        }
                                    }
                                }
                                else { file_preview.set(None); if is_entry { app_state.write().active_source = None; }
                                else if let Err(error) = app_state.write().open_additional_source(&target) { error_report.set(Some(error.report().log())); }
                                caret.position.set(storyfella_language::Position::default()); }
                                open_asset.set(false);
                            }, "{label}"
                        } } }
                        } }
                        }
                        if row.folder && creation.as_ref().is_some_and(|(_, target)| target == &item_path) {
                            {creation_form.clone()}
                        }
                        if item_menu.read().as_deref() == Some(item_path.as_str()) {
                            div { class: "studio-explorer-menu-dismiss", aria_hidden: "true", onclick: move |_| item_menu.set(None), oncontextmenu: move |event| { event.prevent_default(); item_menu.set(None); } }
                            div { class: "studio-action-menu-list", style: "position:absolute;left:20px;top:20px", role: "menu",
                                button { class: "studio-action-menu-item", title: format!("Rename ({})", explorer_rename_hint()), disabled: entry_item, onclick: { let p = item_path.clone(); let n = item_name.clone(); move |_| { rename_name.set(n.clone()); rename_item.set(Some(p.clone())); item_menu.set(None); } }, "Rename" }
                                button { class: "studio-action-menu-item", title: "Duplicate", onclick: { let p = item_path.clone(); move |_| { let root = app_state.read().story.path().and_then(|p| p.parent()).map(std::path::Path::to_path_buf); if let Some(root) = root { if let Err(e) = super::project_files::duplicate(&root, &p) { drop_error.set(Some(e.to_string())); } else { refresh += 1; } } item_menu.set(None); } }, "Duplicate" }
                                button { class: "studio-action-menu-item", title: "Copy absolute path", onclick: { let p = item_path.clone(); move |_| { let root = app_state.read().story.path().and_then(|p| p.parent()).map(std::path::Path::to_path_buf); if let Some(root) = root { match super::project_files::existing(&root, &p) { Ok(path) => { let value = serde_json::to_string(&path.to_string_lossy().to_string()).unwrap(); document::eval(&format!("navigator.clipboard.writeText({value})")); }, Err(e) => drop_error.set(Some(e.to_string())) } } item_menu.set(None); } }, "Copy absolute path" }
                                button { class: "studio-action-menu-item", title: "Copy", onclick: { let p = item_path.clone(); move |_| { let root = app_state.read().story.path().and_then(|p| p.parent()).map(std::path::Path::to_path_buf); if let Some(root) = root { match super::project_files::existing(&root, &p) { Ok(path) => { if let Err(e) = super::project_files::copy_item(&path) { drop_error.set(Some(e.to_string())); } }, Err(e) => drop_error.set(Some(e.to_string())) } } item_menu.set(None); } }, "Copy" }
                                button { class: "studio-action-menu-item", title: "Delete", disabled: entry_item, onclick: { let p = item_path.clone(); move |_| {
                                    let confirmed = rfd::MessageDialog::new().set_title("Delete item?").set_description(format!("Delete {p}? This cannot be undone.")).set_buttons(rfd::MessageButtons::OkCancel).show() == rfd::MessageDialogResult::Ok;
                                    if confirmed {
                                        let root = app_state.read().story.path().and_then(|p| p.parent()).map(std::path::Path::to_path_buf);
                                        if let Some(root) = root {
                                            match super::project_files::remove(&mut app_state.write(), &root, &p) {
                                                Ok(()) => refresh += 1,
                                                Err(error) => drop_error.set(Some(error.to_string())),
                                            }
                                        }
                                    }
                                    item_menu.set(None);
                                } }, "Delete" }
                            }
                        }
                        } } }
                    }
                }
                for (id, name) in assets {
                    button { key: "{id}", title: format!("Open {name} · Rename ({})", explorer_rename_hint()), class: if open_asset() && app_state.read().selected_artifact_id == Some(id) { "rounded bg-white/10 px-2 py-1 text-left text-xs text-zinc-100" } else { "rounded px-2 py-1 text-left text-xs text-zinc-400 hover:bg-white/5" }, onkeydown: move |event| { if event.modifiers().is_empty() && explorer_rename_key(&event.key(), cfg!(target_os = "macos")) && !app_state.read().operation_in_progress() { event.prevent_default(); event.stop_propagation(); file_preview.set(None); app_state.write().select_artifact(id); open_asset.set(true); rename_asset.set(Some(id)); } }, onclick: move |_| { file_preview.set(None); app_state.write().select_artifact(id); open_asset.set(true); }, "{name}" }
                }
            }
            div { class: "flex min-w-0 flex-1 flex-col",
                div { hidden: !showing_novel_review && (active.is_some() || open_asset() || file_preview.read().is_some()), class: "min-h-0 flex-1", MainWriter { app_state, backend, novel_proposal } }
                if let Some(path) = active.clone() { div { hidden: showing_novel_review || open_asset() || file_preview.read().is_some(), class: "min-h-0 flex-1", AdditionalWriter { key: "{path}", path, app_state } } }
                if let Some(path) = file_preview().filter(|_| !showing_novel_review) { div { class: "flex h-full flex-col", header { class: "border-b border-white/10 p-3 text-sm", "{path.file_name().unwrap_or_default().to_string_lossy()}" }
                    if path.extension().and_then(|s| s.to_str()).is_some_and(|ext| ["png", "jpg", "jpeg", "webp"].iter().any(|supported| ext.eq_ignore_ascii_case(supported))) {
                        if let Some(url) = path.canonicalize().ok().and_then(|path| path.to_str().and_then(generated_asset_url)) { img { class: "min-h-0 flex-1 object-contain p-4", src: url, alt: "Project image" } }
                    } else { p { class: "p-4 text-sm text-zinc-400", "File in project · open with its associated application to edit." } }
                } }
                if open_asset() && !showing_novel_review { if let Some(artifact) = app_state.read().selected_artifact().cloned() {
                    super::assets::AssetDetail { key: "{artifact.id}", rename_requested: rename_asset() == Some(artifact.id), on_rename_started: move |_| rename_asset.set(None), app_state, artifact, disabled: app_state.read().operation_in_progress(), on_back: move |_| open_asset.set(false), on_new_variant: move |id| new_asset.set(Some(super::actions::ArtAction::Variant(id))) }
                } }
            }
        }
        if let Some(action) = new_asset() { super::art_actions::ArtActionDialog { action, app_state, onclose: move |_| new_asset.set(None), oncreated: move |_| { file_preview.set(None); open_asset.set(true); } } }
    }
}

const COMPLETION_POSITION: &str = r#"
const menu = document.querySelector('.studio-completions');
const editor = document.getElementById(EDITOR_ID);
if (menu && editor) {
  const place = () => {
    if (!menu.isConnected) { controller.abort(); observer.disconnect(); return; }
    const style = getComputedStyle(editor), rect = editor.getBoundingClientRect();
    const mirror = document.createElement('div');
    for (const key of ['font','letterSpacing','lineHeight','padding','border','boxSizing','whiteSpace','overflowWrap','wordBreak']) mirror.style[key] = style[key];
    Object.assign(mirror.style, {position:'fixed', visibility:'hidden', width:rect.width+'px', whiteSpace:'pre-wrap', overflowWrap:'break-word', left:'-10000px', top:'0'});
    mirror.textContent = editor.value.slice(0, editor.selectionStart);
    const caret = document.createElement('span'); caret.textContent = '\u200b'; mirror.appendChild(caret); document.body.appendChild(mirror);
    const x = rect.left + caret.offsetLeft - editor.scrollLeft;
    const y = rect.top + caret.offsetTop - editor.scrollTop + parseFloat(style.lineHeight);
    mirror.remove();
    menu.style.left = Math.max(8, Math.min(x, innerWidth - menu.offsetWidth - 8))+'px';
    menu.style.top = Math.max(8, Math.min(y, innerHeight - menu.offsetHeight - 8))+'px';
  };
  const controller = new AbortController();
  const observer = new MutationObserver(() => {
    if (!menu.isConnected) { controller.abort(); observer.disconnect(); }
  });
  observer.observe(menu.parentNode, { childList: true });
  const schedule = () => requestAnimationFrame(place);
  for (const event of ['input', 'keyup', 'click', 'select', 'scroll']) editor.addEventListener(event, schedule, { signal: controller.signal });
  window.addEventListener('resize', schedule, { signal: controller.signal });
  document.addEventListener('scroll', schedule, { capture: true, signal: controller.signal });
  requestAnimationFrame(place);
}
"#;

#[component]
fn AdditionalWriter(path: String, mut app_state: Signal<AppState>) -> Element {
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
    let writer_caret = use_context::<super::WriterCaret>();
    let mut navigation = writer_caret.target;
    let mut writer_position = writer_caret.position;
    let mut mounted = use_signal(|| false);
    let mut rename_target = use_signal(|| None::<(usize, String)>);
    let mut cursor = use_signal(storyfella_language::Position::default);
    let mut show_completions = use_signal(|| false);
    let mut show_references = use_signal(|| false);
    let mut completion_index = use_signal(|| 0usize);
    let source = app_state
        .read()
        .additional_sources
        .as_ref()
        .and_then(|sources| sources.get(&path))
        .map(|document| document.source().to_owned())
        .unwrap_or_default();
    let catalog = AssetCatalog::from_registry(&app_state.read().registry);
    let analysis = analyze(&source, &catalog.assets);
    let offset = analysis.document.source.offset(cursor()).unwrap_or(0);
    let completions = if show_completions() {
        workspace_completions(&app_state.read(), &path, offset)
            .unwrap_or_else(|| storyfella_language::complete(&analysis, offset, &catalog.assets))
    } else {
        Vec::new()
    };
    let selected_completion = completion_index().min(completions.len().saturating_sub(1));
    let key_completions = completions.clone();
    let included = match super::preview::workspace(&app_state.read()) {
        Ok(workspace) => workspace.files.contains_key(&path),
        Err(error) => error.path == path,
    };
    let symbols = symbol_navigation(&app_state.read(), &path, &analysis, offset, &catalog);
    let definition = symbols.definition;
    let rename_symbol = symbols.rename.filter(|_| included);
    let reference_positions = symbols.references;
    let on_navigate = EventHandler::new(move |(file, position): SourcePosition| {
        if let Err(error) = navigate_source(&mut app_state.write(), writer_caret, &file, position) {
            error_report.set(Some(error.report().log()));
        }
    });
    let problems = if included {
        project_diagnostics(&app_state.read())
    } else {
        analysis
            .diagnostics
            .iter()
            .filter_map(|diagnostic| {
                analysis
                    .document
                    .source
                    .position(diagnostic.span.start)
                    .map(|position| (path.clone(), diagnostic.clone(), position))
            })
            .collect()
    };
    let mut previous_source = use_signal(|| source.clone());
    use_effect(use_reactive((&source,), move |(source,)| {
        let previous = previous_source.peek().replace("\r\n", "\n");
        previous_source.set(source.clone());
        let next = source.replace("\r\n", "\n");
        document::eval(&format!(
            "const el = document.getElementById('svs-extra-writer'); if (el && el.value === {}) el.value = {};",
            serde_json::to_string(&previous).unwrap(),
            serde_json::to_string(&next).unwrap()
        ));
        document::eval(
            r#"const el = document.getElementById('svs-extra-writer'); if (el) { el.style.height = 'auto'; el.style.height = Math.max(el.scrollHeight, el.parentElement.clientHeight) + 'px'; }"#,
        );
    }));
    let target = path.clone();
    use_effect(move || {
        if mounted() {
            if let Some(position) = navigation() {
                focus_in("svs-extra-writer", position);
                navigation.set(None);
            }
        }
    });
    rsx! {
        section { class: "flex h-full min-h-0 flex-col", aria_label: "Story file editor",
            header { class: "flex items-center gap-2 border-b border-white/10 px-4 py-2 text-xs text-zinc-400",
                span { class: "min-w-0 flex-1 truncate", "{path}" }
                IconButton { label: "Go to definition", icon: IconName::Definition, disabled: definition.is_none(),
                    onclick: move |_| { if let Some(location) = definition.clone() { on_navigate.call(location); } }
                }
                IconButton { label: if show_references() { "Hide references" } else { "Show references" }, icon: IconName::References,
                    expanded: show_references(), disabled: reference_positions.is_empty() && !show_references(),
                    onclick: move |_| show_references.toggle()
                }
                IconButton { label: "Rename symbol", icon: IconName::Edit, disabled: rename_symbol.is_none(),
                    onclick: move |_| { if let Some(name) = rename_symbol.clone() { rename_target.set(Some((offset, name))); } }
                }
            }
            if show_references() {
                div { class: "max-h-24 overflow-auto px-4 py-1", role: "region", aria_label: "Story references",
                    ReferenceButtons { positions: reference_positions, on_navigate }
                }
            }
            div { class: "studio-writer-surface relative min-h-0 flex-1 overflow-auto",
              div { aria_hidden: "true", class: "pointer-events-none absolute inset-0 z-10 whitespace-pre-wrap px-4 py-3 font-mono text-[12.5px] leading-[1.75]",
                for index in 0..analysis.document.source.line_count() {
                    div { for piece in highlight_line(&analysis, index) { span { class: piece.class, "{piece.text}" } } if analysis.document.source.line_span(index).is_some_and(|span| span.start == span.end) { span { "\u{200b}" } } }
                }
              }
              textarea { id: "svs-extra-writer", aria_label: "Story source", spellcheck: "false", autocomplete: "off",
                class: "absolute inset-x-0 top-0 z-20 min-h-full w-full resize-none overflow-hidden bg-transparent px-4 py-3 font-mono text-[12.5px] leading-[1.75] text-transparent caret-sky-300 outline-none placeholder:text-zinc-600",
                onmounted: { let initial = source.clone(); move |_| {
                    document::eval(&format!("const el = document.getElementById('svs-extra-writer'); el.value = {};", serde_json::to_string(&initial.replace("\r\n", "\n")).unwrap())); mounted.set(true);
                    spawn(async move { let script = CARET_BRIDGE.replace("svs-writer", "svs-extra-writer"); let mut bridge = document::eval(&script); while let Ok(position) = bridge.recv::<storyfella_language::Position>().await { cursor.set(position); writer_position.set(position); } });
                } },
                oninput: move |event| {
                    let mut state = app_state.write();
                    if let Some(document) = state.additional_sources.as_ref().and_then(|sources| sources.get(&target)) {
                        let edited = storyfella_language::textarea_input(document.source(), &event.value());
                        if let Err(error) = state.set_additional_source(&target, edited) { error_report.set(Some(error.report().log())); }
                    }
                    show_completions.set(true); completion_index.set(0);
                },
                onkeydown: { let file = path.clone(); move |event| {
                    if event.key() == Key::Tab && !event.modifiers().contains(Modifiers::SHIFT) && (!show_completions() || key_completions.is_empty()) { event.prevent_default(); insert_indent("svs-extra-writer"); return; }
                    if event.modifiers().ctrl() && event.key() == Key::Character(" ".into()) { event.prevent_default(); show_completions.set(true); }
                    if show_completions() && !key_completions.is_empty() { match event.key() {
                        Key::ArrowDown => { event.prevent_default(); completion_index.set((selected_completion + 1) % key_completions.len()); }
                        Key::ArrowUp => { event.prevent_default(); completion_index.set((selected_completion + key_completions.len() - 1) % key_completions.len()); }
                        Key::Enter | Key::Tab if !event.modifiers().contains(Modifiers::SHIFT) => { event.prevent_default(); accept_additional_completion(app_state, &file, &key_completions[selected_completion], navigation); show_completions.set(false); }
                        Key::Escape => { event.prevent_default(); show_completions.set(false); }
                        _ => {}
                    } }
                } },
              }
            }
            if !completions.is_empty() { div { class: "studio-completions", role: "listbox", onmounted: move |_| { document::eval(&COMPLETION_POSITION.replace("EDITOR_ID", "'svs-extra-writer'")); },
                for (index, completion) in completions.iter().enumerate() {
                    button { class: if index == selected_completion { "studio-completion is-selected" } else { "studio-completion" }, title: "{completion.label}", onclick: { let completion = completion.clone(); let file = path.clone(); move |_| { accept_additional_completion(app_state, &file, &completion, navigation); show_completions.set(false); } }, "{completion.label}" }
                }
            } }
            details { class: "max-h-40 overflow-auto border-t border-white/10 px-4 py-2 text-xs text-zinc-400",
                    summary { "Problems ({problems.iter().filter(|(file, _, _)| file == &path).count()})" }
                    for (_, diagnostic, position) in problems.into_iter().filter(|(file, _, _)| file == &path) {
                        button { class: "block py-1 text-left", title: "Open problem: {diagnostic.message}", onclick: move |_| navigation.set(Some(position)), "{position.line + 1}:{position.character + 1} · {diagnostic.message}" }
                    }
            }
            if let Some((offset, name)) = rename_target() {
                RenameSymbolDialog { app_state, offset, name, file: Some(path.clone()), onclose: move |_| rename_target.set(None) }
            }
        }
    }
}

#[component]
fn MainWriter(
    app_state: Signal<AppState>,
    backend: Signal<storyfella_studio_core::planner::PlannerBackend>,
    novel_proposal: Signal<Option<storyfella_studio_core::workflows::novel::NovelProposal>>,
) -> Element {
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
    let caret = use_context::<super::WriterCaret>();
    let mut cursor = caret.position;
    let mut navigation = caret.target;
    let mut screen = use_context::<Signal<super::AppScreen>>();
    let mut show_completions = use_signal(|| false);
    let mut completion_index = use_signal(|| 0usize);
    let mut show_references = use_signal(|| false);
    let mut rename_target = use_signal(|| None::<(usize, String)>);
    let mut problem_filter = use_signal(|| "all".to_owned());
    let mut show_problems = use_signal(|| false);
    let mut edit_error = use_signal(|| None::<String>);
    let mut writing_help = use_context::<super::WritingHelp>().0;
    let review_controller = super::writing_review::ReviewController {
        review: use_signal(|| None),
        mode: use_signal(super::writing_review::ReviewMode::default),
        error: use_signal(|| None),
        busy: use_signal(|| false),
        changes: use_signal(Vec::new),
        selected: use_signal(|| 0),
        marked: use_signal(Vec::new),
        sequence: use_signal(|| 0),
        ready: use_signal(|| false),
        bridge: use_signal(|| None),
        retry: use_signal(|| 0),
    };
    use_context_provider(|| review_controller);
    let mut review_mode = review_controller.mode;
    let reviewing = review_controller.review.read().is_some();
    let novel_review = novel_proposal();
    let remaining = review_controller.changes.read().len();
    let mut had_review = use_signal(|| false);
    use_effect(move || {
        let now = review_controller.review.read().is_some();
        if *had_review.peek() && !now {
            focus_position(cursor());
        }
        had_review.set(now);
    });
    use_effect(move || {
        if writing_help() {
            document::eval(
                "document.querySelector('[aria-label=\"Writing instructions\"]')?.focus();",
            );
        }
    });
    let source = app_state.read().story.source().to_owned();
    let mut previous_source = use_signal(|| source.clone());
    let story_path = app_state
        .read()
        .story
        .path()
        .map(std::path::Path::to_path_buf);
    let registry = app_state.read().registry.clone();
    let catalog = AssetCatalog::from_registry(&registry);
    let analysis = analyze(&source, &catalog.assets);
    let entry = story_path
        .as_deref()
        .and_then(std::path::Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let problems = if storyfella_language::has_includes(&source) {
        project_diagnostics(&app_state.read())
    } else {
        storyfella_compiler::compile_analysis(&analysis, &catalog.bindings)
            .diagnostics
            .into_iter()
            .filter_map(|diagnostic| {
                let position = analysis.document.source.position(diagnostic.span.start)?;
                Some((entry.to_owned(), diagnostic, position))
            })
            .collect()
    };
    let error_count = problems
        .iter()
        .filter(|(_, d, _)| d.severity == Severity::Error)
        .count();
    let warning_count = problems.len() - error_count;
    let problem_label = format!(
        "Problems · {error_count} {} · {warning_count} {}",
        if error_count == 1 { "error" } else { "errors" },
        if warning_count == 1 {
            "warning"
        } else {
            "warnings"
        }
    );
    let diagnostics: Vec<_> = problems
        .iter()
        .filter(|(_, d, _)| match problem_filter().as_str() {
            "errors" => d.severity == Severity::Error,
            "warnings" => d.severity != Severity::Error,
            _ => true,
        })
        .collect();
    let offset = analysis.document.source.offset(cursor()).unwrap_or(0);
    let completions = if show_completions() {
        let entry = story_path
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        workspace_completions(&app_state.read(), entry, offset)
            .unwrap_or_else(|| storyfella_language::complete(&analysis, offset, &catalog.assets))
    } else {
        Vec::new()
    };
    let selected_completion = completion_index().min(completions.len().saturating_sub(1));
    let key_completions = completions.clone();
    let symbols = symbol_navigation(&app_state.read(), entry, &analysis, offset, &catalog);
    let definition = symbols.definition;
    let rename_symbol = symbols.rename;
    let reference_positions = symbols.references;
    let on_navigate = EventHandler::new(move |(file, position): SourcePosition| {
        let mut state = app_state;
        let result = navigate_source(&mut state.write(), caret, &file, position);
        if let Err(error) = result {
            error_report.set(Some(error.report().log()));
        }
    });

    let lines: Vec<String> = if source.is_empty() {
        Vec::new()
    } else {
        source
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned())
            .collect()
    };
    let scopes = outline_analysis(&analysis);
    let at = (cursor().line as usize).min(lines.len().saturating_sub(1));
    // A controlled textarea can overwrite newer native keystrokes with an
    // older queued render. Only replace DOM text when it still matches the
    // previous snapshot; native input already holds the new text.
    use_effect(use_reactive((&source,), move |(source,)| {
        let previous = previous_source.peek().replace("\r\n", "\n");
        previous_source.set(source.clone());
        let next = source.replace("\r\n", "\n");
        document::eval(&format!(
            "const el = document.getElementById('svs-writer'); if (el && el.value === {}) el.value = {};",
            serde_json::to_string(&previous).unwrap(),
            serde_json::to_string(&next).unwrap()
        ));
    }));
    use_effect(move || {
        if app_state.read().active_source.is_none() {
            if let Some(position) = navigation() {
                focus_position(position);
                navigation.set(None);
            }
        }
    });
    // Keep both editor layers on the parent's scroll surface. Following the
    // caret also makes driver typing visible when it appends below the viewport.
    use_effect(use_reactive((&source, &at), move |(_, line)| {
        document::eval(&format!(
            r#"
            const el = document.getElementById("svs-writer");
            if (el) {{
                const grow = () => {{
                    el.style.height = "auto";
                    el.style.height = Math.max(el.scrollHeight, el.parentElement.clientHeight) + "px";
                }};
                grow();
                if (!el.dataset.sizing) {{
                    el.dataset.sizing = "1";
                    new ResizeObserver(grow).observe(el.parentElement);
                }}
            }}
            document.getElementById("svs-writer-line-{line}")?.scrollIntoView({{block: "nearest"}});
            "#
        ));
    }));
    let here = scopes.get(at).cloned().unwrap_or_default();
    let scene_artwork = registry
        .story_scenes
        .iter()
        .find(|s| Some(&s.name) == here.scene.as_ref());
    let in_scope_artwork: Vec<(String, Uuid, ArtifactKind)> = scene_artwork
        .into_iter()
        .flat_map(|scene| {
            std::iter::once((scene.name.clone(), scene.artifact_id, ArtifactKind::Scene)).chain(
                scene
                    .beats
                    .iter()
                    .filter(|b| Some(&b.name) == here.beat.as_ref())
                    .map(|b| (b.name.clone(), b.artifact_id, ArtifactKind::Beat)),
            )
        })
        .collect();
    let atelier_target = scene_artwork.map(|scene| scene.artifact_id);
    let (beat_mentions, scene_mentions) = mentions_in(&analysis, at, &registry);
    let pending = beat_mentions
        .iter()
        .chain(scene_mentions.iter())
        .filter(|mention| mention.pending())
        .count();

    rsx! {
        // `h-full` matters: both editor layers are absolutely positioned and
        // contribute no intrinsic height, so without a definite height from
        // here the pane collapses to nothing.
        div { class: "flex h-full min-h-0",

            // ------------------------------------------------------ editor
            div { class: "flex min-w-0 flex-1 flex-col border-r border-white/[0.055]",
                div { class: "flex items-center gap-2 border-b border-white/[0.05] px-4 py-2.5",
                    p { class: "text-[10px] font-semibold uppercase tracking-[0.16em] text-zinc-600",
                        "Writer"
                    }
                    IconButton {
                        label: "Save a copy of the story", icon: IconName::Copy,
                        onclick: move |_| {
                            let Some(mut path) = rfd::FileDialog::new()
                                .set_title("Save a copy of the story")
                                .add_filter("Storyfella source", &["sf"])
                                .set_file_name("story-copy.sf")
                                .save_file() else { return; };
                            if path.extension().is_none() { path.set_extension("sf"); }
                            if let Err(error) = app_state.read().story.save_copy(&path) {
                                error_report.set(Some(error.report().log()));
                            }
                        },
                    }
                    if let Some(path) = story_path.as_ref() {
                        p { class: "min-w-0 truncate font-mono text-[10px] text-zinc-500", title: "{path.display()}",
                            "{path.file_name().unwrap_or_default().to_string_lossy()}"
                        }
                        IconButton {
                            label: "Reload story from disk", icon: IconName::Refresh,
                            onclick: move |_| {
                                let dirty = app_state.read().story.is_dirty();
                                if dirty && rfd::MessageDialog::new()
                                    .set_title("Reload story from disk?")
                                    .set_description("Your unsaved story edits will be discarded. Artwork changes stay in the project.")
                                    .set_buttons(rfd::MessageButtons::OkCancel)
                                    .show() != rfd::MessageDialogResult::Ok {
                                    return;
                                }
                                if let Err(error) = crate::commands::reload_story(app_state) {
                                    error_report.set(Some(error.log()));
                                }
                            },
                        }
                    }
                    p { class: "ml-auto font-mono text-[10px] text-zinc-700",
                        if let Some(scene) = here.scene.clone() {
                            if let Some(beat) = here.beat.clone() { "{scene} › {beat}" } else { "{scene}" }
                        } else {
                            "—"
                        }
                    }
                }

                if let Some(proposal) = &novel_review {
                    div { class: "flex min-h-0 flex-1 flex-col",
                        div { class: "studio-review-toolbar", aria_label: "Review view",
                            p { class: "mr-auto text-xs text-zinc-400", "{proposal.files.len()} changed files" }
                            button { class: if review_mode() == super::writing_review::ReviewMode::Inline { "is-selected" } else { "" }, aria_pressed: review_mode() == super::writing_review::ReviewMode::Inline, onclick: move |_| review_mode.set(super::writing_review::ReviewMode::Inline), "Inline" }
                            button { class: if review_mode() == super::writing_review::ReviewMode::SideBySide { "is-selected" } else { "" }, aria_pressed: review_mode() == super::writing_review::ReviewMode::SideBySide, onclick: move |_| review_mode.set(super::writing_review::ReviewMode::SideBySide), "Side by side" }
                        }
                        div { class: "min-h-0 flex-1 overflow-auto", aria_label: "Changed story files",
                            for (index, (path, replacement)) in proposal.files.iter().enumerate() {
                                super::writing_review::NovelFileDiff {
                                    key: "{proposal.run_id}-{index}-{review_mode() == super::writing_review::ReviewMode::SideBySide}",
                                    index,
                                    path: path.clone(),
                                    draft: proposal.request.workspace.files[path].clone(),
                                    proposed: replacement.clone(),
                                    mode: review_mode(),
                                }
                            }
                        }
                    }
                } else if reviewing {
                  div { key: "{review_controller.review.read().as_ref().unwrap().proposal.run_id}", class: "flex min-h-0 flex-1 flex-col",
                    div { class: "studio-review-toolbar", aria_label: "Review view",
                        p { class: "mr-auto text-xs text-zinc-400", if remaining == 1 { "1 change remaining" } else { "{remaining} changes remaining" } }
                        button { class: if review_mode() == super::writing_review::ReviewMode::Inline { "is-selected" } else { "" }, title: "Review changes in one continuous editor", aria_pressed: review_mode() == super::writing_review::ReviewMode::Inline, onclick: move |_| review_mode.set(super::writing_review::ReviewMode::Inline), "Inline" }
                        button { class: if review_mode() == super::writing_review::ReviewMode::SideBySide { "is-selected" } else { "" }, title: "Compare the draft and suggestion side by side", aria_pressed: review_mode() == super::writing_review::ReviewMode::SideBySide, onclick: move |_| review_mode.set(super::writing_review::ReviewMode::SideBySide), "Side by side" }
                    }
                    super::writing_review::WritingReviewEditor { app_state }
                  }
                }
                div { hidden: reviewing || novel_review.is_some(), class: "flex min-h-0 flex-1 flex-col",
                    // The rendered layer is the visible text; the textarea
                    // sits over it transparently, exactly as the command bar
                    // does, so highlighting and the caret share metrics.
                    div { class: "studio-writer-surface relative min-h-0 flex-1 overflow-auto",
                        div { aria_hidden: "true", class: "pointer-events-none absolute inset-0 z-10 px-4 py-3 font-mono text-[12.5px] leading-[1.75] whitespace-pre-wrap",
                            for (index , line) in lines.iter().enumerate() {
                                div {
                                    key: "{index}",
                                    id: "svs-writer-line-{index}",
                                    class: if index == at { "-mx-4 border-l-2 border-sky-400 bg-sky-400/[0.07] pl-[14px] pr-4" } else { "-mx-4 border-l-2 border-transparent pl-[14px] pr-4" },
                                    for (slot , piece) in highlight_line(&analysis, index).iter().enumerate() {
                                        span { key: "{slot}", class: piece.class, "{piece.text}" }
                                    }
                                    if line.is_empty() {
                                        span { "\u{200b}" }
                                    }
                                }
                            }
                        }
                        textarea {
                            aria_label: "Story source",
                            placeholder: "Write your story…",
                            id: "svs-writer",
                            class: "absolute inset-x-0 top-0 z-20 min-h-full w-full resize-none overflow-hidden bg-transparent px-4 py-3 font-mono text-[12.5px] leading-[1.75] text-transparent caret-sky-300 outline-none",
                            spellcheck: "false",
                            autocomplete: "off",
                            onmounted: { let initial = source.clone(); move |_| {
                                document::eval(&format!("document.getElementById('svs-writer').value = {};", serde_json::to_string(&initial.replace("\r\n", "\n")).unwrap()));
                                focus_position(cursor());
                                spawn(async move {
                                    let mut bridge = document::eval(CARET_BRIDGE);
                                    while let Ok(position) = bridge.recv::<storyfella_language::Position>().await { cursor.set(position); }
                                });
                            } },
                            oninput: move |event| {
                                let mut state = app_state.write();
                                let edited = storyfella_language::textarea_input(state.story.source(), &event.value());
                                state.set_story_source(edited);
                                show_completions.set(true);
                                completion_index.set(0);
                            },
                            onkeydown: move |event| {
                                if event.key() == Key::Tab && !event.modifiers().contains(Modifiers::SHIFT) && (!show_completions() || key_completions.is_empty()) { event.prevent_default(); insert_indent("svs-writer"); return; }
                                if event.modifiers().ctrl() && event.key() == Key::Character(" ".into()) { event.prevent_default(); show_completions.set(true); }
                                if show_completions() && !key_completions.is_empty() {
                                    match event.key() {
                                        Key::ArrowDown => { event.prevent_default(); completion_index.set((selected_completion + 1) % key_completions.len()); }
                                        Key::ArrowUp => { event.prevent_default(); completion_index.set((selected_completion + key_completions.len() - 1) % key_completions.len()); }
                                        Key::Enter => { event.prevent_default(); accept_completion(app_state, caret, &key_completions[selected_completion]); show_completions.set(false); }
                                        Key::Tab if !event.modifiers().contains(Modifiers::SHIFT) => { event.prevent_default(); accept_completion(app_state, caret, &key_completions[selected_completion]); show_completions.set(false); }
                                        Key::Escape => { event.prevent_default(); show_completions.set(false); }
                                        _ => {}
                                    }
                                }
                            },
                        }
                    }
                    if !completions.is_empty() {
                        div { class: "studio-completions", role: "listbox", onmounted: move |_| { document::eval(&COMPLETION_POSITION.replace("EDITOR_ID", "'svs-writer'")); },
                            for (index, completion) in completions.iter().enumerate() {
                                button {
                                    class: if index == selected_completion { "studio-completion is-selected" } else { "studio-completion" },
                                    title: "{completion.label}",
                                    onclick: { let completion = completion.clone(); move |_| { accept_completion(app_state, caret, &completion); show_completions.set(false); } },
                                    "{completion.label}"
                                }
                            }
                        }
                    }
                    div { class: "flex flex-wrap items-center gap-2 border-t border-white/[0.05] px-4 py-2 text-[11px] text-zinc-400",
                        IconButton { label: "Go to definition", icon: IconName::Definition, disabled: definition.is_none(),
                            onclick: move |_| { if let Some(location) = definition.clone() { on_navigate.call(location); } }
                        }
                        IconButton { label: if show_references() { "Hide references" } else { "Show references" }, icon: IconName::References, active: show_references(), expanded: show_references(), controls: "studio-writer-references", disabled: reference_positions.is_empty() && !show_references(), onclick: move |_| show_references.toggle() }
                        IconButton { label: "Format story", icon: IconName::Format, onclick: move |_| {
                            let catalog = AssetCatalog::from_registry(&app_state.read().registry);
                            let analysis = analyze(app_state.read().story.source(), &catalog.assets);
                            let result = storyfella_language::format(&analysis).and_then(|edits| storyfella_language::apply_edits(analysis.document.source.text(), &edits));
                            match result { Ok(source) => { app_state.write().set_story_source(source); edit_error.set(None); }, Err(error) => edit_error.set(Some(error)) }
                        } }
                        IconButton { label: "Rename symbol", icon: IconName::Edit, disabled: rename_symbol.is_none(), onclick: move |_| {
                            if let Some(name) = rename_symbol.clone() { rename_target.set(Some((offset, name))); }
                        } }
                        super::atelier::OpenInAtelier { target: atelier_target, show_label: false }
                        div { class: "ml-auto flex flex-wrap items-center gap-3",
                            div { role: "status",
                                IconButton { label: problem_label, icon: IconName::List, show_label: true,
                                    tooltip: if show_problems() { "Hide story problems" } else { "Show story problems" },
                                    expanded: show_problems(), controls: "studio-writer-problems",
                                    disabled: problems.is_empty() && !show_problems(),
                                    onclick: move |_| show_problems.toggle()
                                }
                            }
                            p { class: "font-mono text-[10px] text-zinc-500", "Line {at + 1} of {lines.len()}" }
                        }
                    }
                    if let Some(error) = edit_error() { p { class: "px-4 py-1 text-[11px] text-rose-300", "{error}" } }
                    div { id: "studio-writer-references", role: "region", aria_label: "Story references", hidden: !show_references(), class: "max-h-24 overflow-auto px-4 py-1",
                        ReferenceButtons { positions: reference_positions, on_navigate }
                    }
                    div { id: "studio-writer-problems", role: "region", aria_label: "Story problems", hidden: !show_problems(), class: "shrink-0 border-t border-white/[0.05]",
                        div { class: "px-4 py-2",
                            select { aria_label: "Filter story problems", oninput: move |event| problem_filter.set(event.value()),
                                option { value: "all", selected: problem_filter() == "all", "All problems" }
                                option { value: "errors", selected: problem_filter() == "errors", "Errors" }
                                option { value: "warnings", selected: problem_filter() == "warnings", "Warnings" }
                            }
                        }
                        div { class: "max-h-40 overflow-y-auto flex flex-col pb-2",
                            for (file, diagnostic, position) in diagnostics.iter().copied() {
                                {
                                    let position = *position;
                                    let other_file = !file.is_empty() && file != entry;
                                    rsx! { button { class: "flex items-start gap-2 px-4 py-2 text-left text-xs text-zinc-300 hover:bg-white/[0.04]",
                                        title: "Open problem: {diagnostic.message}",
                                        onclick: { let file = file.clone(); move |_| {
                                            on_navigate.call((file.clone(), position));
                                        } },
                                        span { class: if diagnostic.severity == Severity::Error { "mt-1 size-1.5 shrink-0 rounded-full bg-rose-400" } else { "mt-1 size-1.5 shrink-0 rounded-full bg-amber-400" } }
                                        span { class: "min-w-0 flex-1 break-words", "{diagnostic.message}" }
                                        span { class: "shrink-0 font-mono text-[10px] text-zinc-500",
                                            if other_file { "{file} · " }
                                            "{diagnostic.code} · {position.line + 1}:{position.character + 1}"
                                        }
                                    } }
                                }
                            }
                        }
                        if diagnostics.is_empty() {
                            p { role: "status", class: "px-4 pb-3 text-xs text-zinc-400", "No problems in this filter." }
                        }
                        if analysis.has_errors() {
                            p { class: "px-4 py-1 text-[11px] text-amber-300", "Story artwork keeps the last valid scene and beat association while this draft has errors." }
                        }
                    }
                }
            }

            if let Some((offset, name)) = rename_target() {
                RenameSymbolDialog { app_state, offset, name, onclose: move |_| rename_target.set(None) }
            }

            // ------------------------------------------------------- panel
            div { hidden: !writing_help(), class: "studio-writing-rail",
                super::writing_assistant::WritingAssistant { app_state, backend, open: writing_help, novel_proposal, onclose: move |_| { writing_help.set(false); document::eval("document.getElementById('svs-writer')?.focus();"); } }
            }
            aside { hidden: writing_help(), class: "flex w-[320px] shrink-0 flex-col bg-zinc-950/35",
                div { class: "flex items-center gap-2 border-b border-white/[0.05] px-4 py-2.5",
                    p { class: "text-[10px] font-semibold uppercase tracking-[0.16em] text-zinc-600",
                        "In scope"
                    }
                    if pending > 0 {
                        span { class: "ml-auto text-[10px] text-amber-400", "{pending} pending" }
                    }
                }
                div { class: "min-h-0 flex-1 space-y-5 overflow-y-auto p-3",
                    if !in_scope_artwork.is_empty() {
                        div { class: "space-y-1",
                            for (name, id, kind) in in_scope_artwork {
                                button {
                                    class: "flex min-h-10 w-full items-center gap-3 rounded-lg border border-white/[0.06] px-3 py-2 text-left transition hover:bg-white/[0.05]",
                                    title: "Open {name} in Assets",
                                    onclick: move |_| { app_state.write().select_artifact(id); screen.set(super::AppScreen::Assets); },
                                    super::icons::AssetIcon { kind, class: "size-4 shrink-0 text-zinc-400" }
                                    span { class: "text-xs capitalize text-zinc-400", "{kind}" }
                                    span { class: "min-w-0 truncate text-sm font-medium text-zinc-200", "{name}" }
                                }
                            }
                        }
                    }
                    if beat_mentions.is_empty() && scene_mentions.is_empty() {
                        p { class: "px-1 text-[11px] italic text-zinc-700",
                            "Nothing referenced here yet."
                        }
                    }
                    if !beat_mentions.is_empty() {
                        MentionGroup {
                            title: "This beat".to_owned(),
                            mentions: beat_mentions,
                            app_state,
                        }
                    }
                    if !scene_mentions.is_empty() {
                        MentionGroup {
                            title: "Elsewhere in the scene".to_owned(),
                            mentions: scene_mentions,
                            app_state,
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn MentionGroup(title: String, mentions: Vec<Mention>, app_state: Signal<AppState>) -> Element {
    let mut open_asset = use_context::<WriterAssetView>().0;
    rsx! {
        section {
            div { class: "mb-2 flex items-center gap-2 px-1",
                p { class: "text-[10px] font-semibold uppercase tracking-[0.07em] text-zinc-600",
                    "{title}"
                }
                span { class: "text-[10px] text-zinc-700", "{mentions.len()}" }
            }
            div { class: "space-y-1.5",
                for mention in mentions {
                    div {
                        key: "{mention.name}",
                        class: if mention.pending() { "flex items-center gap-2.5 rounded-[10px] border border-dashed border-amber-400/30 bg-white/[0.02] p-2" } else { "flex items-center gap-2.5 rounded-[10px] border border-white/[0.06] bg-white/[0.025] p-2" },

                        div { class: "relative h-[52px] w-10 shrink-0 overflow-hidden rounded-md",
                            if let Some(url) = mention.image.clone() {
                                super::artwork::ArtworkImage {
                                    class: "size-full object-contain ring-1 ring-inset ring-white/[0.12]",
                                    src: url, alt: mention.name.clone(),
                                }
                            } else if mention.pending() {
                                div { class: "grid size-full place-items-center rounded-md border border-dashed border-amber-400/35 text-lg font-light text-amber-400/60",
                                    "+"
                                }
                            } else {
                                div { class: "grid size-full place-items-center rounded-md bg-white/[0.05] text-zinc-700 ring-1 ring-inset ring-white/[0.07]",
                                    if let Some(kind) = mention.kind {
                                        super::icons::AssetIcon { kind, class: "size-4" }
                                    }
                                }
                            }
                        }

                        div { class: "min-w-0 flex-1",
                            p {
                                class: if mention.pending() { "truncate font-mono text-[12px] font-semibold text-amber-400" } else { "truncate font-mono text-[12px] font-semibold text-zinc-200" },
                                "{mention.name}"
                            }
                            p { class: "mt-0.5 truncate text-[10px] text-zinc-600",
                                if mention.pending() {
                                    "not created"
                                } else if let Some(kind) = mention.kind {
                                    "{kind}"
                                }
                            }
                        }

                        if mention.can_create {
                            button {
                                class: "shrink-0 rounded-md bg-amber-400/15 px-2 py-1 text-[10px] font-semibold text-amber-300 transition hover:bg-amber-400/25",
                                title: "Create a catalog entry without generating an image",
                                onclick: {
                                    let name = mention.name.clone();
                                    let kind = match mention.kind {
                                        Some(ArtifactKind::Environment) => AssetKind::Location,
                                        Some(ArtifactKind::Object) => AssetKind::Prop,
                                        Some(ArtifactKind::Illustration) => AssetKind::Illustration,
                                        _ => AssetKind::Character,
                                    };
                                    move |_| { app_state.write().create_pending_asset(&name, kind); }
                                },
                                "Create"
                            }
                        } else if !mention.pending() {
                            super::atelier::OpenInAtelier { target: mention.artifact, show_label: false }
                            button {
                                class: "shrink-0 rounded-md px-2 py-1 text-[10px] text-zinc-600 transition hover:text-zinc-200",
                                title: "Open asset",
                                onclick: {
                                    let id = mention.artifact;
                                    move |_| {
                                        if let Some(id) = id {
                                            app_state.write().select_artifact(id);
                                            open_asset.set(true);
                                        }
                                    }
                                },
                                "Open"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyfella_studio_core::registry::CreateArtifact;

    #[test]
    fn explorer_uses_native_rename_keys() {
        assert!(explorer_rename_key(&Key::Enter, true));
        assert!(!explorer_rename_key(&Key::F2, true));
        assert!(explorer_rename_key(&Key::F2, false));
        assert!(!explorer_rename_key(&Key::Enter, false));
        assert!(!explorer_rename_key(&Key::Character(" ".into()), true));
    }

    #[test]
    fn explorer_groups_nested_sources_and_hides_collapsed_descendants() {
        let collapsed = std::collections::BTreeSet::from(["chapters/part".to_owned()]);
        assert!(ancestors_visible("chapters/part", &collapsed));
        assert!(!ancestors_visible("chapters/part/two.sf", &collapsed));
        assert!(ancestors_visible("chapters/one.sf", &collapsed));
    }

    #[test]
    fn project_problems_follow_unsaved_chapter_locations() {
        let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("writer-project-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let entry = directory.join("story.sf");
        std::fs::write(&entry, "include \"chapter.sf\"\n").unwrap();
        std::fs::write(directory.join("chapter.sf"), "scene start\n  end\n").unwrap();
        let mut state = AppState::default();
        state.story = storyfella_studio_core::story::StoryDocument::open(&entry).unwrap();
        state.open_additional_source("chapter.sf").unwrap();
        state
            .set_additional_source("chapter.sf", "scene start\n  goto missing\n".into())
            .unwrap();
        let problems = project_diagnostics(&state);
        assert!(problems
            .iter()
            .any(|(path, diagnostic, position)| path == "chapter.sf"
                && diagnostic.code == "SF202"
                && position.line == 1));
        assert!(!problems
            .iter()
            .any(|(path, diagnostic, _)| path == "story.sf"
                && diagnostic.severity == Severity::Error));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn completion_resolves_scenes_in_other_files_and_maps_the_edit_back() {
        let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("writer-completion-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let entry = directory.join("story.sf");
        std::fs::write(&entry, "include \"chapter.sf\"\nscene lighthouse\n  end\n").unwrap();
        let chapter = "scene start\n  goto li";
        std::fs::write(directory.join("chapter.sf"), chapter).unwrap();
        let mut state = AppState::default();
        state.story = storyfella_studio_core::story::StoryDocument::open(&entry).unwrap();
        state.open_additional_source("chapter.sf").unwrap();
        let completions = workspace_completions(&state, "chapter.sf", chapter.len()).unwrap();
        let result = completions
            .iter()
            .find(|completion| completion.label == "lighthouse")
            .unwrap();
        assert_eq!(&chapter[result.replace.start..result.replace.end], "li");
        let chapter = "scene start\n  goto lighthouse\n";
        state
            .set_additional_source("chapter.sf", chapter.into())
            .unwrap();
        let catalog = AssetCatalog::from_registry(&state.registry);
        let navigation = symbol_navigation(
            &state,
            "chapter.sf",
            &analyze(chapter, &catalog.assets),
            chapter.find("lighthouse").unwrap(),
            &catalog,
        );
        assert_eq!(
            navigation.definition,
            Some((
                "story.sf".into(),
                storyfella_language::Position {
                    line: 1,
                    character: 6
                }
            ))
        );
        assert_eq!(navigation.references.len(), 2);
        assert_eq!(navigation.rename.as_deref(), Some("lighthouse"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn sample_is_valid_storyfella() {
        let analysis = analyze(crate::app::sample::STORY, &[]);
        assert!(!analysis.has_errors(), "{:?}", analysis.diagnostics);
    }

    #[test]
    fn branches_are_not_presented_as_missing_artwork() {
        let source = "scene harbor\n  goto lighthouse\nscene lighthouse\n  end\n";
        let (beat, scene) = mentions_at(source, 1, &ArtifactRegistry::new());
        assert!(beat.is_empty() && scene.is_empty());
    }

    const STORY: &str = "scene harbor\n  cast: mara, tam\n\n  beat arrival\n    mara: \"You're late.\"\n\n  beat handoff\n    prop: brass_lantern\n\nscene lighthouse\n  location: lighthouse_tower\n";

    #[test]
    fn the_outline_tracks_scene_and_beat() {
        let scopes = outline(STORY);
        assert_eq!(scopes[0].scene.as_deref(), Some("harbor"));
        assert_eq!(scopes[0].beat, None);
        assert_eq!(scopes[4].beat.as_deref(), Some("arrival"));
        assert_eq!(scopes[7].beat.as_deref(), Some("handoff"));
        // A new scene clears the beat rather than carrying it across.
        assert_eq!(scopes[9].scene.as_deref(), Some("lighthouse"));
        assert_eq!(scopes[9].beat, None);
    }

    #[test]
    fn references_come_from_slots_never_from_prose() {
        assert_eq!(names_in("  cast: mara, tam"), ["mara", "tam"]);
        assert_eq!(names_in("    prop: brass_lantern"), ["brass_lantern"]);
        assert_eq!(names_in("    mara: \"You're late.\""), ["mara"]);
        assert_eq!(names_in("    tam think: she counts"), ["tam"]);
        // Prose slots hold no references, and dialogue text is never scanned.
        assert!(names_in("    shot: wide, dawn").is_empty());
        assert!(names_in("# mara is mentioned here").is_empty());
        assert!(names_in("").is_empty());
    }

    #[test]
    fn the_panel_splits_this_beat_from_the_rest_of_the_scene() {
        let registry = ArtifactRegistry::new();
        // Line 4 is mara's dialogue, inside `beat arrival`.
        let (beat, scene) = mentions_at(STORY, 4, &registry);
        let beat: Vec<&str> = beat.iter().map(|m| m.name.as_str()).collect();
        let scene: Vec<&str> = scene.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(beat, ["mara"]);
        // `cast:` sits on the scene, and the other beat's prop is elsewhere.
        assert!(scene.contains(&"tam"));
        assert!(scene.contains(&"brass_lantern"));
        assert!(!scene.contains(&"mara"), "the cursor's beat wins the name");
    }

    #[test]
    fn a_different_scene_is_out_of_scope() {
        let registry = ArtifactRegistry::new();
        let (beat, scene) = mentions_at(STORY, 10, &registry);
        let names: Vec<&str> = beat
            .iter()
            .chain(scene.iter())
            .map(|m| m.name.as_str())
            .collect();
        assert_eq!(names, ["lighthouse_tower"]);
    }

    #[test]
    fn mentions_resolve_against_the_registry() {
        let mut registry = ArtifactRegistry::new();
        registry
            .create_artifact(
                ArtifactKind::Character,
                CreateArtifact {
                    name: "mara".into(),
                    description: "a harbourmaster".into(),
                    ..Default::default()
                },
            )
            .expect("artifact should create");

        let (beat, _) = mentions_at(STORY, 4, &registry);
        assert!(!beat[0].pending(), "mara exists and must resolve");

        let (_, scene) = mentions_at(STORY, 4, &registry);
        let tam = scene.iter().find(|m| m.name == "tam").unwrap();
        assert!(tam.pending(), "tam has no artifact yet");
    }

    #[test]
    fn highlighting_preserves_the_line() {
        for line in [
            "scene harbor",
            "  cast: mara, tam",
            "    mara: \"You're late. Tide's turning.\"",
            "# a comment",
            "",
        ] {
            let rebuilt: String = highlight(line, &["mara".to_owned()])
                .iter()
                .map(|piece| piece.text.as_str())
                .collect();
            assert_eq!(rebuilt, line, "highlighting lost text");
        }
    }

    #[test]
    fn unresolved_references_are_painted_as_pending_not_as_errors() {
        let pieces = highlight("  cast: mara, tam", &["mara".to_owned()]);
        let mara = pieces.iter().find(|p| p.text.starts_with("mara")).unwrap();
        let tam = pieces.iter().find(|p| p.text == "tam").unwrap();
        assert!(mara.class.contains("sky"), "resolved reads as resolved");
        assert!(tam.class.contains("amber"), "pending is amber, never red");
    }
}

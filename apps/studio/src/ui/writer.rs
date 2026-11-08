//! The Writer — the story as text, with the artifacts it names beside it.
//!
//! Left: the `.sf` source, highlighted. Right: the artifacts the cursor's
//! scene references, resolved ones with their picture and pending ones as
//! placeholders that offer to create them. Writing the story is what fills
//! the asset catalog with work.
//!
//! Uses the same tokens, AST and reference analysis as the external LSP.
//! Unresolved assets remain pending, with a Create affordance.

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

fn project_diagnostics(
    state: &AppState,
) -> Result<
    Vec<(
        String,
        storyfella_language::Diagnostic,
        storyfella_language::Position,
    )>,
    String,
> {
    let sources = super::preview::workspace(state)?;
    let catalog = AssetCatalog::from_registry(&state.registry);
    let (compilation, expanded) = storyfella_compiler::compile_workspace_with_catalog(
        &sources,
        &catalog.assets,
        &catalog.bindings,
    )
    .map_err(|error| format!("{}: {}", error.path, error.message))?;
    Ok(compilation
        .diagnostics
        .into_iter()
        .filter_map(|mut diagnostic| {
            let (path, span) = expanded.original_span(diagnostic.span)?;
            let position =
                storyfella_language::Source::new(&sources.files[path]).position(span.start)?;
            diagnostic.span = span;
            Some((path.to_owned(), diagnostic, position))
        })
        .collect())
}

fn workspace_symbol(state: &AppState, file: &str, offset: usize) -> Result<String, String> {
    let sources = super::preview::workspace(state)?;
    let expanded = sources.expand().map_err(|error| error.message)?;
    let offset = expanded
        .expanded_offset(file, offset)
        .ok_or("Select a scene or beat in an included file")?;
    let catalog = AssetCatalog::from_registry(&state.registry);
    let analysis = analyze(&expanded.source, &catalog.assets);
    let definition = analysis
        .definition_at(offset)
        .ok_or("Select a scene or beat name")?;
    analysis
        .symbols
        .iter()
        .find(|symbol| symbol.name.span == definition)
        .map(|symbol| symbol.name.value.clone())
        .ok_or_else(|| "Select a scene or beat name".into())
}

/// The starter document, so the screen explains itself rather than opening
/// on an empty box.
pub const STARTER: &str = r#"# Write the story; Assets lists the artwork it needs.

story "Untitled"

scene harbor
  location: harbor_dock
  cast: mara, tam

  beat arrival
    shot: wide, dawn
    mara: "You're late. Tide's already turning."
    tam think: She counts the minutes like coins.
"#;

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
        for (const name of ["input", "keyup", "click", "focus", "select"]) {
            el.addEventListener(name, push);
        }
    }
"#;

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
        state.write().set_story_source(edited);
        caret.navigate(position);
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
                    r.name.value == name && here.scene.as_deref() == Some(r.scene.as_str())
                });
                let artifact = reference
                    .and_then(|r| r.asset_id.as_ref())
                    .and_then(|id| registry.artifacts.iter().find(|a| a.id.to_string() == *id));
                let expected_kind = reference.and_then(|r| match r.kind {
                    ReferenceKind::Asset(AssetKind::Character) => Some(ArtifactKind::Character),
                    ReferenceKind::Asset(AssetKind::Location) => Some(ArtifactKind::Environment),
                    ReferenceKind::Asset(AssetKind::Prop) => Some(ArtifactKind::Object),
                    ReferenceKind::Scene => None,
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
                "rounded-sm bg-violet-400/15 text-violet-200 shadow-[0_0_0_2px_rgba(167,139,250,.15)]"
            } else {
                "rounded-sm bg-amber-400/15 text-amber-300 shadow-[0_0_0_2px_rgba(251,191,36,.15)]"
            }
        } else {
            match token.kind {
                TokenKind::Keyword => "text-violet-300 font-semibold",
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
        super::dialog::Dialog { title: "Rename scene or beat", on_close: onclose,
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
                    button { r#type: "button", class: "studio-button", onclick: move |_| onclose.call(()), "Cancel" }
                    button { r#type: "submit", class: "studio-button studio-button-primary", disabled: replacement().trim().is_empty(), "Rename" }
                }
            }
        }
    }
}

#[component]
pub fn Writer(
    mut app_state: Signal<AppState>,
    backend: Signal<storyfella_studio_core::registry::backend::CreativeBackend>,
) -> Element {
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
    let mut screen = use_context::<Signal<super::AppScreen>>();
    let mut files = use_signal(Vec::<String>::new);
    let mut scan_error = use_signal(|| None::<String>);
    let mut refresh = use_signal(|| 0u64);
    let path = app_state
        .read()
        .story
        .path()
        .map(std::path::Path::to_path_buf);
    let revision = refresh();
    use_effect(use_reactive(
        (&path, &revision),
        move |(path, _)| match path
            .map(|path| storyfella_studio_core::story::list_source_files(&path))
        {
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
    let caret = use_context::<super::WriterCaret>();
    let project_problems = if app_state
        .read()
        .story
        .source()
        .lines()
        .any(|line| line.starts_with("include "))
    {
        Some(project_diagnostics(&app_state.read()))
    } else {
        None
    };
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
    rsx! {
        div { class: "flex h-full min-h-0 min-w-0",
            aside { class: "flex w-52 shrink-0 flex-col overflow-y-auto border-r border-white/10 p-3", aria_label: "Project explorer",
                if let Some(notice) = app_state.read().source_notice.clone() {
                    div { class: "mb-3 text-xs text-amber-300", role: "status", "{notice}"
                        button { class: "ml-1 underline", onclick: move |_| app_state.write().source_notice = None, "Dismiss" }
                    }
                }
                div { class: "flex items-center justify-between gap-2",
                    h2 { class: "text-xs font-semibold text-zinc-400", "Story files" }
                    IconButton { label: "Create story file", icon: IconName::Add, onclick: move |_| {
                        let root = app_state.read().story.path().and_then(|path| path.parent()).map(std::path::Path::to_path_buf);
                        let Some(root) = root else { return; };
                        let Some(mut destination) = rfd::FileDialog::new().set_title("Create story file").set_directory(root).set_file_name("chapter.sf").add_filter("Storyfella source", &["sf"]).save_file() else { return; };
                        if destination.extension().is_none() { destination.set_extension("sf"); }
                        if let Err(error) = app_state.write().create_additional_source(&destination) { error_report.set(Some(error.report().log())); }
                        else { refresh += 1; }
                    } }
                    IconButton { label: "Refresh project files", icon: IconName::Refresh, onclick: move |_| refresh += 1 }
                }
                if let Some(error) = scan_error() { p { class: "text-xs text-red-400", role: "alert", "{error}" } }
                for file in files() {
                    {
                        let is_entry = file == entry;
                        let selected = if is_entry { active.is_none() } else { active.as_deref() == Some(file.as_str()) };
                        let label = if is_entry { format!("{file} · entry") } else if included.as_ref().is_some_and(|sources| !sources.contains(&file)) { format!("{file} · not included") } else { file.clone() };
                        let target = file.clone();
                        rsx! { button { key: "{file}", class: if selected { "rounded bg-white/10 px-2 py-1 text-left text-xs text-zinc-100" } else { "rounded px-2 py-1 text-left text-xs text-zinc-400 hover:bg-white/5" },
                            title: "{file}", onclick: move |_| {
                                if is_entry { app_state.write().active_source = None; }
                                else if let Err(error) = app_state.write().open_additional_source(&target) { error_report.set(Some(error.report().log())); }
                            }, "{label}"
                        } }
                    }
                }
                if project_problems.is_some() {
                    h2 { class: "mt-5 text-xs font-semibold text-zinc-400", "Story problems" }
                    if let Some(Ok(problems)) = project_problems.clone() {
                        for (path, diagnostic, position) in problems.into_iter().filter(|(_, diagnostic, _)| diagnostic.severity != Severity::Information) {
                            {
                                let is_entry = path == entry;
                                let target = path.clone();
                                rsx! { button { class: "rounded px-2 py-1 text-left text-xs text-zinc-400 hover:bg-white/5", title: "{diagnostic.message}", onclick: move |_| {
                                    if is_entry { app_state.write().active_source = None; }
                                    else if let Err(error) = app_state.write().open_additional_source(&target) { error_report.set(Some(error.report().log())); return; }
                                    caret.navigate(position);
                                }, "{path}:{position.line + 1} · {diagnostic.message}" } }
                            }
                        }
                    }
                    if let Some(Err(message)) = project_problems {
                        p { class: "px-2 py-1 text-xs text-rose-300", role: "alert", "{message}" }
                    }
                }
                h2 { class: "mt-5 text-xs font-semibold text-zinc-400", "Assets" }
                for (id, name) in assets {
                    button { key: "{id}", class: "rounded px-2 py-1 text-left text-xs text-zinc-400 hover:bg-white/5", onclick: move |_| { app_state.write().select_artifact(id); screen.set(super::AppScreen::Assets); }, "{name}" }
                }
            }
            div { class: "min-w-0 flex-1",
                div { hidden: active.is_some(), class: "h-full", MainWriter { app_state, backend } }
                if let Some(path) = active.clone() { AdditionalWriter { key: "{path}", path, app_state } }
            }
        }
    }
}

#[component]
fn AdditionalWriter(path: String, mut app_state: Signal<AppState>) -> Element {
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
    let mut navigation = use_context::<super::WriterCaret>().target;
    let mut mounted = use_signal(|| false);
    let mut rename_target = use_signal(|| None::<(usize, String)>);
    let source = app_state
        .read()
        .additional_sources
        .as_ref()
        .and_then(|sources| sources.get(&path))
        .map(|document| document.source().to_owned())
        .unwrap_or_default();
    let mut previous_source = use_signal(|| source.clone());
    use_effect(use_reactive((&source,), move |(source,)| {
        let previous = previous_source.peek().replace("\r\n", "\n");
        previous_source.set(source.clone());
        let next = source.replace("\r\n", "\n");
        document::eval(&format!("const el = document.getElementById('svs-extra-writer'); if (el && el.value === {}) el.value = {};", serde_json::to_string(&previous).unwrap(), serde_json::to_string(&next).unwrap()));
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
                IconButton { label: "Rename scene or beat at cursor", icon: IconName::Edit, onclick: { let path = path.clone(); move |_| {
                    let path = path.clone();
                    spawn(async move {
                        let mut query = document::eval(r#"const el = document.getElementById('svs-extra-writer'); const lines = el.value.slice(0, el.selectionStart ?? 0).split('\n'); dioxus.send({line: lines.length - 1, character: lines.at(-1).length});"#);
                        let Ok(position) = query.recv::<storyfella_language::Position>().await else { return; };
                        let state = app_state.read();
                        if state.active_source.as_deref() != Some(path.as_str()) { return; }
                        let Some(source) = state.additional_sources.as_ref().and_then(|sources| sources.get(&path)).map(|document| document.source()) else { return; };
                        let Some(offset) = storyfella_language::Source::new(source).offset(position) else { return; };
                        let result = workspace_symbol(&state, &path, offset);
                        drop(state);
                        match result { Ok(name) => rename_target.set(Some((offset, name))), Err(message) => error_report.set(Some(ReportedError::plain(storyfella_studio_core::errors::ErrorCode::AppCommandInvalid, message))) }
                    });
                } } }
            }
            textarea { id: "svs-extra-writer", aria_label: "Story source", spellcheck: "false", autocomplete: "off",
                class: "min-h-0 w-full flex-1 resize-none bg-transparent px-4 py-3 font-mono text-sm leading-relaxed text-zinc-100 outline-none",
                onmounted: { let initial = source.clone(); move |_| { document::eval(&format!("document.getElementById('svs-extra-writer').value = {};", serde_json::to_string(&initial.replace("\r\n", "\n")).unwrap())); mounted.set(true); } },
                oninput: move |event| {
                    let mut state = app_state.write();
                    if let Some(document) = state.additional_sources.as_ref().and_then(|sources| sources.get(&target)) {
                        let edited = storyfella_language::textarea_input(document.source(), &event.value());
                        if let Err(error) = state.set_additional_source(&target, edited) { error_report.set(Some(error.report().log())); }
                    }
                },
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
    backend: Signal<storyfella_studio_core::registry::backend::CreativeBackend>,
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
    let mut writing_help = use_signal(|| false);
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
    let mut compilation = storyfella_compiler::compile_analysis(&analysis, &catalog.bindings);
    if source.lines().any(|line| line.starts_with("include ")) {
        let entry = story_path
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        compilation.diagnostics = match project_diagnostics(&app_state.read()) {
            Ok(problems) => problems
                .into_iter()
                .filter_map(|(path, diagnostic, _)| (path == entry).then_some(diagnostic))
                .collect(),
            Err(message) => vec![storyfella_language::Diagnostic {
                code: "SF400".into(),
                message,
                severity: Severity::Error,
                span: storyfella_language::Span::new(0, 0),
            }],
        };
    }
    let error_count = compilation
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    let warning_count = compilation.diagnostics.len() - error_count;
    let problem_label = format!(
        "Problems · {error_count} {} · {warning_count} {}",
        if error_count == 1 { "error" } else { "errors" },
        if warning_count == 1 {
            "warning"
        } else {
            "warnings"
        }
    );
    let diagnostics: Vec<_> = compilation
        .diagnostics
        .iter()
        .filter(|d| match problem_filter().as_str() {
            "errors" => d.severity == Severity::Error,
            "warnings" => d.severity != Severity::Error,
            _ => true,
        })
        .collect();
    let offset = analysis.document.source.offset(cursor()).unwrap_or(0);
    let completions = if show_completions() {
        storyfella_language::complete(&analysis, offset, &catalog.assets)
    } else {
        Vec::new()
    };
    let selected_completion = completion_index().min(completions.len().saturating_sub(1));
    let key_completions = completions.clone();
    let definition = analysis
        .definition_at(offset)
        .and_then(|span| analysis.document.source.position(span.start));
    let rename_symbol = analysis
        .definition_at(offset)
        .and_then(|span| {
            analysis
                .symbols
                .iter()
                .find(|symbol| symbol.name.span == span)
        })
        .map(|symbol| symbol.name.value.clone());
    let rename_symbol = if source.lines().any(|line| line.starts_with("include ")) {
        let entry = story_path
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        workspace_symbol(&app_state.read(), entry, offset).ok()
    } else {
        rename_symbol
    };
    let reference_positions: Vec<_> = analysis
        .references_at(offset, true)
        .iter()
        .filter_map(|span| analysis.document.source.position(span.start))
        .collect();

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
            serde_json::to_string(&previous).unwrap(), serde_json::to_string(&next).unwrap()
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
                    IconButton { label: "Writing help", icon: IconName::Sparkles, active: writing_help(), onclick: move |_| writing_help.toggle() }
                }

                if reviewing {
                  div { key: "{review_controller.review.read().as_ref().unwrap().proposal.run_id}", class: "flex min-h-0 flex-1 flex-col",
                    div { class: "studio-review-toolbar", aria_label: "Review view",
                        p { class: "mr-auto text-xs text-zinc-400", if remaining == 1 { "1 change remaining" } else { "{remaining} changes remaining" } }
                        button { class: if review_mode() == super::writing_review::ReviewMode::Inline { "is-selected" } else { "" }, title: "Review changes in one continuous editor", aria_pressed: review_mode() == super::writing_review::ReviewMode::Inline, onclick: move |_| review_mode.set(super::writing_review::ReviewMode::Inline), "Inline" }
                        button { class: if review_mode() == super::writing_review::ReviewMode::SideBySide { "is-selected" } else { "" }, title: "Compare the draft and suggestion side by side", aria_pressed: review_mode() == super::writing_review::ReviewMode::SideBySide, onclick: move |_| review_mode.set(super::writing_review::ReviewMode::SideBySide), "Side by side" }
                    }
                    super::writing_review::WritingReviewEditor { app_state }
                  }
                }
                div { hidden: reviewing, class: "flex min-h-0 flex-1 flex-col",
                if source.is_empty() {
                    div { class: "grid flex-1 place-items-center px-6 text-center",
                        div { class: "grid size-12 place-items-center rounded-2xl bg-white/[0.045] text-zinc-600 ring-1 ring-inset ring-white/[0.06]",
                            Icon { name: IconName::List, class: "size-5" }
                        }
                        p { class: "mt-4 text-[13px] font-semibold text-zinc-200", "No story yet" }
                        p { class: "mt-1 max-w-sm text-[11px] leading-5 text-zinc-500",
                            "Write scenes and beats here; the artifacts they name appear on the right, ready to create."
                        }
                        button {
                            class: "mt-4 rounded-lg bg-violet-400/15 px-3 py-1.5 text-[11px] font-semibold text-violet-200 transition hover:bg-violet-400/25",
                            onclick: move |_| {
                                let mut state = app_state.write();
                                state.set_story_source(STARTER.to_owned());
                            },
                            "Start a story"
                        }
                    }
                } else {
                    // The rendered layer is the visible text; the textarea
                    // sits over it transparently, exactly as the command bar
                    // does, so highlighting and the caret share metrics.
                    div { class: "relative min-h-0 flex-1 overflow-auto",
                        div { aria_hidden: "true", class: "pointer-events-none absolute inset-0 z-10 px-4 py-3 font-mono text-[12.5px] leading-[1.75] whitespace-pre-wrap",
                            for (index , line) in lines.iter().enumerate() {
                                div {
                                    key: "{index}",
                                    id: "svs-writer-line-{index}",
                                    class: if index == at { "-mx-4 border-l-2 border-violet-400 bg-violet-400/[0.07] pl-[14px] pr-4" } else { "-mx-4 border-l-2 border-transparent pl-[14px] pr-4" },
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
                            id: "svs-writer",
                            class: "absolute inset-x-0 top-0 z-20 min-h-full w-full resize-none overflow-hidden bg-transparent px-4 py-3 font-mono text-[12.5px] leading-[1.75] text-transparent caret-violet-400 outline-none",
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
                        div { class: "max-h-32 overflow-y-auto border-t border-white/[0.05] px-4 py-2",
                            for (index, completion) in completions.iter().enumerate() {
                                button {
                                    class: if index == selected_completion { "mr-2 rounded bg-violet-400/15 px-2 py-1 font-mono text-[11px] text-violet-200" } else { "mr-2 px-2 py-1 font-mono text-[11px] text-zinc-400" },
                                    onclick: { let completion = completion.clone(); move |_| { accept_completion(app_state, caret, &completion); show_completions.set(false); } },
                                    "{completion.label}"
                                }
                            }
                        }
                    }
                    div { class: "flex flex-wrap items-center gap-2 border-t border-white/[0.05] px-4 py-2 text-[11px] text-zinc-400",
                        IconButton { label: "Go to definition", icon: IconName::Definition, disabled: definition.is_none(), onclick: move |_| { if let Some(position) = definition { cursor.set(position); focus_position(position); } } }
                        IconButton { label: if show_references() { "Hide references" } else { "Show references" }, icon: IconName::References, active: show_references(), expanded: show_references(), controls: "studio-writer-references", disabled: reference_positions.is_empty() && !show_references(), onclick: move |_| show_references.toggle() }
                        IconButton { label: "Format story", icon: IconName::Format, onclick: move |_| {
                            let catalog = AssetCatalog::from_registry(&app_state.read().registry);
                            let analysis = analyze(app_state.read().story.source(), &catalog.assets);
                            let result = storyfella_language::format(&analysis).and_then(|edits| storyfella_language::apply_edits(analysis.document.source.text(), &edits));
                            match result { Ok(source) => { app_state.write().set_story_source(source); edit_error.set(None); }, Err(error) => edit_error.set(Some(error)) }
                        } }
                        IconButton { label: "Rename scene or beat", icon: IconName::Edit, disabled: rename_symbol.is_none(), onclick: move |_| {
                            if let Some(name) = rename_symbol.clone() { rename_target.set(Some((offset, name))); }
                        } }
                        super::atelier::OpenInAtelier { target: atelier_target, show_label: false }
                        div { class: "ml-auto flex flex-wrap items-center gap-3",
                            div { role: "status",
                                IconButton { label: problem_label, icon: IconName::List, show_label: true,
                                    tooltip: if show_problems() { "Hide story problems" } else { "Show story problems" },
                                    expanded: show_problems(), controls: "studio-writer-problems",
                                    disabled: compilation.diagnostics.is_empty() && !show_problems(),
                                    onclick: move |_| show_problems.toggle()
                                }
                            }
                            p { class: "font-mono text-[10px] text-zinc-500", "Line {at + 1} of {lines.len()}" }
                        }
                    }
                    if let Some(error) = edit_error() { p { class: "px-4 py-1 text-[11px] text-rose-300", "{error}" } }
                    div { id: "studio-writer-references", role: "region", aria_label: "Story references", hidden: !show_references(), class: "max-h-24 overflow-auto px-4 py-1",
                        for position in reference_positions {
                            button { class: "mr-2 font-mono text-[11px] text-violet-300", onclick: move |_| { cursor.set(position); focus_position(position); }, "{position.line + 1}:{position.character + 1}" }
                        }
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
                            for diagnostic in &diagnostics {
                                if let Some(position) = analysis.document.source.position(diagnostic.span.start) {
                                    button { class: "flex items-start gap-2 px-4 py-2 text-left text-xs text-zinc-300 hover:bg-white/[0.04]",
                                        onclick: move |_| { cursor.set(position); focus_position(position); },
                                        span { class: if diagnostic.severity == Severity::Error { "mt-1 size-1.5 shrink-0 rounded-full bg-rose-400" } else { "mt-1 size-1.5 shrink-0 rounded-full bg-amber-400" } }
                                        span { class: "min-w-0 flex-1 break-words", "{diagnostic.message}" }
                                        span { class: "shrink-0 font-mono text-[10px] text-zinc-500", "{diagnostic.code} · {position.line + 1}:{position.character + 1}" }
                                    }
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
            }

            if let Some((offset, name)) = rename_target() {
                RenameSymbolDialog { app_state, offset, name, onclose: move |_| rename_target.set(None) }
            }

            // ------------------------------------------------------- panel
            div { hidden: !writing_help(), class: "studio-writing-rail",
                super::writing_assistant::WritingAssistant { app_state, backend, open: writing_help, onclose: move |_| { writing_help.set(false); document::eval("document.getElementById('svs-writer')?.focus();"); } }
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
    let mut screen = use_context::<Signal<super::AppScreen>>();
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
                                title: "Open in Assets",
                                onclick: {
                                    let id = mention.artifact;
                                    move |_| {
                                        if let Some(id) = id {
                                            app_state.write().select_artifact(id);
                                            screen.set(super::AppScreen::Assets);
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
        let problems = project_diagnostics(&state).unwrap();
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
    fn starter_and_sample_are_valid_storyfella() {
        for source in [STARTER, crate::app::sample::STORY] {
            let analysis = analyze(source, &[]);
            assert!(!analysis.has_errors(), "{:?}", analysis.diagnostics);
        }
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
        assert!(mara.class.contains("violet"), "resolved reads as resolved");
        assert!(tam.class.contains("amber"), "pending is amber, never red");
    }
}

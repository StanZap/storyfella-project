//! The Writer — the story as text, with the artifacts it names beside it.
//!
//! Left: the `.sf` source, highlighted. Right: the artifacts the cursor's
//! scene references, resolved ones with their picture and pending ones as
//! placeholders that offer to create them. Writing the story is what fills
//! the canvas with work.
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
    generated_asset_url,
    icons::{Icon, IconName},
};

/// The starter document, so the screen explains itself rather than opening
/// on an empty box.
pub const STARTER: &str = r#"# Write the story; the canvas fills with the work it needs.

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
    document::eval(&format!(
        r#"const el = document.getElementById("svs-writer");
        if (el) {{ const lines = el.value.split("\n");
          const line = Math.min({}, lines.length - 1);
          const offset = lines.slice(0, line).reduce((n, s) => n + s.length + 1, 0) + Math.min({}, lines[line].length);
          el.focus(); el.setSelectionRange(offset, offset); }}"#,
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
                    image: artifact.and_then(|a| {
                        registry
                            .latest_image(a.id)
                            .and_then(|r| r.asset_path.as_deref())
                            .and_then(generated_asset_url)
                    }),
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
    analysis.document.tokens.iter().filter(|t| t.span.start >= span.start && t.span.end <= span.end).map(|token| {
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
        Piece { text: source.slice(token.span).unwrap().to_owned(), class }
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
pub fn Writer(app_state: Signal<AppState>) -> Element {
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
    let caret = use_context::<super::WriterCaret>();
    let mut cursor = caret.position;
    let mut navigation = caret.target;
    let mut screen = use_context::<Signal<super::AppScreen>>();
    let mut show_completions = use_signal(|| false);
    let mut completion_index = use_signal(|| 0usize);
    let mut show_references = use_signal(|| false);
    let mut rename_name = use_signal(String::new);
    let (rename_id, rename_mount) = super::text_input::use_text_value(rename_name(), None);
    let mut problem_filter = use_signal(|| "all".to_owned());
    let mut edit_error = use_signal(|| None::<String>);
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
    let compilation = storyfella_compiler::compile_analysis(&analysis, &catalog.bindings);
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
        if let Some(position) = navigation() {
            focus_position(position);
            navigation.set(None);
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
    let in_scope_artwork: Vec<(String, Uuid)> = scene_artwork
        .into_iter()
        .flat_map(|scene| {
            std::iter::once((scene.name.clone(), scene.artifact_id)).chain(
                scene
                    .beats
                    .iter()
                    .filter(|b| Some(&b.name) == here.beat.as_ref())
                    .map(|b| (b.name.clone(), b.artifact_id)),
            )
        })
        .collect();
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
                    button {
                        class: "rounded px-2 py-1 text-[10px] text-zinc-400 hover:bg-white/[0.05] hover:text-white",
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
                        "Save copy…"
                    }
                    if let Some(path) = story_path.as_ref() {
                        p { class: "min-w-0 truncate font-mono text-[10px] text-zinc-500", title: "{path.display()}",
                            "{path.file_name().unwrap_or_default().to_string_lossy()}"
                        }
                        button {
                            class: "rounded px-2 py-1 text-[10px] text-zinc-400 hover:bg-white/[0.05] hover:text-white",
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
                            "Reload .sf"
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
                        button { disabled: definition.is_none(), onclick: move |_| { if let Some(position) = definition { cursor.set(position); focus_position(position); } }, "Definition" }
                        button { onclick: move |_| show_references.toggle(), "References" }
                        button { onclick: move |_| {
                            let catalog = AssetCatalog::from_registry(&app_state.read().registry);
                            let analysis = analyze(app_state.read().story.source(), &catalog.assets);
                            let result = storyfella_language::format(&analysis).and_then(|edits| storyfella_language::apply_edits(analysis.document.source.text(), &edits));
                            match result { Ok(source) => { app_state.write().set_story_source(source); edit_error.set(None); }, Err(error) => edit_error.set(Some(error)) }
                        }, "Format" }
                        input { id: rename_id, onmounted: rename_mount, aria_label: "New symbol name", class: "w-28 rounded bg-white/[0.05] px-2 py-1 font-mono text-zinc-200", placeholder: "New symbol name", oninput: move |event| rename_name.set(event.value()) }
                        button { onclick: move |_| {
                            let offset = storyfella_language::Source::new(app_state.read().story.source()).offset(cursor()).unwrap_or(0);
                            edit_error.set(app_state.write().rename_story_symbol(offset, &rename_name()).err());
                        }, "Rename" }
                    }
                    if let Some(error) = edit_error() { p { class: "px-4 py-1 text-[11px] text-rose-300", "{error}" } }
                    if show_references() {
                        div { class: "max-h-24 overflow-auto px-4 py-1",
                            for position in reference_positions {
                                button { class: "mr-2 font-mono text-[11px] text-violet-300", onclick: move |_| { cursor.set(position); focus_position(position); }, "{position.line + 1}:{position.character + 1}" }
                            }
                        }
                    }
                    div { class: "flex items-center gap-3 px-4 py-1",
                        p { role: "status", class: "text-xs text-zinc-300",
                            "{compilation.diagnostics.iter().filter(|d| d.severity == Severity::Error).count()} errors · {compilation.diagnostics.iter().filter(|d| d.severity != Severity::Error).count()} warnings"
                        }
                        select { aria_label: "Filter story problems", class: "rounded bg-zinc-900 px-2 py-1 text-xs text-zinc-200", oninput: move |event| problem_filter.set(event.value()),
                            option { value: "all", selected: problem_filter() == "all", "All problems" }
                            option { value: "errors", selected: problem_filter() == "errors", "Errors" }
                            option { value: "warnings", selected: problem_filter() == "warnings", "Warnings" }
                        }
                    }
                    div { class: "max-h-40 overflow-y-auto flex flex-col",
                    for diagnostic in compilation.diagnostics.iter().filter(|d| match problem_filter().as_str() {
                        "errors" => d.severity == Severity::Error,
                        "warnings" => d.severity != Severity::Error,
                        _ => true,
                    }) {
                        if let Some(position) = analysis.document.source.position(diagnostic.span.start) {
                            button { class: if diagnostic.severity == Severity::Error { "px-4 py-1 text-left font-mono text-[11px] text-rose-300" } else { "px-4 py-1 text-left font-mono text-[11px] text-amber-300" },
                                onclick: move |_| { cursor.set(position); focus_position(position); },
                                "{position.line + 1}:{position.character + 1} · {diagnostic.code} · {diagnostic.message}"
                            }
                        }
                    }
                    }
                    if let Some(plan) = &compilation.plan {
                        p { class: "px-4 py-1 text-[11px] text-zinc-500", "Playback readiness: {plan.readiness.pending_assets.len()} pending asset uses · {plan.readiness.unsupported_directions.len()} unsupported directions" }
                    }
                    if analysis.has_errors() {
                        p { class: "px-4 py-1 text-[11px] text-amber-300", "Story artwork keeps the last valid scene and beat association while this draft has errors." }
                    }
                    div { class: "flex items-center gap-3 border-t border-white/[0.05] px-4 py-1.5",
                        p { class: "font-mono text-[10px] text-zinc-700",
                            "line {at + 1} of {lines.len()}"
                        }
                        p { class: "ml-auto font-mono text-[10px] text-zinc-800",
                            "the panel follows the caret"
                        }
                    }
                }
            }

            // ------------------------------------------------------- panel
            aside { class: "flex w-[320px] shrink-0 flex-col bg-zinc-950/35",
                div { class: "flex items-center gap-2 border-b border-white/[0.05] px-4 py-2.5",
                    p { class: "text-[10px] font-semibold uppercase tracking-[0.16em] text-zinc-600",
                        "In scope"
                    }
                    if pending > 0 {
                        span { class: "ml-auto text-[10px] text-amber-400", "{pending} pending" }
                    }
                }
                div { class: "min-h-0 flex-1 space-y-5 overflow-y-auto p-3",
                    for (name, id) in in_scope_artwork {
                        button {
                            class: "block rounded px-2 py-1 font-mono text-[11px] text-violet-300 hover:bg-white/[0.05]",
                            onclick: move |_| { app_state.write().select_artifact(id); screen.set(super::AppScreen::Canvas); },
                            "Open {name} artwork"
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
    let composer = use_context::<Signal<String>>();
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
                                img { class: "size-full object-cover ring-1 ring-inset ring-white/[0.12]", src: "{url}", alt: "{mention.name}" }
                            } else if mention.pending() {
                                div { class: "grid size-full place-items-center rounded-md border border-dashed border-amber-400/35 text-lg font-light text-amber-400/60",
                                    "+"
                                }
                            } else {
                                div { class: "grid size-full place-items-center rounded-md bg-white/[0.05] text-zinc-700 ring-1 ring-inset ring-white/[0.07]",
                                    Icon { name: IconName::Layers, class: "size-3.5" }
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
                                title: "Create a catalog entry; render its image on Canvas",
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
                            button {
                                class: "shrink-0 rounded-md bg-amber-400/15 px-2 py-1 text-[10px] font-semibold text-amber-300 transition hover:bg-amber-400/25",
                                title: "Prefill the command bar to create it",
                                onclick: {
                                    let name = mention.name.clone();
                                    let kind = mention.kind.unwrap_or(ArtifactKind::Character).as_str();
                                    move |_| { super::cmdbar::prefill(composer, format!("/create {kind} --name {name} ")); screen.set(super::AppScreen::Canvas); }
                                },
                                "Render…"
                            }
                        } else if !mention.pending() {
                            button {
                                class: "shrink-0 rounded-md px-2 py-1 text-[10px] text-zinc-600 transition hover:text-zinc-200",
                                title: "Open in the canvas",
                                onclick: {
                                    let id = mention.artifact;
                                    move |_| {
                                        if let Some(id) = id {
                                            app_state.write().select_artifact(id);
                                            screen.set(super::AppScreen::Canvas);
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

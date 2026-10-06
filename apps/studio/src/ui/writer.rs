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
use storyfella_language::{
    analyze, Analysis, Asset, AssetKind, ReferenceKind, Severity, TokenKind,
};
use uuid::Uuid;

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

/// Reports which line the caret is on.
///
/// The editor's layers are absolutely positioned, so there is no element
/// whose scroll position could stand in for the caret — and Dioxus form
/// events do not carry `selectionStart`. Same bridge as the command bar,
/// counting newlines instead of bytes.
const CARET_LINE_BRIDGE: &str = r#"
    const el = document.getElementById("svs-writer");
    if (el && !el.dataset.bridged) {
        el.dataset.bridged = "1";
        const push = () => {
            const caret = el.selectionStart ?? 0;
            dioxus.send(el.value.slice(0, caret).split("\n").length - 1);
        };
        for (const name of ["input", "keyup", "click", "focus", "select"]) {
            el.addEventListener(name, push);
        }
        push();
    }
"#;

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

pub fn asset_catalog(registry: &ArtifactRegistry) -> Vec<Asset> {
    registry
        .artifacts
        .iter()
        .filter_map(|artifact| {
            let kind = match artifact.kind {
                ArtifactKind::Character => AssetKind::Character,
                ArtifactKind::Environment => AssetKind::Location,
                ArtifactKind::Object => AssetKind::Prop,
                _ => return None,
            };
            Some(Asset {
                id: artifact.id.to_string(),
                name: artifact.name.clone(),
                kind,
            })
        })
        .collect()
}

/// A name the story references, and whether the registry has it.
#[derive(Clone, Debug, PartialEq)]
pub struct Mention {
    pub name: String,
    pub artifact: Option<Uuid>,
    pub image: Option<String>,
    pub kind: Option<ArtifactKind>,
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
    mentions_in(&analyze(source, &asset_catalog(registry)), line, registry)
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
    let catalog: Vec<Asset> = known
        .iter()
        .map(|name| Asset {
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
    let mut cursor = use_context::<super::WriterCaret>().0;
    use_future(move || async move {
        let mut bridge = document::eval(CARET_LINE_BRIDGE);
        while let Ok(line) = bridge.recv::<usize>().await {
            cursor.set(line);
        }
    });
    let source = app_state.read().story_source.clone();
    let registry = app_state.read().registry.clone();
    let analysis = analyze(&source, &asset_catalog(&registry));

    let lines: Vec<String> = if source.is_empty() {
        Vec::new()
    } else {
        source.lines().map(str::to_owned).collect()
    };
    let scopes = outline_analysis(&analysis);
    // The caret starts at line 0, which is usually a comment or the story
    // header — a scope with nothing in it, which reads as an empty panel on
    // a story that is not empty. Open on the first line that has a scene.
    let first_scene = scopes
        .iter()
        .position(|scope| scope.scene.is_some())
        .unwrap_or(0);
    let at = (*cursor.read())
        .max(first_scene)
        .min(lines.len().saturating_sub(1));
    let here = scopes.get(at).cloned().unwrap_or_default();
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
                                state.story_source = STARTER.to_owned();
                                state.has_unsaved_changes = true;
                            },
                            "Start a story"
                        }
                    }
                } else {
                    // The rendered layer is the visible text; the textarea
                    // sits over it transparently, exactly as the command bar
                    // does, so highlighting and the caret share metrics.
                    div { class: "relative min-h-0 flex-1 overflow-auto",
                        div { class: "pointer-events-none absolute inset-0 z-10 px-4 py-3 font-mono text-[12.5px] leading-[1.75] whitespace-pre-wrap",
                            for (index , line) in lines.iter().enumerate() {
                                div {
                                    key: "{index}",
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
                            id: "svs-writer",
                            class: "absolute inset-0 z-20 size-full resize-none bg-transparent px-4 py-3 font-mono text-[12.5px] leading-[1.75] text-transparent caret-violet-400 outline-none",
                            spellcheck: "false",
                            autocomplete: "off",
                            value: "{source}",
                            oninput: move |event| {
                                let mut state = app_state.write();
                                state.story_source = event.value();
                                state.has_unsaved_changes = true;
                            },
                        }
                    }
                    for diagnostic in analysis.diagnostics.iter().filter(|d| d.severity == Severity::Error).take(3) {
                        p { class: "px-4 py-1 font-mono text-[11px] text-rose-300",
                            "{diagnostic.message}"
                        }
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
    let mut composer = use_context::<Signal<String>>();
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

                        if mention.pending() {
                            button {
                                class: "shrink-0 rounded-md bg-amber-400/15 px-2 py-1 text-[10px] font-semibold text-amber-300 transition hover:bg-amber-400/25",
                                title: "Prefill the command bar to create it",
                                onclick: {
                                    let name = mention.name.clone();
                                    let kind = mention.kind.unwrap_or(ArtifactKind::Character).as_str();
                                    move |_| composer.set(format!("/create {kind} --name {name} "))
                                },
                                "Create"
                            }
                        } else {
                            button {
                                class: "shrink-0 rounded-md px-2 py-1 text-[10px] text-zinc-600 transition hover:text-zinc-200",
                                title: "Open in the canvas",
                                onclick: {
                                    let id = mention.artifact;
                                    move |_| {
                                        if let Some(id) = id {
                                            app_state.write().select_artifact(id);
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

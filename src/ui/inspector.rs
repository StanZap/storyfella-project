//! The inspector rail — an artifact's canonical description and the
//! parameters its generations use.
//!
//! Every control here is also a command. Changing the canvas runs `/resize`,
//! the steps slider and the seed lock run `/set`, editing the description
//! runs `/describe`. The rail and the command bar are two views of one
//! vocabulary, not two features — which is why nothing in this file mutates
//! the registry directly: it emits text into the composer's submit path.
//!
//! The description is labelled **canon** because that is what it is:
//! generations are grounded against it, so editing it changes every future
//! render, while `/modify` only produces an experiment against it. Making
//! that difference visible is the point of the rail.

use dioxus::prelude::*;

use crate::registry::{Artifact, LoraEntry};

/// The canvas sizes offered as presets. Not a closed set — the parser takes
/// any multiple of 32 within bounds — but these are the ones the draft-then-
/// commit workflow actually uses.
const SIZE_PRESETS: [(u32, u32); 4] = [(384, 576), (512, 768), (768, 1152), (1024, 1536)];

/// Steps the slider snaps to, cheapest first.
const STEP_STOPS: [u32; 6] = [4, 6, 8, 12, 16, 24];

#[component]
pub fn Inspector(
    artifact: Artifact,
    /// The artifact's `c:` reference, for the commands this rail emits.
    reference: String,
    bound_loras: Vec<LoraEntry>,
    disabled: bool,
    /// Runs a command through the same path as the command bar.
    onrun: EventHandler<String>,
) -> Element {
    let mut draft = use_signal(|| artifact.description.clone());
    // The description belongs to the artifact, not to this component: when
    // the selection changes, or a promote rewrites it, the field must follow.
    use_effect(use_reactive(&artifact.description, move |description| {
        draft.set(description);
    }));

    let size = artifact
        .default_size
        .or_else(|| artifact.kind.default_size());
    let steps = artifact.default_steps;
    let seed = artifact.default_seed;
    let dirty = *draft.read() != artifact.description;
    let visual = artifact.kind.is_visual();

    // The commands are built here, one owned string per control, because a
    // closure inside an rsx `for` cannot move the shared `reference` twice.
    let sizes: Vec<(u32, u32, bool, String)> = SIZE_PRESETS
        .iter()
        .map(|&(width, height)| {
            (
                width,
                height,
                size == Some((width, height)),
                format!("/resize {reference} {width}x{height}"),
            )
        })
        .collect();
    let step_stops: Vec<(u32, bool, String)> = STEP_STOPS
        .iter()
        .map(|&stop| {
            (
                stop,
                steps == Some(stop),
                format!("/set {reference} --steps {stop}"),
            )
        })
        .collect();
    let steps_auto = format!("/set {reference} --steps auto");
    let seed_toggle = if seed.is_some() {
        format!("/set {reference} --seed random")
    } else {
        // Pinning picks a seed now so the value is visible and repeatable;
        // releasing hands it back to the sampler.
        format!("/set {reference} --seed {}", fresh_seed())
    };
    let describe_ref = reference.clone();
    // `/attach-lora` needs an adapter name, so this hands the caret to the
    // bar rather than running anything.
    let attach_prefix = format!("/attach-lora {reference} ");
    let mut composer = use_context::<Signal<String>>();

    rsx! {
        aside { class: "flex w-[300px] shrink-0 flex-col border-l border-white/[0.055] bg-zinc-950/35",
            div { class: "flex items-center gap-2 border-b border-white/[0.05] px-4 py-3",
                p { class: "text-[10px] font-semibold uppercase tracking-[0.18em] text-zinc-600",
                    "{artifact.kind}"
                }
                span { class: "ml-auto font-mono text-[10px] text-zinc-600", "{reference}" }
            }

            div { class: "min-h-0 flex-1 space-y-6 overflow-y-auto p-4",

                // ---------------------------------------------------- canon
                section {
                    div { class: "mb-2 flex items-center gap-2",
                        p { class: "text-[10px] font-semibold uppercase tracking-[0.07em] text-zinc-600",
                            "Description"
                        }
                        span { class: "rounded bg-violet-400/15 px-1.5 py-0.5 text-[9px] font-bold text-violet-300",
                            "canon"
                        }
                    }
                    textarea {
                        class: "block min-h-[104px] w-full resize-y rounded-[9px] bg-white/[0.03] px-2.5 py-2 text-[12px] leading-6 text-zinc-200 outline-none ring-1 ring-inset ring-white/[0.06] transition focus:bg-white/[0.05] focus:ring-violet-400/40 disabled:opacity-40",
                        value: "{draft}",
                        disabled,
                        spellcheck: "false",
                        oninput: move |event| draft.set(event.value()),
                    }
                    div { class: "mt-1.5 flex items-center gap-2",
                        p { class: "flex-1 text-[10px] leading-4 text-zinc-600",
                            "Every generation is grounded against this."
                        }
                        button {
                            class: "rounded-md bg-violet-400/15 px-2.5 py-1 text-[11px] font-semibold text-violet-200 transition hover:bg-violet-400/25 disabled:opacity-30",
                            disabled: disabled || !dirty || draft.read().trim().is_empty(),
                            onclick: move |_| {
                                onrun.call(describe_command(&describe_ref, &draft.read()));
                            },
                            "Save"
                        }
                    }
                }

                if visual {
                    // ----------------------------------------------- canvas
                    section {
                        p { class: "mb-2 text-[10px] font-semibold uppercase tracking-[0.07em] text-zinc-600",
                            "Canvas"
                        }
                        div { class: "grid grid-cols-2 gap-1 rounded-[9px] bg-white/[0.03] p-1",
                            for (width , height , active , command) in sizes {
                                SizeButton {
                                    key: "{width}x{height}",
                                    width,
                                    height,
                                    active,
                                    disabled,
                                    onpick: move |_| onrun.call(command.clone()),
                                }
                            }
                        }
                        p { class: "mt-1.5 text-[10px] leading-4 text-zinc-600",
                            "Resize re-renders at the new canvas."
                        }
                    }

                    // ------------------------------------------------ steps
                    section {
                        div { class: "mb-2 flex items-baseline gap-2",
                            p { class: "text-[10px] font-semibold uppercase tracking-[0.07em] text-zinc-600",
                                "Steps"
                            }
                            span { class: "ml-auto font-mono text-[11px] text-zinc-300",
                                match steps {
                                    Some(steps) => format!("{steps}"),
                                    None => "auto".to_owned(),
                                }
                            }
                        }
                        div { class: "flex flex-wrap gap-1",
                            for (stop , active , command) in step_stops {
                                StepButton {
                                    key: "{stop}",
                                    steps: stop,
                                    active,
                                    disabled,
                                    onpick: move |_| onrun.call(command.clone()),
                                }
                            }
                            button {
                                class: if steps.is_none() { "rounded-md bg-violet-400/15 px-2 py-1 font-mono text-[10px] text-violet-300" } else { "rounded-md px-2 py-1 font-mono text-[10px] text-zinc-600 transition hover:text-zinc-300" },
                                disabled,
                                onclick: move |_| onrun.call(steps_auto.clone()),
                                "auto"
                            }
                        }
                        p { class: "mt-1.5 text-[10px] leading-4 text-zinc-600",
                            "Fewer steps is faster and rougher. Applies to the next render."
                        }
                    }

                    // ------------------------------------------------- seed
                    section {
                        div { class: "mb-2 flex items-baseline gap-2",
                            p { class: "text-[10px] font-semibold uppercase tracking-[0.07em] text-zinc-600",
                                "Seed"
                            }
                            span { class: "ml-auto text-[10px] text-zinc-600",
                                if seed.is_some() { "reproducible" } else { "rerolls each run" }
                            }
                        }
                        div { class: "flex items-center gap-2",
                            span { class: "flex-1 truncate rounded-[7px] bg-white/[0.03] px-2.5 py-1.5 font-mono text-[12px] text-zinc-300 ring-1 ring-inset ring-white/[0.06]",
                                match seed {
                                    Some(seed) => format!("{seed}"),
                                    None => "—".to_owned(),
                                }
                            }
                            button {
                                class: if seed.is_some() { "rounded-[7px] bg-amber-400/15 px-2.5 py-1.5 text-[11px] font-semibold text-amber-300 transition hover:bg-amber-400/25 disabled:opacity-30" } else { "rounded-[7px] bg-white/[0.05] px-2.5 py-1.5 text-[11px] font-semibold text-zinc-400 transition hover:bg-white/[0.09] disabled:opacity-30" },
                                disabled,
                                onclick: move |_| onrun.call(seed_toggle.clone()),
                                if seed.is_some() { "Unpin" } else { "Pin" }
                            }
                        }
                        p { class: "mt-1.5 text-[10px] leading-4 text-zinc-600",
                            "Pinned, an edit shows only what the edit changed."
                        }
                    }

                    // ------------------------------------------------ loras
                    section {
                        p { class: "mb-2 text-[10px] font-semibold uppercase tracking-[0.07em] text-zinc-600",
                            "Adapters"
                        }
                        if bound_loras.is_empty() {
                            p { class: "text-[10px] italic text-zinc-700", "None bound." }
                            button {
                                class: "mt-2 w-full rounded-md bg-white/[0.05] py-1.5 text-[10px] font-semibold text-zinc-400 transition hover:bg-white/[0.09] hover:text-zinc-200 disabled:opacity-30",
                                disabled,
                                title: "Prefill the command bar — the adapter name comes next",
                                onclick: move |_| composer.set(attach_prefix.clone()),
                                "Attach an adapter"
                            }
                        } else {
                            div { class: "space-y-1.5",
                                for lora in bound_loras.iter() {
                                    div { key: "{lora.id}",
                                        class: "flex items-center gap-2 rounded-[7px] bg-white/[0.03] px-2 py-1.5 ring-1 ring-inset ring-white/[0.05]",
                                        span { class: "min-w-0 flex-1 truncate font-mono text-[11px] text-violet-200",
                                            "{lora.name}"
                                        }
                                        span { class: "font-mono text-[10px] text-zinc-500",
                                            "{lora.multiplier:.2}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // ------------------------------------------------ appears in
                // Filled by the compiler once `.sf` files exist; renders
                // nothing until then (see docs/design/ui-plan.md §2).
                if let Some(origins) = story_origins(&artifact) {
                    section {
                        p { class: "mb-2 text-[10px] font-semibold uppercase tracking-[0.07em] text-zinc-600",
                            "Appears in"
                        }
                        div { class: "flex flex-wrap gap-1.5",
                            for origin in origins {
                                span { key: "{origin}",
                                    class: "rounded-md bg-teal-400/10 px-1.5 py-1 font-mono text-[10px] text-teal-300",
                                    "{origin}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SizeButton(
    width: u32,
    height: u32,
    active: bool,
    disabled: bool,
    onpick: EventHandler<()>,
) -> Element {
    rsx! {
        button {
            class: if active { "rounded-md bg-violet-400/20 py-1.5 font-mono text-[10.5px] font-semibold text-violet-200" } else { "rounded-md py-1.5 font-mono text-[10.5px] text-zinc-500 transition hover:text-zinc-200 disabled:opacity-40" },
            disabled,
            onclick: move |_| onpick.call(()),
            "{width}x{height}"
        }
    }
}

#[component]
fn StepButton(steps: u32, active: bool, disabled: bool, onpick: EventHandler<()>) -> Element {
    rsx! {
        button {
            class: if active { "rounded-md bg-violet-400/15 px-2 py-1 font-mono text-[10px] text-violet-300" } else { "rounded-md px-2 py-1 font-mono text-[10px] text-zinc-600 transition hover:text-zinc-300 disabled:opacity-40" },
            disabled,
            onclick: move |_| onpick.call(()),
            "{steps}"
        }
    }
}

/// Where the story references this artifact. Always `None` until the DSL
/// lands — the slot exists so the compiler can fill it without this
/// component changing shape.
fn story_origins(_artifact: &Artifact) -> Option<Vec<String>> {
    None
}

/// A seed to pin. Uses the clock rather than a generator dependency; the
/// only requirement is that it differs between pins.
fn fresh_seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64 % 1_000_000)
        .unwrap_or(42)
}

/// The `/describe` command the Save button emits, exposed for tests.
pub fn describe_command(reference: &str, description: &str) -> String {
    format!("/describe {reference} {:?}", description.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_presets_are_all_valid_canvases() {
        // The rail must never offer a size /resize would reject.
        for (width, height) in SIZE_PRESETS {
            assert!(
                crate::registry::pipeline::is_valid_size(width, height),
                "{width}x{height} is not a valid canvas"
            );
        }
    }

    #[test]
    fn the_commands_the_rail_emits_parse() {
        use crate::registry::slash::parse_slash;
        let mut commands = vec![
            describe_command("c:mara", "  A harbourmaster  "),
            "/set c:mara --steps auto".to_owned(),
            "/set c:mara --seed random".to_owned(),
            format!("/set c:mara --seed {}", fresh_seed()),
        ];
        for (width, height) in SIZE_PRESETS {
            commands.push(format!("/resize c:mara {width}x{height}"));
        }
        for steps in STEP_STOPS {
            commands.push(format!("/set c:mara --steps {steps}"));
        }
        for command in commands {
            assert!(
                parse_slash(&command).is_ok(),
                "rail emitted an unparseable command: {command}"
            );
        }
    }

    #[test]
    fn descriptions_with_quotes_survive_the_round_trip() {
        use crate::registry::{ops::Operation, slash::parse_slash};
        // `{:?}` escapes embedded quotes, which is why the rail uses it
        // rather than wrapping in bare quotes.
        let command = describe_command("c:mara", r#"wearing a "storm" coat"#);
        match parse_slash(&command).expect("should parse") {
            Operation::Describe { description, .. } => {
                assert_eq!(description, r#"wearing a "storm" coat"#);
            }
            other => panic!("expected describe, got {other:?}"),
        }
    }

    #[test]
    fn the_story_slot_stays_empty_until_the_dsl_lands() {
        let artifact = Artifact {
            id: uuid::Uuid::new_v4(),
            kind: crate::registry::ArtifactKind::Character,
            name: "mara".into(),
            description: "a harbourmaster".into(),
            variant_of: None,
            variant_axis: None,
            default_size: None,
            default_steps: None,
            default_seed: None,
            parent_id: None,
            composition: None,
            active_revision_id: None,
            revisions: Vec::new(),
            drafts: Vec::new(),
        };
        assert!(story_origins(&artifact).is_none());
    }
}

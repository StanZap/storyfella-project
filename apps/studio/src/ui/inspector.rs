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

use storyfella_studio_core::registry::{Artifact, LoraEntry};

/// The canvas sizes offered as presets. Not a closed set — the parser takes
/// any multiple of 32 within bounds — but these are the ones the draft-then-
/// commit workflow actually uses.
const SIZE_PRESETS: [(u32, u32); 7] = [
    (384, 576),
    (512, 768),
    (768, 1152),
    (1024, 1536),
    (1024, 576),
    (768, 768),
    (1024, 1024),
];

/// Steps the slider snaps to, cheapest first.
const STEP_STOPS: [u32; 7] = [4, 6, 8, 12, 16, 24, 25];

/// How much one press moves an adapter's weight. Small enough that the
/// visible effect of a step is a nudge rather than a different image.
const ADAPTER_WEIGHT_STEP: f32 = 0.05;

/// One bound adapter, with the commands its controls emit.
#[derive(Clone, PartialEq)]
struct AdapterRow {
    id: uuid::Uuid,
    name: String,
    multiplier: f32,
    /// Bound to a base artifact rather than this one.
    inherited: bool,
    weaker: String,
    stronger: String,
    remove: String,
}

/// Builds the adapter rows for `artifact_id` from its bound entries.
///
/// Separated from the component so the commands the controls emit are
/// assertable: every one of them has to survive `parse_slash` and the
/// operation's own validation, and a control that emits a rejected command
/// is a dead button.
fn adapter_rows(artifact_id: uuid::Uuid, bound: &[LoraEntry]) -> Vec<AdapterRow> {
    bound
        .iter()
        .map(|lora| {
            let step = |delta: f32| {
                // The same bounds the operation validates against, so a
                // control can never emit a command the compiler rejects.
                let next = (lora.multiplier + delta).clamp(-2.0, 2.0);
                format!("/lora-scale {} {next:.2}", lora.name)
            };
            AdapterRow {
                id: lora.id,
                name: lora.name.clone(),
                multiplier: lora.multiplier,
                // Bindings are inherited down the variant chain, so a variant
                // lists its base's adapters too. Saying which is which
                // matters: detaching one here removes it from the base, and
                // from every other variant of that base with it.
                inherited: lora.artifact_id != Some(artifact_id),
                weaker: step(-ADAPTER_WEIGHT_STEP),
                stronger: step(ADAPTER_WEIGHT_STEP),
                remove: format!("/lora-remove {}", lora.name),
            }
        })
        .collect()
}

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
    let mut app_state = use_context::<Signal<crate::state::AppState>>();
    let draft = app_state
        .read()
        .description_draft(artifact.id)
        .unwrap_or(&artifact.description)
        .to_owned();
    let artifact_id = artifact.id;
    let (description_id, description_mount) =
        super::text_input::use_text_value(draft.clone(), None);
    let defaults = use_context::<Signal<storyfella_studio_core::registry::GenerationDefaults>>();

    let size = artifact
        .default_size
        .or_else(|| defaults.read().size_for(artifact.kind));
    let steps = artifact.default_steps;
    let seed = artifact.default_seed;
    let dirty = draft != artifact.description;
    let effective_steps = steps.unwrap_or(defaults.read().steps);
    let mut custom_size = use_signal(String::new);
    let (size_id, size_mount) = super::text_input::use_text_value(custom_size(), None);
    let mut custom_error = use_signal(|| None::<String>);
    let custom_ref = reference.clone();
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
    let size_auto = format!("/set {reference} --size auto");
    let seed_toggle = if seed.is_some() {
        format!("/set {reference} --seed random")
    } else {
        // Pinning picks a seed now so the value is visible and repeatable;
        // releasing hands it back to the sampler.
        format!("/set {reference} --seed {}", fresh_seed())
    };
    // One owned row per adapter, commands included: a closure inside an rsx
    // `for` cannot borrow from the iterator, and each control is a command
    // like every other in this rail.
    let adapters = adapter_rows(artifact.id, &bound_loras);

    let describe_ref = reference.clone();
    // `/attach-lora` needs an adapter name, so this hands the caret to the
    // bar rather than running anything.
    let attach_prefix = format!("/attach-lora {reference} ");
    let composer = use_context::<Signal<String>>();

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
                        aria_label: "Canonical artifact description",
                        class: "block min-h-[104px] w-full resize-y rounded-[9px] bg-white/[0.03] px-2.5 py-2 text-[12px] leading-6 text-zinc-200 outline-none ring-1 ring-inset ring-white/[0.06] transition focus:bg-white/[0.05] focus:ring-violet-400/40 disabled:opacity-40",
                        id: description_id,
                        onmounted: description_mount,
                        disabled,
                        spellcheck: "false",
                        oninput: move |event| app_state.write().set_description_draft(artifact_id, event.value()),
                    }
                    div { class: "mt-1.5 flex items-center gap-2",
                        p { class: "flex-1 text-[10px] leading-4 text-zinc-600",
                            "Every generation is grounded against this."
                        }
                        button {
                            class: "rounded-md bg-violet-400/15 px-2.5 py-1 text-[11px] font-semibold text-violet-200 transition hover:bg-violet-400/25 disabled:opacity-30",
                            disabled: disabled || !dirty || draft.trim().is_empty(),
                            onclick: move |_| {
                                onrun.call(describe_command(&describe_ref, &draft));
                            },
                            "Apply description"
                        }
                        if dirty {
                            button { disabled, onclick: move |_| app_state.write().discard_description_draft(artifact_id), "Discard draft" }
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
                            {size.map(|(width, height)| format!("Effective size: {width}×{height}{}", if artifact.default_size.is_none() { " (inherited)" } else { "" })).unwrap_or_default()}
                            " · Resize generates a new image."
                        }
                        button { disabled: disabled || artifact.default_size.is_none(), onclick: move |_| onrun.call(size_auto.clone()), "Use project size on next generation" }
                        input { id: size_id, onmounted: size_mount, class: "setting-control mt-2", aria_label: "Custom canvas size", placeholder: "Custom width x height", disabled,
                            oninput: move |event| custom_size.set(event.value()) }
                        button { disabled, onclick: move |_| {
                            let size = custom_size().replace('×', "x");
                            let dimensions = size.split_once('x').and_then(|(w, h)| Some((w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?)))
                                .filter(|&(w, h)| storyfella_studio_core::registry::pipeline::is_valid_size(w, h));
                            if let Some((width, height)) = dimensions { custom_error.set(None); onrun.call(format!("/resize {custom_ref} {width}x{height}")); }
                            else { custom_error.set(Some("Use width x height, multiples of 32 within supported bounds.".into())); }
                        }, "Generate at custom size" }
                        if let Some(error) = custom_error() { p { role: "alert", "{error}" } }
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
                                    None => format!("{effective_steps} (inherited)"),
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
                        if adapters.is_empty() {
                            p { class: "text-[10px] italic text-zinc-700", "None bound." }
                        } else {
                            div { class: "space-y-1.5",
                                for adapter in adapters.iter().cloned() {
                                    div { key: "{adapter.id}",
                                        class: "flex items-center gap-1.5 rounded-[7px] bg-white/[0.03] px-2 py-1.5 ring-1 ring-inset ring-white/[0.05]",
                                        span { class: "min-w-0 flex-1 truncate font-mono text-[11px] text-violet-200",
                                            title: if adapter.inherited { "Bound to the base artifact — every variant of it carries this" } else { "Bound to this artifact" },
                                            "{adapter.name}"
                                        }
                                        if adapter.inherited {
                                            span { class: "rounded bg-white/[0.06] px-1 py-0.5 text-[8px] font-bold uppercase tracking-wider text-zinc-500",
                                                "base"
                                            }
                                        }
                                        button {
                                            class: "rounded px-1 font-mono text-[11px] leading-none text-zinc-500 transition hover:bg-white/[0.07] hover:text-zinc-200 disabled:opacity-30",
                                            disabled,
                                            title: "Weaker",
                                            onclick: {
                                                let command = adapter.weaker.clone();
                                                move |_| onrun.call(command.clone())
                                            },
                                            "−"
                                        }
                                        span { class: "w-8 text-center font-mono text-[10px] text-zinc-400",
                                            "{adapter.multiplier:.2}"
                                        }
                                        button {
                                            class: "rounded px-1 font-mono text-[11px] leading-none text-zinc-500 transition hover:bg-white/[0.07] hover:text-zinc-200 disabled:opacity-30",
                                            disabled,
                                            title: "Stronger",
                                            onclick: {
                                                let command = adapter.stronger.clone();
                                                move |_| onrun.call(command.clone())
                                            },
                                            "+"
                                        }
                                        button {
                                            class: "rounded px-1 font-mono text-[11px] leading-none text-zinc-600 transition hover:bg-rose-400/15 hover:text-rose-300 disabled:opacity-30",
                                            disabled,
                                            title: "Detach this adapter",
                                            onclick: {
                                                let command = adapter.remove.clone();
                                                move |_| onrun.call(command.clone())
                                            },
                                            "×"
                                        }
                                    }
                                }
                            }
                        }
                        button {
                            class: "mt-2 w-full rounded-md bg-white/[0.05] py-1.5 text-[10px] font-semibold text-zinc-400 transition hover:bg-white/[0.09] hover:text-zinc-200 disabled:opacity-30",
                            disabled,
                            title: "Prefill the command bar — the adapter name comes next",
                            onclick: move |_| super::cmdbar::prefill(composer, attach_prefix.clone()),
                            "Attach an adapter"
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
                storyfella_studio_core::registry::pipeline::is_valid_size(width, height),
                "{width}x{height} is not a valid canvas"
            );
        }
    }

    /// The adapter controls are commands like everything else in the rail,
    /// so they must parse — and the weight steps must stay inside the range
    /// `/lora-scale` validates, or a button at the end of the range emits a
    /// command that is rejected.
    #[test]
    fn adapter_controls_emit_commands_that_parse_and_stay_in_range() {
        use storyfella_studio_core::registry::slash::parse_slash;
        let mine = uuid::Uuid::new_v4();
        let base = uuid::Uuid::new_v4();
        let entry = |name: &str, multiplier: f32, owner: uuid::Uuid| LoraEntry {
            id: uuid::Uuid::new_v4(),
            name: name.to_owned(),
            path: format!("{name}.safetensors"),
            base_model: None,
            multiplier,
            artifact_id: Some(owner),
        };
        let bound = vec![
            entry("facial-detail", 0.8, mine),
            // At the ceiling: stepping up must clamp, not overshoot.
            entry("inherited-style", 2.0, base),
        ];
        let rows = adapter_rows(mine, &bound);

        assert!(!rows[0].inherited, "bound to this artifact");
        assert!(rows[1].inherited, "bound to the base, shown as inherited");

        for row in &rows {
            for command in [&row.weaker, &row.stronger, &row.remove] {
                parse_slash(command).unwrap_or_else(|error| {
                    panic!("the rail emitted {command:?}, which does not parse: {error}")
                });
            }
        }
        assert_eq!(rows[0].weaker, "/lora-scale facial-detail 0.75");
        assert_eq!(rows[0].stronger, "/lora-scale facial-detail 0.85");
        assert_eq!(
            rows[1].stronger, "/lora-scale inherited-style 2.00",
            "stepping past the ceiling clamps to it"
        );
    }

    #[test]
    fn the_commands_the_rail_emits_parse() {
        use storyfella_studio_core::registry::slash::parse_slash;
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
        use storyfella_studio_core::registry::{ops::Operation, slash::parse_slash};
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
}

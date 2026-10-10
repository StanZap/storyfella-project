//! Searchable actions and requests, with an explicit slash argument editor.
//!
//! A transparent `<textarea>` sits exactly on top of a highlight layer that
//! paints the same text with per-token colour. The caret comes from the
//! input, the colour from the layer, and the two stay aligned because they
//! share font metrics and because **chips take their padding from
//! `box-shadow` spread rather than `padding`** — padding would change the
//! text's advance width and drift the caret away from the glyphs.
//!
//! Actions come from the shared palette catalog; argument completion comes
//! from [`storyfella_studio_core::registry::completion`].
//!
//! ## Why there is JavaScript here
//!
//! Dioxus 0.7's `HasFormData` exposes `value()` but no caret, and completion
//! is meaningless without knowing where the caret is (`/modify c:ma| --steps
//! 8` must complete the reference, not the tail). So a long-lived
//! `document::eval` attaches listeners to the input and streams
//! `(value, caret)` back.
//!
//! The caret crosses that boundary as a **UTF-8 byte offset**, converted on
//! the JavaScript side. `selectionStart` counts UTF-16 code units, and the
//! two disagree on exactly the characters this bar has to handle: WebKit
//! rewrites `--` to `—` (1 unit, 3 bytes) and `"` to `“ ”`. Passing
//! `selectionStart` straight through would put the caret in the wrong token
//! the moment autocorrect fires.

use super::{
    actions::ActionDispatcher,
    icons::{Icon, IconName},
    palette,
};
use crate::state::AppState;
use dioxus::prelude::*;

use storyfella_studio_core::registry::{
    completion::{self, Completion, CompletionKind, RefCandidate, TokenKind},
    slash::{self, Span},
};

/// The id the caret bridge binds to. One command bar at a time.
pub const BAR_ID: &str = "svs-cmdbar";

/// Prefills the command field and puts the caret in it.
///
/// Every affordance that hands the user a command — a keyboard shortcut, a
/// canvas button, a mention in the Writer, the palette — goes through here: a
/// prefill is only useful when the next keystroke lands in the field, and the
/// field may sit on a screen (the canvas) still hidden when the caller runs, so
/// focus is retried across frames until the bar is visible.
pub fn prefill(mut composer: Signal<String>, command: impl Into<String>) {
    let command = command.into();
    let encoded = serde_json::to_string(&command).expect("command string serializes");
    composer.set(command);
    document::eval(&format!(
        "const el = document.getElementById('{BAR_ID}'); if (el) el.value = {encoded};"
    ));
    document::eval(&focus_bar(true));
}

/// Focuses the command field without changing it (the `/` shortcut).
pub fn focus_field() {
    document::eval(&focus_bar(false));
}

/// Clear accepted input in native and application state. A conditional typing
/// sync can skip a value after a burst of native input events.
pub fn clear(mut composer: Signal<String>) {
    composer.set(String::new());
    document::eval(&format!("const el = document.getElementById('{BAR_ID}'); if (el) {{ el.value = ''; el.style.height = 'auto'; el.setSelectionRange(0, 0); }}"));
}

/// The script that focuses the command bar once it is visible; `caret_to_end`
/// moves the caret after the text a prefill just wrote.
fn focus_bar(caret_to_end: bool) -> String {
    let caret = if caret_to_end {
        "const end = el.value.length; el.setSelectionRange(end, end);"
    } else {
        ""
    };
    format!(
        "(() => {{ let tries = 0; const tick = () => {{ const el = document.getElementById(\"{BAR_ID}\"); if (el && el.offsetParent !== null) {{ el.focus(); {caret} return; }} if (tries++ < 10) requestAnimationFrame(tick); }}; requestAnimationFrame(tick); }})();"
    )
}

/// Streams `[value, caret_in_utf8_bytes]` on every edit, click, or selection
/// change, and keeps the field's height matched to its content.
///
/// Re-attaching is guarded so a re-render cannot stack listeners. The
/// auto-grow is here rather than in Rust because it needs `scrollHeight`,
/// which only exists once the browser has laid the text out.
const CARET_BRIDGE: &str = r#"
    const el = document.getElementById("svs-cmdbar");
    if (el && !el.dataset.bridged) {
        el.dataset.bridged = "1";
        const encoder = new TextEncoder();
        const grow = () => {
            // Collapse first: scrollHeight never shrinks below the current
            // height, so without this the field only ever grows.
            el.style.height = "auto";
            el.style.height = el.scrollHeight + "px";
        };
        const push = () => {
            grow();
            const caret = el.selectionStart ?? el.value.length;
            const upto = el.value.slice(0, caret);
            // UTF-16 units -> UTF-8 bytes, so Rust can slice the string.
            dioxus.send([el.value, encoder.encode(upto).length]);
        };
        for (const name of ["input", "keyup", "click", "focus", "select"]) {
            el.addEventListener(name, push);
        }
        push();
    }
"#;

/// One painted run of the input line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    /// Utility classes for this run; empty for the gaps between tokens.
    pub class: &'static str,
}

fn ref_class(known: bool) -> &'static str {
    if known {
        "rounded-sm bg-violet-400/15 text-violet-200 shadow-[0_0_0_2px_rgba(167,139,250,.15)]"
    } else {
        "rounded-sm bg-rose-400/15 text-rose-300 shadow-[0_0_0_2px_rgba(251,113,133,.15)]"
    }
}

const BAD: &str = "text-rose-400 underline decoration-wavy decoration-rose-400/60";

/// Splits `input` into coloured runs.
///
/// Gaps between tokens (whitespace, and the quote characters a token's span
/// covers) are emitted verbatim so the layer is character-for-character the
/// same width as the input. `known` is the set of artifact names that
/// resolve; a `c:` reference to anything else is painted as an error.
pub fn paint(input: &str, known: &[String]) -> Vec<Segment> {
    let tokens = slash::tokenize(input);
    let kinds = completion::classify(&tokens);
    let mut segments = Vec::new();
    let mut cut = 0usize;

    for (token, kind) in tokens.iter().zip(kinds) {
        if token.span.start > cut {
            segments.push(Segment {
                text: input[cut..token.span.start].to_owned(),
                class: "",
            });
        }
        let class = match kind {
            TokenKind::Operation { known: true } => "font-semibold text-violet-300",
            TokenKind::Operation { known: false } => BAD,
            TokenKind::Ref => {
                let name = token.text.strip_prefix("c:").unwrap_or(&token.text);
                ref_class(known.iter().any(|candidate| candidate == name))
            }
            TokenKind::Flag { accepted: true } => "text-sky-300",
            TokenKind::Flag { accepted: false } => BAD,
            TokenKind::Size | TokenKind::Number => "text-emerald-300",
            TokenKind::Text => "text-zinc-400",
        };
        segments.push(Segment {
            text: input[token.span.start..token.span.end].to_owned(),
            class,
        });
        cut = token.span.end;
    }
    if cut < input.len() {
        segments.push(Segment {
            text: input[cut..].to_owned(),
            class: "",
        });
    }
    segments
}

/// Splices an accepted completion into the line.
///
/// Returns the new text and the caret as a byte offset.
///
/// A separating space is added unless the tail already starts with one, so
/// accepting twice in a row cannot glue arguments together. The caret always
/// lands immediately after the inserted label — when a space was added it
/// follows that space (ready for the next argument), and when the tail
/// already supplied one the caret stays on the word just completed.
pub fn accept(input: &str, label: &str, replace: Span) -> (String, usize) {
    let start = replace.start.min(input.len());
    let end = replace.end.min(input.len()).max(start);
    let tail = &input[end..];
    let needs_space = !tail.starts_with(' ');

    let mut next = String::with_capacity(input.len() + label.len() + 1);
    next.push_str(&input[..start]);
    next.push_str(label);
    if needs_space {
        next.push(' ');
    }
    next.push_str(tail);

    let caret = start + label.len() + usize::from(needs_space);
    (next, caret)
}

/// Inserts `label` over `replace` and moves the caret after it.
///
/// Dioxus can set an input's value but not its selection, so the write goes
/// through `eval`. The offset is converted to UTF-16 here because
/// `setSelectionRange` counts code units while `accept` returns bytes.
fn splice(
    mut value: Signal<String>,
    mut caret: Signal<usize>,
    mut selected: Signal<usize>,
    label: &str,
    replace: Span,
) {
    let current = value.peek().clone();
    let (next, byte_caret) = accept(&current, label, replace);
    let utf16 = next[..byte_caret].encode_utf16().count();
    let encoded = serde_json::to_string(&next).unwrap_or_else(|_| "\"\"".to_owned());

    value.set(next);
    caret.set(byte_caret);
    selected.set(0);
    document::eval(&format!(
        "const el = document.getElementById(\"{BAR_ID}\");
         if (el) {{ el.value = {encoded}; el.setSelectionRange({utf16}, {utf16}); el.focus(); }}"
    ));
}

fn completion_badge(kind: CompletionKind) -> (&'static str, &'static str) {
    match kind {
        CompletionKind::Operation => ("/", "bg-violet-400/15 text-violet-300"),
        CompletionKind::Ref => ("c", "bg-violet-400/15 text-violet-200"),
        CompletionKind::Flag => ("-", "bg-sky-400/15 text-sky-300"),
        CompletionKind::Value => ("=", "bg-emerald-400/15 text-emerald-300"),
    }
}

#[component]
pub fn CommandBar(
    /// The line being edited. Owned by the parent so submit can read and
    /// clear it.
    value: Signal<String>,
    app_state: Signal<AppState>,
    /// Artifacts offered as `c:` targets, and the set that resolves.
    refs: Vec<RefCandidate>,
    disabled: bool,
    onsubmit: EventHandler<()>,
) -> Element {
    let mut value = value;
    let mut caret = use_signal(|| 0usize);
    let mut selected = use_signal(|| 0usize);
    let mut dismissed = use_signal(|| false);
    // The bridge pushes once on mount so the bar starts in sync, which would
    // otherwise pop the whole vocabulary open over an untouched screen.
    // Suggestions belong to a focused bar only.
    let mut focused = use_signal(|| false);
    let dispatch = use_context::<ActionDispatcher>();

    // The caret bridge (see the module docs). Runs once after mount; the
    // script itself guards against double-binding.
    use_future(move || async move {
        let mut bridge = document::eval(CARET_BRIDGE);
        while let Ok((text, offset)) = bridge.recv::<(String, usize)>().await {
            // Native oninput owns text. Queued caret events can contain the
            // old command after submission or completion.
            if *value.peek() == text {
                caret.set(offset);
            }
        }
    });

    let text = value.read().clone();
    let (_, mount_text) = super::text_input::use_text_value(text.clone(), Some(BAR_ID));
    let position = (*caret.read()).min(text.len());
    let known: Vec<String> = refs.iter().map(|item| item.name.clone()).collect();

    let segments = paint(&text, &known);
    let completions = completion::complete(&text, position, &refs);
    let action_mode = !raw_command(&text);
    let quick = palette::quick_entries(&app_state.read(), &text);
    let help = (!action_mode)
        .then(|| completion::signature(&text, position))
        .flatten();
    let count = if action_mode {
        quick.len() + usize::from(!text.trim().is_empty() && !slash::is_slash_command(&text))
    } else {
        completions.items.len()
    };
    let showing = focused() && count > 0 && !dismissed() && !disabled;
    let index = (*selected.read()).min(count.saturating_sub(1));
    let scroll_id = showing.then(|| format!("command-option-{index}"));
    use_effect(use_reactive(&scroll_id, move |id| {
        if let Some(id) = id {
            document::eval(&format!(
                "document.getElementById('{id}')?.scrollIntoView({{block:'nearest'}})"
            ));
        }
    }));
    let choose = EventHandler::new(move |action: palette::Action| {
        dismissed.set(true);
        clear(value);
        dispatch.0.call(action);
    });
    let activate = EventHandler::new(move |_| {
        if disabled {
            return;
        }
        // Input and Enter can arrive before the next render. Resolve from
        // current signals so fast typing cannot run a stale request/action.
        let current = value.peek().clone();
        let action = if !raw_command(&current)
            && !*dismissed.peek()
            && (!current.trim().is_empty() || *focused.peek())
        {
            let entries = palette::quick_entries(&app_state.peek(), &current);
            entries.get(*selected.peek()).map(|e| e.action.clone())
        } else {
            None
        };
        if let Some(action) = action {
            choose.call(action);
        } else {
            onsubmit.call(());
        }
    });
    let fallback_label = if slash::is_slash_command(&text) {
        "Run command"
    } else {
        "Review request"
    };
    let submit_label = if action_mode && !dismissed() {
        quick
            .get(index)
            .map(|e| e.label.as_str())
            .unwrap_or(fallback_label)
    } else {
        fallback_label
    };

    let replace = completions.replace;
    // Cloned per render so each handler can own the label it inserts;
    // `splice` itself is a free function because signals are `Copy` and
    // closures are not.
    let labels: Vec<String> = completions
        .items
        .iter()
        .map(|item| item.label.clone())
        .collect();

    rsx! {
        // `flex-1 min-w-0` so the bar fills the composer row and the Run
        // button stays pinned to the right edge; `min-w-0` lets it shrink
        // below its content instead of forcing the row wider.
        div { class: "relative min-w-0 flex-1",

            // Signature help only while an operation is being typed; at
            // rest the placeholder carries the invitation, so the field is
            // not permanently shortened by a line of hint text.
            if help.is_some() {
            div { class: "flex items-center gap-2 px-1 pb-1 font-mono text-[10px] text-zinc-600",
                if let Some(help) = help {
                    span { class: "font-semibold text-violet-300", "{help.op.name}" }
                    for (slot , param) in help.op.params.iter().enumerate() {
                        span {
                            class: if help.active == Some(slot) { "font-semibold text-teal-300" } else { "text-zinc-700" },
                            "{param}"
                        }
                    }
                    if !help.op.flags.is_empty() {
                        span { class: "text-zinc-700", "{help.op.flags.join(\" \")}" }
                    }
                    span { class: "ml-auto font-sans text-[10px] text-zinc-700", "{help.op.summary}" }
                }
            }
            }

            // completion popup, opening upward
            if showing {
                div { class: "absolute bottom-full left-0 z-40 mb-3 w-full min-w-[280px] overflow-hidden rounded-xl bg-[#17171d] shadow-[0_-12px_40px_rgba(0,0,0,.35)] ring-1 ring-white/[0.09]",
                    div { id: "command-completions", role: "listbox", aria_label: if action_mode { "Actions" } else { "Command completions" }, class: "max-h-[min(300px,45vh)] overflow-y-auto p-1",
                    if action_mode {
                        for (slot, entry) in quick.iter().enumerate() {
                            div { key: "{slot}", id: "command-option-{slot}", role: "option", aria_selected: slot == index,
                                class: if slot == index { "flex cursor-pointer items-center gap-3 rounded-lg bg-violet-400/15 px-3 py-2" } else { "flex cursor-pointer items-center gap-3 rounded-lg px-3 py-2 hover:bg-white/[0.04]" },
                                onmousedown: { let action = entry.action.clone(); move |event| { event.prevent_default(); choose.call(action.clone()); } },
                                Icon { name: entry.icon(), class: "size-4 shrink-0 text-zinc-400" }
                                span { class: "text-sm text-zinc-100", "{entry.label}" }
                                if matches!(entry.action, palette::Action::Task(super::actions::StudioAction::Art(action)) if !matches!(action, super::actions::ArtAction::Create(_))) {
                                    span { class: "ml-auto truncate text-xs text-zinc-400", "{entry.hint}" }
                                }
                            }
                        }
                        if !text.trim().is_empty() && !slash::is_slash_command(&text) {
                            div { id: "command-option-{quick.len()}", role: "option", aria_selected: index == quick.len(),
                                class: if index == quick.len() { "flex cursor-pointer items-center gap-3 rounded-lg bg-violet-400/15 px-3 py-2" } else { "flex cursor-pointer items-center gap-3 rounded-lg px-3 py-2 hover:bg-white/[0.04]" },
                                onmousedown: move |event| { event.prevent_default(); onsubmit.call(()); },
                                Icon { name: IconName::Sparkles, class: "size-4 text-zinc-400" }
                                span { class: "text-sm", "Ask assistant" }
                                span { class: "ml-auto text-xs text-zinc-400", "{super::shortcuts::request_keys()}" }
                            }
                        }
                    } else {
                        for (slot , item) in completions.items.iter().enumerate() {
                            CompletionRow {
                                key: "{item.label}",
                                item: item.clone(),
                                id: format!("command-option-{slot}"),
                                active: slot == index,
                                label: item.label.clone(),
                                onpick: move |label: String| {
                                    splice(value, caret, selected, &label, replace);
                                },
                            }
                        }
                    }
                    }
                    div { class: "border-t border-white/[0.06] px-3 py-1.5 text-[10px] text-zinc-600",
                        if action_mode { "↑↓ choose · ↵ open · Esc dismiss" } else { "⇥ complete · ↵ run · ⇧↵ newline" }
                    }
                }
            }

            div { class: "flex items-start gap-2",

                // The field is in flow and drives the height; the painted
                // layer is absolutely positioned over it. Both wrap at the
                // same width with the same font, so their line breaks agree —
                // which is why this can wrap at all. A single-line input
                // would have to scroll, and the layer would have to chase its
                // scrollLeft on every keystroke.
                div { class: "relative min-w-0 flex-1",
                    div { aria_hidden: "true", class: "studio-command-input pointer-events-none absolute inset-0 z-10 whitespace-pre-wrap break-words text-sm leading-5",
                        for (slot , segment) in segments.iter().enumerate() {
                            span { key: "{slot}", class: segment.class, "{segment.text}" }
                        }
                    }
                    textarea {
                        id: BAR_ID,
                        role: "combobox",
                        aria_label: "Search actions or request a change",
                        aria_expanded: showing,
                        aria_haspopup: "listbox",
                        aria_autocomplete: "list",
                        aria_controls: if showing { "command-completions" } else { "" },
                        aria_activedescendant: if showing { format!("command-option-{index}") } else { String::new() },
                        class: "studio-command-input relative z-20 block max-h-24 w-full resize-none overflow-y-auto bg-transparent text-sm leading-5 text-transparent caret-violet-300 disabled:opacity-40",
                        rows: "1",
                        placeholder: "Search actions or describe a change…",
                        onmounted: mount_text,
                        disabled,
                        // WebKit rewrites `--` to an em dash and straight to
                        // curly quotes; the tokenizer normalizes both, but the
                        // bar leans on the literal characters much harder than
                        // a plain textarea does.
                        autocorrect: "off",
                        spellcheck: "false",
                        autocomplete: "off",
                        oninput: move |event| {
                            value.set(event.value());
                            selected.set(0);
                            dismissed.set(false);
                        },
                        onfocusin: move |_| {
                            focused.set(true);
                            dismissed.set(false);
                        },
                        // A completion row cancels its own mousedown, so
                        // clicking one never blurs the input and this cannot
                        // close the popup out from under the click.
                        onfocusout: move |_| focused.set(false),
                        onkeydown: move |event| {
                            match event.key() {
                                Key::Tab if !action_mode && showing && !event.modifiers().contains(Modifiers::SHIFT) && labels.get(index).is_some() => {
                                    event.prevent_default();
                                    if let Some(label) = labels.get(index) {
                                        splice(value, caret, selected, label, replace);
                                    }
                                }
                                Key::Enter if event.modifiers().intersects(Modifiers::META | Modifiers::CONTROL) => {
                                    event.prevent_default();
                                    onsubmit.call(());
                                }
                                Key::Enter if !event.modifiers().contains(Modifiers::SHIFT) => {
                                    event.prevent_default();
                                    activate.call(());
                                }
                                Key::ArrowDown if showing => {
                                    event.prevent_default();
                                    selected.set(index.saturating_add(1).min(count.saturating_sub(1)));
                                }
                                Key::ArrowUp if showing => {
                                    event.prevent_default();
                                    selected.set(index.saturating_sub(1));
                                }
                                Key::Escape => {
                                    event.prevent_default();
                                    dismissed.set(true);
                                }
                                _ => {}
                            }
                        },
                    }
                }
                button { r#type: "button", class: "studio-icon-button", aria_label: submit_label, title: submit_label,
                    disabled: disabled || (text.trim().is_empty() && !showing),
                    onmousedown: move |event| event.prevent_default(),
                    onclick: move |_| activate.call(()),
                    Icon { name: IconName::Arrow, class: "size-4" }
                }
            }
        }
    }
}

/// A space after a slash operation explicitly enters its argument editor.
fn raw_command(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with('/') && text.chars().any(char::is_whitespace)
}

#[component]
fn CompletionRow(
    id: String,
    item: Completion,
    active: bool,
    label: String,
    onpick: EventHandler<String>,
) -> Element {
    let (glyph, badge) = completion_badge(item.kind);
    rsx! {
        div {
            id,
            role: "option",
            aria_selected: active,
            class: if active { "flex cursor-pointer items-center gap-2.5 bg-violet-400/15 px-3 py-1.5" } else { "flex cursor-pointer items-center gap-2.5 px-3 py-1.5" },
            // mousedown, not click: the input must not lose focus first.
            onmousedown: move |event| {
                event.prevent_default();
                onpick.call(label.clone());
            },
            span { class: "flex size-[18px] shrink-0 items-center justify-center rounded-[5px] text-[9px] font-bold {badge}",
                "{glyph}"
            }
            span { class: "font-mono text-[12px] text-zinc-100", "{item.label}" }
            if !item.hint.is_empty() {
                span { class: "ml-auto text-[10.5px] text-zinc-500", "{item.hint}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_require_explicit_raw_command_mode() {
        for text in ["", "edit", "new environment", "/", "/modify"] {
            assert!(!raw_command(text));
        }
        for text in [
            "/modify ",
            "/describe \"c:Captain Mara\" coat",
            "  /set c:mara --steps 12",
        ] {
            assert!(raw_command(text));
        }
    }

    fn known() -> Vec<String> {
        vec!["mara".to_owned(), "tam".to_owned()]
    }

    /// The painted layer must be character-for-character the same as the
    /// input, or the caret drifts away from the glyphs it sits between.
    fn rendered(segments: &[Segment]) -> String {
        segments.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn painting_preserves_the_input_exactly() {
        for input in [
            "",
            "/modify c:mara give her a bob cut",
            "  /variant   c:tam  \"in oilskins\"  --axis outfit  ",
            "/modify c:mara “a bob cut” —steps 8",
            "/nope",
        ] {
            assert_eq!(
                rendered(&paint(input, &known())),
                input,
                "lost text: {input:?}"
            );
        }
    }

    #[test]
    fn references_are_painted_by_whether_they_resolve() {
        let segments = paint("/modify c:mara", &known());
        let chip = segments.last().unwrap();
        assert_eq!(chip.text, "c:mara");
        assert_eq!(chip.class, ref_class(true));

        let segments = paint("/modify c:ghost", &known());
        assert_eq!(segments.last().unwrap().class, ref_class(false));
    }

    #[test]
    fn unknown_operations_and_rejected_flags_are_marked() {
        assert_eq!(paint("/nope", &known())[0].class, BAD);
        // /describe takes no flags at all.
        let segments = paint("/describe c:mara --axis outfit", &known());
        let flag = segments.iter().find(|s| s.text == "--axis").unwrap();
        assert_eq!(flag.class, BAD);
    }

    #[test]
    fn quoted_and_smart_punctuation_keep_their_original_width() {
        // The span covers `—steps` (8 bytes) while the token text is
        // `--steps` (7). Painting must use the span, not the text.
        let input = "/modify c:mara —steps 8";
        let segments = paint(input, &known());
        assert_eq!(rendered(&segments), input);
        assert!(segments.iter().any(|s| s.text == "—steps"));
    }

    #[test]
    fn accepting_replaces_the_span_and_adds_one_space() {
        let (next, caret) = accept("/mod", "/modify", Span { start: 0, end: 4 });
        assert_eq!(next, "/modify ");
        assert_eq!(caret, 8);
    }

    #[test]
    fn accepting_mid_line_keeps_the_tail() {
        let input = "/modify c:ma --steps 8";
        let (next, caret) = accept(input, "c:mara", Span { start: 8, end: 12 });
        assert_eq!(next, "/modify c:mara --steps 8");
        // The caret lands immediately after the inserted label — not past the
        // separator that was already there. Fixing a reference mid-line
        // should leave the caret on the word just fixed.
        assert_eq!(&next[..caret], "/modify c:mara");
    }

    #[test]
    fn accepting_does_not_double_the_separator() {
        let input = "/modify c:ma something";
        let (next, _) = accept(input, "c:mara", Span { start: 8, end: 12 });
        assert_eq!(next, "/modify c:mara something");
    }

    #[test]
    fn accepting_at_an_empty_span_inserts_at_the_caret() {
        let input = "/modify ";
        let (next, caret) = accept(input, "c:mara", Span::empty_at(input.len()));
        assert_eq!(next, "/modify c:mara ");
        assert_eq!(caret, next.len());
    }

    #[test]
    fn the_caret_is_a_valid_char_boundary_after_accepting() {
        // The bar converts this offset to UTF-16 for setSelectionRange, so a
        // mid-character offset would panic on the slice.
        let input = "/modify c:mara “a bob”";
        let (next, caret) = accept(input, "--steps", Span::empty_at(input.len()));
        assert!(next.is_char_boundary(caret));
        let _ = next[..caret].encode_utf16().count();
    }
}

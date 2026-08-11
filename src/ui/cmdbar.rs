//! The tokenized command bar: the composer as an editing surface.
//!
//! A transparent `<input>` sits exactly on top of a highlight layer that
//! paints the same text with per-token colour. The caret comes from the
//! input, the colour from the layer, and the two stay aligned because they
//! share font metrics and because **chips take their padding from
//! `box-shadow` spread rather than `padding`** — padding would change the
//! text's advance width and drift the caret away from the glyphs.
//!
//! Everything it knows about the language comes from
//! [`crate::registry::completion`]; this module only decides how it looks
//! and which key does what.
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

use dioxus::prelude::*;

use crate::registry::{
    completion::{self, Completion, CompletionKind, RefCandidate, TokenKind},
    slash::{self, Span},
};

/// The id the caret bridge binds to. One command bar at a time.
const BAR_ID: &str = "svs-cmdbar";

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

    // The caret bridge (see the module docs). Runs once after mount; the
    // script itself guards against double-binding.
    use_future(move || async move {
        let mut bridge = document::eval(CARET_BRIDGE);
        while let Ok((text, offset)) = bridge.recv::<(String, usize)>().await {
            if *value.peek() != text {
                value.set(text);
                selected.set(0);
                dismissed.set(false);
            }
            caret.set(offset);
        }
    });

    let text = value.read().clone();
    let position = (*caret.read()).min(text.len());
    let known: Vec<String> = refs.iter().map(|item| item.name.clone()).collect();

    let segments = paint(&text, &known);
    let completions = completion::complete(&text, position, &refs);
    let help = completion::signature(&text, position);
    let showing = *focused.read() && !completions.is_empty() && !*dismissed.read() && !disabled;
    let index = (*selected.read()).min(completions.items.len().saturating_sub(1));

    let replace = completions.replace;
    let count = completions.items.len();
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

            // signature help
            div { class: "flex min-h-[18px] items-center gap-2 px-1 pb-1 font-mono text-[10px] text-zinc-600",
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
                } else {
                    span { class: "text-zinc-700",
                        "type / for the operations, or describe a change in plain language"
                    }
                }
            }

            // completion popup, opening upward
            if showing {
                div { class: "absolute bottom-full left-0 z-40 mb-1.5 w-[330px] overflow-hidden rounded-[10px] bg-[#17171d] shadow-[0_-18px_50px_rgba(0,0,0,.6)] ring-1 ring-white/[0.09]",
                    div { class: "px-3 pb-1 pt-2 text-[9.5px] uppercase tracking-[0.07em] text-zinc-600",
                        "{completions.head}"
                    }
                    div { class: "max-h-[260px] overflow-y-auto pb-1",
                        for (slot , item) in completions.items.iter().enumerate() {
                            CompletionRow {
                                key: "{item.label}",
                                item: item.clone(),
                                active: slot == index,
                                label: item.label.clone(),
                                onpick: move |label: String| {
                                    splice(value, caret, selected, &label, replace);
                                },
                            }
                        }
                    }
                    div { class: "border-t border-white/[0.06] px-3 py-1.5 text-[10px] text-zinc-600",
                        "⇥ complete · ⏎ run · ⇧⏎ newline"
                    }
                }
            }

            div { class: "flex items-start gap-2",
                span { class: "shrink-0 pt-px font-mono text-[13px] leading-5 text-violet-300", ">" }

                // The field is in flow and drives the height; the painted
                // layer is absolutely positioned over it. Both wrap at the
                // same width with the same font, so their line breaks agree —
                // which is why this can wrap at all. A single-line input
                // would have to scroll, and the layer would have to chase its
                // scrollLeft on every keystroke.
                div { class: "relative min-w-0 flex-1",
                    div { class: "pointer-events-none absolute inset-0 z-10 whitespace-pre-wrap break-words font-mono text-[12.5px] leading-5",
                        for (slot , segment) in segments.iter().enumerate() {
                            span { key: "{slot}", class: segment.class, "{segment.text}" }
                        }
                    }
                    textarea {
                        id: BAR_ID,
                        class: "relative z-20 block max-h-40 w-full resize-none overflow-y-auto bg-transparent font-mono text-[12.5px] leading-5 text-transparent caret-violet-400 outline-none placeholder:text-zinc-700 disabled:opacity-40",
                        rows: "1",
                        placeholder: "/modify c:<ref> …",
                        value: "{text}",
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
                                Key::Tab => {
                                    event.prevent_default();
                                    if let Some(label) = labels.get(index) {
                                        splice(value, caret, selected, label, replace);
                                    }
                                }
                                // Enter always runs. An exact-match popup must
                                // never swallow the submit — the hint says
                                // "⏎ run", so Tab is the only way to accept.
                                Key::Enter if !event.modifiers().contains(Modifiers::SHIFT) => {
                                    event.prevent_default();
                                    onsubmit.call(());
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
            }
        }
    }
}

#[component]
fn CompletionRow(
    item: Completion,
    active: bool,
    label: String,
    onpick: EventHandler<String>,
) -> Element {
    let (glyph, badge) = completion_badge(item.kind);
    rsx! {
        div {
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

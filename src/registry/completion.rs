//! Editor services over the slash vocabulary: token classification for
//! highlighting, context-aware completion, and signature help.
//!
//! This is the language-server half of [`crate::registry::slash`] — the same
//! tokens the parser consumes, described well enough for an editing surface
//! to paint them and to suggest what comes next. It knows nothing about the
//! UI: everything is `&str` in, plain data out, so it is testable without a
//! renderer.
//!
//! Three rules hold the design together:
//!
//! - **One tokenizer.** Classification runs over [`slash::tokenize`], never a
//!   second lexer, so the bar and the parser cannot disagree.
//! - **Spans, not strings.** Every result carries the byte range it applies
//!   to, so accepting a completion is a splice and highlighting is a paint.
//! - **Syntax here, resolution outside.** `c:mara` is a [`TokenKind::Ref`]
//!   whether or not `mara` exists; whether it resolves is the registry's
//!   question, and the caller overlays that.

use crate::registry::{
    slash::{self, op_spec, tokenize, OpSpec, Span, Token, OPS},
    ArtifactKind, VariantAxis,
};

/// What a token is, for painting. Position-sensitive: only the first token
/// can be an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    /// The leading `/op`. `known` is false for a command the parser will
    /// reject, so the bar can underline it before the user hits enter.
    Operation {
        known: bool,
    },
    /// A `c:` reference. Whether the target exists is not decided here.
    Ref,
    /// `--flag`. `accepted` is false when the current operation does not
    /// take it — a typo that would otherwise fail only on submit.
    Flag {
        accepted: bool,
    },
    /// `WxH`, shaped like a size (bounds are the parser's business).
    Size,
    Number,
    /// Free text, including anything quoted.
    Text,
}

/// A single suggestion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    /// The text that replaces [`Completions::replace`].
    pub label: String,
    /// Shown dimmed beside the label; may be empty.
    pub hint: String,
    pub kind: CompletionKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionKind {
    Operation,
    Ref,
    Flag,
    Value,
}

/// The suggestions for one cursor position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completions {
    /// Group heading for the popup ("operation", "artifact", "flags", …).
    pub head: &'static str,
    pub items: Vec<Completion>,
    /// The span an accepted label replaces. Empty (`start == end`) when the
    /// cursor sits at a fresh argument position rather than inside a word.
    pub replace: Span,
}

impl Completions {
    fn none() -> Self {
        Self {
            head: "",
            items: Vec::new(),
            replace: Span::empty_at(0),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// An artifact the caller is willing to offer as a `c:` target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefCandidate {
    pub name: String,
    /// Free-form, shown beside the name — kind, "not created", whatever the
    /// surface wants to say.
    pub hint: String,
}

impl RefCandidate {
    pub fn new(name: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            hint: hint.into(),
        }
    }
}

/// The grammar of the operation being typed, with the parameter the cursor
/// is on.
#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    pub op: &'static OpSpec,
    /// Index into `op.params`, or `None` past the last positional (in the
    /// trailing free text, or among flags).
    pub active: Option<usize>,
}

/// Suggested values for flags that take a closed or conventional set. Flags
/// absent from this list take free text and offer nothing.
fn flag_values(flag: &str) -> Vec<Completion> {
    let values: &[(&str, &str)] = match flag {
        // Not a closed set — the parser accepts any multiple of 32 in
        // 256..=2048 — but these are the sizes the workflow actually uses.
        "--size" => &[
            ("384x576", "draft"),
            ("512x768", "draft"),
            ("768x1152", ""),
            ("1024x1536", "full"),
        ],
        "--steps" => &[
            ("4", "rough"),
            ("8", ""),
            ("12", ""),
            ("16", ""),
            ("24", "slow"),
        ],
        "--multiplier" => &[("0.4", ""), ("0.6", ""), ("0.8", ""), ("1.0", "")],
        "--axis" => {
            return VariantAxis::ALL
                .iter()
                .map(|axis| Completion {
                    label: axis.as_str().to_owned(),
                    hint: String::new(),
                    kind: CompletionKind::Value,
                })
                .collect()
        }
        _ => &[],
    };
    values
        .iter()
        .map(|(label, hint)| Completion {
            label: (*label).to_owned(),
            hint: (*hint).to_owned(),
            kind: CompletionKind::Value,
        })
        .collect()
}

/// Whether text is shaped like `WxH` (or `W×H`). Shape only — the parser
/// enforces the bounds.
fn looks_like_size(text: &str) -> bool {
    let mut parts = text.split(['x', 'X', '×']);
    match (parts.next(), parts.next(), parts.next()) {
        (Some(w), Some(h), None) => {
            !w.is_empty()
                && !h.is_empty()
                && w.chars().all(|c| c.is_ascii_digit())
                && h.chars().all(|c| c.is_ascii_digit())
        }
        _ => false,
    }
}

/// Paints a tokenized line. One [`TokenKind`] per token, in order.
pub fn classify(tokens: &[Token]) -> Vec<TokenKind> {
    let op = tokens.first().and_then(|first| op_spec(&first.text));

    tokens
        .iter()
        .enumerate()
        .map(|(index, token)| {
            if index == 0 && token.text.starts_with('/') {
                return TokenKind::Operation {
                    known: op.is_some(),
                };
            }
            // Quoted text is content, never syntax — the same rule the
            // parser applies when splitting flags.
            if token.quoted {
                return TokenKind::Text;
            }
            if token.text.starts_with("c:") {
                return TokenKind::Ref;
            }
            if slash::is_flag_like(&token.text) {
                return TokenKind::Flag {
                    accepted: op.is_some_and(|op| op.flags.contains(&token.text.as_str())),
                };
            }
            if looks_like_size(&token.text) {
                return TokenKind::Size;
            }
            if !token.text.is_empty() && token.text.chars().all(|c| c.is_ascii_digit() || c == '.')
            {
                return TokenKind::Number;
            }
            TokenKind::Text
        })
        .collect()
}

/// Where the caret sits, in terms of the tokens around it.
struct Cursor<'a> {
    /// The token being typed, if the caret is inside or at the edge of one.
    partial: Option<&'a Token>,
    /// Tokens that end before the partial (or before the caret).
    before: Vec<&'a Token>,
}

fn locate<'a>(tokens: &'a [Token], cursor: usize) -> Cursor<'a> {
    // A caret touching either edge of a token is inside it. When two tokens
    // touch the caret (`ab|cd` cannot happen, but `ab| cd` puts the caret at
    // the end of one and before the next), the earlier one wins: you are
    // finishing a word, not starting the next.
    let partial = tokens
        .iter()
        .find(|token| token.span.contains(cursor) && token.span.start < cursor);
    let boundary = partial.map_or(cursor, |token| token.span.start);
    let before = tokens
        .iter()
        .filter(|token| token.span.end <= boundary)
        .collect();
    Cursor { partial, before }
}

/// Suggestions for `input` with the caret at byte offset `cursor`.
///
/// `refs` is the artifact vocabulary the surface wants to offer; pass an
/// empty slice to suppress reference completion entirely.
pub fn complete(input: &str, cursor: usize, refs: &[RefCandidate]) -> Completions {
    let cursor = cursor.min(input.len());
    let tokens = tokenize(input);
    let Cursor { partial, before } = locate(&tokens, cursor);

    let typed = partial.map_or("", |token| token.text.as_str());
    let replace = partial.map_or_else(|| Span::empty_at(cursor), |token| token.span);

    let filtered = |items: Vec<Completion>, head: &'static str| {
        let items: Vec<Completion> = items
            .into_iter()
            .filter(|item| item.label.starts_with(typed))
            .collect();
        if items.is_empty() {
            Completions::none()
        } else {
            Completions {
                head,
                items,
                replace,
            }
        }
    };

    // Position 0: the operation itself.
    if before.is_empty() {
        return filtered(
            OPS.iter()
                .map(|op| Completion {
                    label: op.name.to_owned(),
                    hint: op.summary.to_owned(),
                    kind: CompletionKind::Operation,
                })
                .collect(),
            "operation",
        );
    }

    let Some(op) = op_spec(&before[0].text) else {
        return Completions::none();
    };

    // Directly after a flag that takes a value.
    if let Some(previous) = before.last() {
        if !previous.quoted && op.flags.contains(&previous.text.as_str()) {
            let values = flag_values(&previous.text);
            if !values.is_empty() {
                return filtered(values, "value");
            }
            return Completions::none();
        }
    }

    // Typing a flag.
    if typed.starts_with('-') {
        return filtered(
            op.flags
                .iter()
                .map(|flag| Completion {
                    label: (*flag).to_owned(),
                    hint: String::new(),
                    kind: CompletionKind::Flag,
                })
                .collect(),
            "flags",
        );
    }

    // The first positional argument.
    let argument = before.len(); // 1 == first argument after the op
    if argument == 1 {
        return match op.params.first() {
            Some(&"c:<ref>") => {
                let bare = typed.strip_prefix("c:").unwrap_or(typed);
                let items: Vec<Completion> = refs
                    .iter()
                    .filter(|candidate| candidate.name.contains(bare))
                    .map(|candidate| Completion {
                        label: format!("c:{}", candidate.name),
                        hint: candidate.hint.clone(),
                        kind: CompletionKind::Ref,
                    })
                    .collect();
                if items.is_empty() {
                    Completions::none()
                } else {
                    Completions {
                        head: "artifact",
                        items,
                        replace,
                    }
                }
            }
            Some(&"<kind>") => filtered(
                ArtifactKind::ALL
                    .iter()
                    .map(|kind| Completion {
                        label: kind.as_str().to_owned(),
                        hint: String::new(),
                        kind: CompletionKind::Value,
                    })
                    .collect(),
                "kind",
            ),
            _ => Completions::none(),
        };
    }

    // Second positional of /resize is a size; everything else past the first
    // argument is free text, where suggesting words would only be noise.
    if argument == 2 && op.params.get(1) == Some(&"<WxH>") {
        return filtered(flag_values("--size"), "size");
    }

    Completions::none()
}

/// The grammar of the operation under the caret, if there is one.
pub fn signature(input: &str, cursor: usize) -> Option<Signature> {
    let cursor = cursor.min(input.len());
    let tokens = tokenize(input);
    let Cursor { partial, before } = locate(&tokens, cursor);

    // While the operation itself is still being typed there is no signature
    // to show — completion is covering that position.
    let name = if before.is_empty() {
        return None;
    } else {
        &before[0].text
    };
    let op = op_spec(name)?;

    // `before.len()` counts the operation, so argument 1 is params[0]. Flags
    // and their values do not advance the positional cursor.
    // Count only positional arguments: flags are skipped here, and the
    // tokens they consume as values are subtracted below.
    let positional = before
        .iter()
        .skip(1)
        .filter(|token| token.quoted || !slash::is_flag_like(&token.text))
        .count();

    let index = positional.saturating_sub(flag_value_count(&before, op));
    let active = (index < op.params.len()).then_some(index);
    let _ = partial;

    Some(Signature { op, active })
}

/// How many non-flag tokens were eaten as flag values, so they do not count
/// as positional arguments.
fn flag_value_count(before: &[&Token], op: &OpSpec) -> usize {
    let mut count = 0;
    for pair in before.windows(2) {
        let (flag, value) = (pair[0], pair[1]);
        if !flag.quoted
            && op.flags.contains(&flag.text.as_str())
            && (value.quoted || !slash::is_flag_like(&value.text))
        {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs() -> Vec<RefCandidate> {
        vec![
            RefCandidate::new("mara", "character"),
            RefCandidate::new("tam", "character"),
            RefCandidate::new("harbor_dock", "environment"),
        ]
    }

    fn labels(completions: &Completions) -> Vec<&str> {
        completions
            .items
            .iter()
            .map(|item| item.label.as_str())
            .collect()
    }

    // ---------------------------------------------------------------- spans

    #[test]
    fn tokens_carry_spans_into_the_original_input() {
        let input = "/modify c:mara give her a bob cut";
        let tokens = tokenize(input);
        for token in &tokens {
            // Every span must slice the original input, and for tokens with
            // no punctuation rewriting it must slice back to the same text.
            let slice = &input[token.span.start..token.span.end];
            assert_eq!(slice, token.text, "span did not round-trip");
        }
    }

    #[test]
    fn spans_survive_smart_punctuation_and_quotes() {
        // WebKit rewrites `--` to an em dash and straight quotes to curly;
        // the token text is normalized but the span still points at what
        // the user actually typed, so highlighting lands on the em dash.
        let input = "/modify c:mara “a bob cut” —steps 8";
        let tokens = tokenize(input);
        let texts: Vec<&str> = tokens.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, ["/modify", "c:mara", "a bob cut", "--steps", "8"]);

        let quoted = &tokens[2];
        assert!(quoted.quoted);
        assert_eq!(&input[quoted.span.start..quoted.span.end], "“a bob cut”");

        // The span covers the em dash as typed (3 bytes) plus `steps`, while
        // the text carries the normalized `--steps`. Span length and text
        // length deliberately differ — highlighting must follow the input.
        let flag = &tokens[3];
        assert_eq!(&input[flag.span.start..flag.span.end], "—steps");
        assert_eq!(flag.text, "--steps");
        assert_ne!(flag.span.len(), flag.text.len());
    }

    #[test]
    fn spans_are_ordered_and_disjoint() {
        let tokens = tokenize("  /variant   c:tam  \"in oilskins\"  --axis outfit ");
        let mut previous = 0;
        for token in &tokens {
            assert!(token.span.start >= previous, "spans overlap or reorder");
            assert!(token.span.end > token.span.start, "empty span");
            previous = token.span.end;
        }
    }

    // ----------------------------------------------------------- classify

    #[test]
    fn classify_paints_each_token() {
        let tokens = tokenize("/modify c:mara --steps 8 512x768 plain words");
        assert_eq!(
            classify(&tokens),
            vec![
                TokenKind::Operation { known: true },
                TokenKind::Ref,
                TokenKind::Flag { accepted: true },
                TokenKind::Number,
                TokenKind::Size,
                TokenKind::Text,
                TokenKind::Text,
            ]
        );
    }

    #[test]
    fn classify_marks_unknown_operations_and_rejected_flags() {
        let tokens = tokenize("/nope c:mara --axis outfit");
        let kinds = classify(&tokens);
        assert_eq!(kinds[0], TokenKind::Operation { known: false });

        // --axis is real, but /describe does not take it.
        let tokens = tokenize("/describe c:mara --axis outfit");
        let kinds = classify(&tokens);
        assert_eq!(kinds[2], TokenKind::Flag { accepted: false });

        let tokens = tokenize("/variant c:mara --axis outfit");
        let kinds = classify(&tokens);
        assert_eq!(kinds[2], TokenKind::Flag { accepted: true });
    }

    #[test]
    fn quoted_text_is_never_syntax() {
        let tokens = tokenize("/modify c:mara \"--steps\"");
        assert_eq!(classify(&tokens)[2], TokenKind::Text);
    }

    // ---------------------------------------------------------- completion

    #[test]
    fn empty_input_offers_every_operation() {
        let completions = complete("", 0, &refs());
        assert_eq!(completions.head, "operation");
        assert_eq!(completions.items.len(), OPS.len());
        assert!(completions.replace.is_empty());
    }

    #[test]
    fn partial_operation_filters_and_replaces_itself() {
        let completions = complete("/re", 3, &refs());
        assert_eq!(labels(&completions), ["/regenerate", "/resize"]);
        // Accepting must overwrite what was typed, not append to it.
        assert_eq!(completions.replace, Span { start: 0, end: 3 });
    }

    #[test]
    fn after_an_operation_offers_artifacts() {
        let input = "/modify ";
        let completions = complete(input, input.len(), &refs());
        assert_eq!(completions.head, "artifact");
        assert_eq!(labels(&completions), ["c:mara", "c:tam", "c:harbor_dock"]);
        assert!(completions.replace.is_empty());
    }

    #[test]
    fn partial_reference_matches_on_the_bare_name() {
        let input = "/modify c:ma";
        let completions = complete(input, input.len(), &refs());
        assert_eq!(labels(&completions), ["c:mara"]);
        assert_eq!(completions.replace, Span { start: 8, end: 12 });
    }

    #[test]
    fn create_offers_kinds_not_references() {
        let input = "/create ";
        let completions = complete(input, input.len(), &refs());
        assert_eq!(completions.head, "kind");
        assert!(labels(&completions).contains(&"character"));
        assert!(!labels(&completions).iter().any(|l| l.starts_with("c:")));
    }

    #[test]
    fn flags_are_scoped_to_the_operation() {
        let input = "/modify c:mara --";
        let completions = complete(input, input.len(), &refs());
        assert_eq!(completions.head, "flags");
        assert_eq!(
            labels(&completions),
            ["--mask", "--inpaint", "--size", "--steps", "--lora"]
        );

        // /describe takes none, so nothing is offered rather than a wrong list.
        let input = "/describe c:mara --";
        assert!(complete(input, input.len(), &refs()).is_empty());
    }

    #[test]
    fn flag_values_are_offered_after_the_flag() {
        let input = "/variant c:mara --axis ";
        let completions = complete(input, input.len(), &refs());
        assert_eq!(completions.head, "value");
        assert!(labels(&completions).contains(&"outfit"));

        let input = "/regenerate c:mara --size ";
        let completions = complete(input, input.len(), &refs());
        assert!(labels(&completions).contains(&"1024x1536"));
    }

    #[test]
    fn resize_offers_sizes_in_its_second_position() {
        let input = "/resize c:mara ";
        let completions = complete(input, input.len(), &refs());
        assert_eq!(completions.head, "size");
        assert!(labels(&completions).contains(&"512x768"));
    }

    #[test]
    fn free_text_offers_nothing() {
        // The instruction is prose; suggesting words there would be noise.
        let input = "/modify c:mara give her ";
        assert!(complete(input, input.len(), &refs()).is_empty());
    }

    #[test]
    fn unknown_operations_complete_nothing_downstream() {
        let input = "/nope ";
        assert!(complete(input, input.len(), &refs()).is_empty());
    }

    #[test]
    fn completing_mid_line_replaces_only_the_word_under_the_caret() {
        let input = "/modify c:ma --steps 8";
        // Caret at the end of `c:ma`, not at the end of the line.
        let completions = complete(input, 12, &refs());
        assert_eq!(labels(&completions), ["c:mara"]);
        assert_eq!(completions.replace, Span { start: 8, end: 12 });

        // Splicing the accepted label in keeps the tail intact.
        let mut spliced = input.to_owned();
        spliced.replace_range(
            completions.replace.start..completions.replace.end,
            &completions.items[0].label,
        );
        assert_eq!(spliced, "/modify c:mara --steps 8");
    }

    #[test]
    fn cursor_past_the_end_is_clamped() {
        // A stale cursor must not panic or slice outside the string.
        let completions = complete("/re", 9_999, &refs());
        assert_eq!(labels(&completions), ["/regenerate", "/resize"]);
    }

    // ----------------------------------------------------------- signature

    #[test]
    fn signature_tracks_the_active_parameter() {
        let input = "/modify ";
        let help = signature(input, input.len()).unwrap();
        assert_eq!(help.op.name, "/modify");
        assert_eq!(help.active, Some(0)); // c:<ref>

        let input = "/modify c:mara ";
        assert_eq!(signature(input, input.len()).unwrap().active, Some(1));
    }

    #[test]
    fn flags_do_not_advance_the_positional_cursor() {
        // --steps and its value sit between the ref and the instruction; the
        // active parameter must still be the instruction.
        let input = "/modify c:mara --steps 8 ";
        assert_eq!(signature(input, input.len()).unwrap().active, Some(1));
    }

    #[test]
    fn signature_is_absent_until_an_operation_is_typed() {
        assert!(signature("", 0).is_none());
        assert!(signature("/mod", 4).is_none());
        assert!(signature("/nope c:mara", 12).is_none());
    }
}

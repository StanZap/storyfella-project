//! Slash-command parsing: free text from the canvas prompt bar → typed
//! operations (the closed vocabulary from [`crate::registry::ops`]).
//!
//! Supported syntax (the `svs` CLI flags map 1:1):
//!
//! ```text
//! /create character Mia, a lighthouse keeper --name mia --size 512x768
//! /variant c:mia "in yellow rain gear" --axis outfit
//! /regenerate c:mia "make it warmer"
//! /modify c:mia "change her hair" --steps 25
//! /attach-lora c:mia character-main --multiplier 0.8
//! /lora-scale character-main 0.9
//! /lora-remove character-main
//! ```
//!
//! Double quotes group words into one argument, and inside them `\"` is a
//! literal quote — without that a description could never contain one. Every
//! flag takes exactly one value. Descriptions are free text, so flags may appear anywhere. `--size`
//! takes `WxH` (the `×` sign is accepted too) and must be within contract
//! bounds; without it, the artifact gets the kind default
//! ([`ArtifactKind::default_size`]).
//!
//! Smart punctuation is normalized because the composer's text input
//! (WebKit) autocorrects `--` to `—` and `"` to `“ ”`: em dashes become `--`,
//! en dashes `-`, curly quotes behave like straight ones. An unquoted token
//! starting with a dash is an option — a lone dash (a prose separator) stays
//! in the description, but an unknown option is rejected with the valid list
//! instead of silently becoming part of the description. Quoted text is
//! never treated as an option.

use std::{collections::HashMap, fmt, str::FromStr};

use clipforge_inference::sd::LoraSelection;

use crate::registry::{ops::Operation, pipeline::is_valid_size, ArtifactKind, Ref, VariantAxis};

/// A slash command that failed to parse; the message is user-facing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlashError(pub String);

impl fmt::Display for SlashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SlashError {}

/// Whether the input looks like a slash command (a leading `/`).
pub fn is_slash_command(input: &str) -> bool {
    input.trim_start().starts_with('/')
}

/// One operation's grammar: the shape the parser expects and the flags it
/// accepts.
///
/// This is the single source of truth for three consumers — the parser's
/// "unknown command" message, the command bar's signature help, and
/// completion. A flag added to a `parse_*` function without being added
/// here will not be offered, which is what the `flags_match_parsers` test
/// guards against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpSpec {
    /// Including the leading slash.
    pub name: &'static str,
    /// Positional parameters after the operation itself.
    pub params: &'static [&'static str],
    pub flags: &'static [&'static str],
    pub summary: &'static str,
}

pub const OPS: &[OpSpec] = &[
    OpSpec {
        name: "/create",
        params: &["<kind>", "<description>"],
        flags: &["--name", "--size", "--steps", "--lora"],
        summary: "new artifact",
    },
    OpSpec {
        name: "/variant",
        params: &["c:<ref>", "<description>"],
        flags: &["--name", "--axis", "--size", "--steps", "--lora"],
        summary: "new look from a base",
    },
    OpSpec {
        name: "/regenerate",
        params: &["c:<ref>", "[prompt]"],
        flags: &["--size", "--steps", "--lora"],
        summary: "reroll from the description",
    },
    OpSpec {
        name: "/set",
        params: &["c:<ref>"],
        flags: &["--steps", "--seed", "--size"],
        summary: "stored generation defaults; renders nothing",
    },
    OpSpec {
        name: "/resize",
        params: &["c:<ref>", "<WxH>"],
        flags: &[],
        summary: "set the artifact canvas",
    },
    OpSpec {
        name: "/modify",
        params: &["c:<ref>", "<instruction>"],
        flags: &["--size", "--steps", "--lora"],
        summary: "instruction edit against the description",
    },
    OpSpec {
        name: "/describe",
        params: &["c:<ref>", "<description>"],
        flags: &[],
        summary: "make a look canonical",
    },
    OpSpec {
        name: "/revert",
        params: &["c:<ref>"],
        flags: &[],
        summary: "activate the previous image",
    },
    OpSpec {
        name: "/attach-lora",
        params: &["c:<ref>", "<lora>"],
        flags: &["--multiplier"],
        summary: "bind an adapter to an artifact",
    },
    OpSpec {
        name: "/lora-scale",
        params: &["<lora>", "<multiplier>"],
        flags: &[],
        summary: "change an adapter's weight",
    },
    OpSpec {
        name: "/lora-remove",
        params: &["<lora>"],
        flags: &[],
        summary: "detach an adapter",
    },
];

/// Looks up an operation by name, with or without the leading slash.
pub fn op_spec(name: &str) -> Option<&'static OpSpec> {
    let name = name.strip_prefix('/').unwrap_or(name);
    OPS.iter().find(|op| &op.name[1..] == name)
}

/// The vocabulary as a user-facing list, for error messages.
fn op_list() -> String {
    let names: Vec<&str> = OPS.iter().map(|op| op.name).collect();
    match names.split_last() {
        Some((last, rest)) => format!("{}, or {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// Parses a slash command into a typed [`Operation`].
pub fn parse_slash(input: &str) -> Result<Operation, SlashError> {
    let tokens = tokenize(input);
    let Some(command) = tokens.first() else {
        return Err(SlashError("empty command".to_owned()));
    };
    let command = command
        .text
        .strip_prefix('/')
        .ok_or_else(|| SlashError(format!("{:?} does not start with /", command.text)))?;

    match command {
        "create" => parse_create(&tokens),
        "variant" => parse_variant(&tokens),
        "regenerate" => parse_regenerate(&tokens),
        "resize" => parse_resize(&tokens),
        "set" => parse_set(&tokens),
        "modify" => parse_modify(&tokens),
        "attach-lora" => parse_attach_lora(&tokens),
        "lora-scale" => parse_lora_scale(&tokens),
        "lora-remove" => parse_lora_remove(&tokens),
        "describe" => parse_describe(&tokens),
        "revert" => parse_revert(&tokens),
        other => Err(SlashError(format!(
            "unknown command /{other}; expected {}",
            op_list()
        ))),
    }
}

fn parse_create(tokens: &[Token]) -> Result<Operation, SlashError> {
    let kind = tokens.get(1).ok_or_else(|| {
        SlashError("create needs a kind: /create character <description>".to_owned())
    })?;
    let kind = ArtifactKind::from_str(&kind.text).map_err(SlashError)?;
    let (description, flags) = split_flags(tokens, 2, &["--name", "--size", "--steps", "--lora"])?;
    let name = flag_value(&flags, "--name").map(str::to_owned);
    let size = match flag_value(&flags, "--size") {
        Some(value) => Some(parse_size(value).map_err(SlashError)?),
        None => None,
    };
    let steps = match flag_value(&flags, "--steps") {
        Some(value) => Some(parse_steps(value).map_err(SlashError)?),
        None => None,
    };
    let mut loras = Vec::new();
    for value in flags.get("--lora").cloned().unwrap_or_default() {
        loras.push(parse_lora_flag(&value).map_err(SlashError)?);
    }
    if description.trim().is_empty() {
        return Err(SlashError("create needs a description".to_owned()));
    }
    Ok(Operation::Create {
        kind,
        description,
        name,
        size,
        steps,
        loras,
    })
}

fn parse_variant(tokens: &[Token]) -> Result<Operation, SlashError> {
    let target = parse_ref(tokens, "variant")?;
    let (description, flags) = split_flags(
        tokens,
        2,
        &["--name", "--axis", "--size", "--steps", "--lora"],
    )?;
    let name = flag_value(&flags, "--name").map(str::to_owned);
    let axis = match flag_value(&flags, "--axis") {
        Some(value) => Some(VariantAxis::from_str(value).map_err(SlashError)?),
        None => None,
    };
    let size = match flag_value(&flags, "--size") {
        Some(value) => Some(parse_size(value).map_err(SlashError)?),
        None => None,
    };
    let steps = match flag_value(&flags, "--steps") {
        Some(value) => Some(parse_steps(value).map_err(SlashError)?),
        None => None,
    };
    let mut loras = Vec::new();
    for value in flags.get("--lora").cloned().unwrap_or_default() {
        loras.push(parse_lora_flag(&value).map_err(SlashError)?);
    }
    if description.trim().is_empty() {
        return Err(SlashError("variant needs a description".to_owned()));
    }
    Ok(Operation::Variant {
        target,
        description,
        name,
        axis,
        size,
        steps,
        loras,
    })
}

fn parse_describe(tokens: &[Token]) -> Result<Operation, SlashError> {
    let target = parse_ref(tokens, "describe")?;
    let (description, _) = split_flags(tokens, 2, &[])?;
    if description.trim().is_empty() {
        return Err(SlashError(
            "describe needs a description: /describe c:<ref> <description>".to_owned(),
        ));
    }
    Ok(Operation::Describe {
        target,
        description,
    })
}

fn parse_regenerate(tokens: &[Token]) -> Result<Operation, SlashError> {
    let target = parse_ref(tokens, "regenerate")?;
    let (description, flags) = split_flags(tokens, 2, &["--size", "--steps", "--lora"])?;
    let prompt = if description.trim().is_empty() {
        None
    } else {
        Some(description)
    };
    let size = match flag_value(&flags, "--size") {
        Some(value) => Some(parse_size(value).map_err(SlashError)?),
        None => None,
    };
    let steps = match flag_value(&flags, "--steps") {
        Some(value) => Some(parse_steps(value).map_err(SlashError)?),
        None => None,
    };
    let mut loras = Vec::new();
    for value in flags.get("--lora").cloned().unwrap_or_default() {
        loras.push(parse_lora_flag(&value).map_err(SlashError)?);
    }
    Ok(Operation::Regenerate {
        target,
        prompt,
        size,
        steps,
        loras,
    })
}

fn parse_resize(tokens: &[Token]) -> Result<Operation, SlashError> {
    let target = parse_ref(tokens, "resize")?;
    let size = tokens
        .get(2)
        .filter(|token| token.quoted || !is_flag_like(&token.text))
        .ok_or_else(|| SlashError("resize needs a size: /resize c:<ref> WxH".to_owned()))?;
    let (width, height) = parse_size(&size.text).map_err(SlashError)?;
    if tokens.len() > 3 {
        let extra = &tokens[3].text;
        return Err(SlashError(format!(
            "unexpected argument {extra:?}; resize takes one size: /resize c:<ref> WxH"
        )));
    }
    Ok(Operation::Resize {
        target,
        width,
        height,
    })
}

/// `/set c:<ref> [--steps N|auto] [--seed N|random] [--size auto]`.
///
/// `auto` and `random` clear a stored default rather than setting one —
/// which is why the fields are `Option<Option<_>>`: absent means "leave it
/// alone", `Some(None)` means "unpin it".
fn parse_set(tokens: &[Token]) -> Result<Operation, SlashError> {
    let target = parse_ref(tokens, "set")?;
    let (extra, flags) = split_flags(tokens, 2, &["--steps", "--seed", "--size"])?;
    let reset_size = match flag_value(&flags, "--size") {
        Some("auto") => true,
        None => false,
        Some(_) => return Err(SlashError(
            "Use --size auto to inherit the project size, or /resize to generate at a new size."
                .into(),
        )),
    };
    if !extra.trim().is_empty() {
        return Err(SlashError(format!(
            "unexpected argument {:?}; set takes only options: /set c:<ref> [--steps N|auto] [--seed N|random] [--size auto]",
            extra.trim()
        )));
    }
    let steps = match flag_value(&flags, "--steps") {
        Some("auto") => Some(None),
        Some(value) => Some(Some(parse_steps(value).map_err(SlashError)?)),
        None => None,
    };
    let seed = match flag_value(&flags, "--seed") {
        Some("random") => Some(None),
        Some(value) => Some(Some(value.parse::<u64>().map_err(|_| {
            SlashError(format!(
                "invalid seed {value:?}; expected a number or `random`"
            ))
        })?)),
        None => None,
    };
    if steps.is_none() && seed.is_none() && !reset_size {
        return Err(SlashError(
            "set needs something to set: /set c:<ref> [--steps N|auto] [--seed N|random] [--size auto]"
                .to_owned(),
        ));
    }
    Ok(Operation::Set {
        target,
        reset_size,
        steps,
        seed,
    })
}

fn parse_modify(tokens: &[Token]) -> Result<Operation, SlashError> {
    let target = parse_ref(tokens, "modify")?;
    let (description, flags) = split_flags(tokens, 2, &["--size", "--steps", "--lora"])?;
    if description.trim().is_empty() {
        return Err(SlashError("modify needs a description".to_owned()));
    }
    let size = match flag_value(&flags, "--size") {
        Some(value) => Some(parse_size(value).map_err(SlashError)?),
        None => None,
    };
    let steps = match flag_value(&flags, "--steps") {
        Some(value) => Some(parse_steps(value).map_err(SlashError)?),
        None => None,
    };
    let mut loras = Vec::new();
    for value in flags.get("--lora").cloned().unwrap_or_default() {
        loras.push(parse_lora_flag(&value).map_err(SlashError)?);
    }
    Ok(Operation::Modify {
        target,
        description,
        size,
        steps,
        loras,
    })
}

fn parse_attach_lora(tokens: &[Token]) -> Result<Operation, SlashError> {
    let target = parse_ref(tokens, "attach-lora")?;
    let lora_token = tokens
        .get(2)
        .filter(|token| token.quoted || !is_flag_like(&token.text))
        .ok_or_else(|| {
            SlashError(
                "attach-lora needs an adapter: /attach-lora c:<ref> <lora-file> [--multiplier]"
                    .to_owned(),
            )
        })?;
    let (_, flags) = split_flags(tokens, 3, &["--multiplier"])?;
    let multiplier = match flag_value(&flags, "--multiplier") {
        Some(value) => Some(parse_multiplier(value).map_err(SlashError)?),
        None => None,
    };
    Ok(Operation::AttachLora {
        target,
        lora: lora_token.text.clone(),
        multiplier,
    })
}

fn parse_lora_scale(tokens: &[Token]) -> Result<Operation, SlashError> {
    let lora = tokens
        .get(1)
        .map(|token| token.text.clone())
        .ok_or_else(|| {
            SlashError("lora-scale needs an adapter: /lora-scale <lora> <multiplier>".to_owned())
        })?;
    let raw = tokens.get(2).ok_or_else(|| {
        SlashError("lora-scale needs a multiplier: /lora-scale <lora> <multiplier>".to_owned())
    })?;
    let multiplier = parse_multiplier(&raw.text).map_err(SlashError)?;
    Ok(Operation::LoraScale { lora, multiplier })
}

fn parse_lora_remove(tokens: &[Token]) -> Result<Operation, SlashError> {
    let lora = tokens
        .get(1)
        .map(|token| token.text.clone())
        .ok_or_else(|| {
            SlashError("lora-remove needs an adapter: /lora-remove <lora>".to_owned())
        })?;
    Ok(Operation::LoraRemove { lora })
}

fn parse_multiplier(value: &str) -> Result<f32, String> {
    let multiplier = value
        .parse::<f32>()
        .map_err(|_| format!("invalid multiplier {value:?}"))?;
    if (-2.0..=2.0).contains(&multiplier) {
        Ok(multiplier)
    } else {
        Err(format!("multiplier {multiplier} must be within -2..=2"))
    }
}

fn parse_steps(value: &str) -> Result<u32, String> {
    let steps = value
        .parse::<u32>()
        .map_err(|_| format!("invalid steps {value:?}"))?;
    if (1..=50).contains(&steps) {
        Ok(steps)
    } else {
        Err(format!("steps {steps} must be within 1..=50"))
    }
}

/// `--lora <file>@<multiplier>` for the slash surface (one adapter per
/// flag; the CLI repeats the flag for more).
fn parse_lora_flag(value: &str) -> Result<LoraSelection, String> {
    let (path, multiplier) = match value.rsplit_once('@') {
        Some((path, multiplier)) => (
            path.to_owned(),
            multiplier
                .parse::<f32>()
                .map_err(|_| format!("invalid lora multiplier {multiplier:?}"))?,
        ),
        None => (value.to_owned(), 1.0),
    };
    if !(-2.0..=2.0).contains(&multiplier) {
        return Err(format!(
            "lora multiplier {multiplier} must be within -2..=2"
        ));
    }
    if path.is_empty() {
        return Err("--lora needs an adapter file: --lora <file>@<multiplier>".to_owned());
    }
    Ok(LoraSelection { path, multiplier })
}

/// `/revert c:<ref>` — activate the artifact's previous image.
fn parse_revert(tokens: &[Token]) -> Result<Operation, SlashError> {
    let target = tokens
        .get(1)
        .map(|token| Ref::new(token.text.clone()))
        .ok_or_else(|| SlashError("revert needs a target: /revert c:<ref>".to_owned()))?;
    Ok(Operation::Revert { target })
}

/// The token at index 1 is the `c:` ref (bare names and UUIDs resolve too).
fn parse_ref(tokens: &[Token], command: &str) -> Result<Ref, SlashError> {
    tokens
        .get(1)
        .map(|token| Ref::new(token.text.clone()))
        .ok_or_else(|| {
            SlashError(format!(
                "{command} needs a target: /{command} c:<ref> <description>"
            ))
        })
}

/// Splits `tokens[start..]` into (description, flags). Tokens in `valid`
/// are consumed with the token that follows as their value; any other
/// unquoted dash-prefixed token is rejected so a mistyped option can never
/// silently become part of the description.
fn split_flags(
    tokens: &[Token],
    start: usize,
    valid: &[&str],
) -> Result<(String, HashMap<String, Vec<String>>), SlashError> {
    let mut description = Vec::new();
    let mut flags: HashMap<String, Vec<String>> = HashMap::new();
    let mut index = start;
    while index < tokens.len() {
        let token = &tokens[index];
        if token.quoted || !is_flag_like(&token.text) {
            description.push(token.text.clone());
            index += 1;
            continue;
        }
        if !valid.contains(&token.text.as_str()) {
            return Err(SlashError(if valid.is_empty() {
                format!(
                    "unknown option {:?}; this command takes no options",
                    token.text
                )
            } else {
                format!(
                    "unknown option {:?}; expected one of: {}",
                    token.text,
                    valid.join(", ")
                )
            }));
        }
        let value = tokens.get(index + 1).ok_or_else(|| {
            SlashError(format!(
                "{} needs a value: {} <value>",
                token.text, token.text
            ))
        })?;
        if !value.quoted && is_flag_like(&value.text) {
            return Err(SlashError(format!(
                "{} needs a value: {} <value>",
                token.text, token.text
            )));
        }
        // Repeated flags accumulate (e.g. `--lora a --lora b`); every
        // other option is single-valued and simply keeps its last value.
        flags
            .entry(token.text.clone())
            .or_default()
            .push(value.text.clone());
        index += 2;
    }
    Ok((description.join(" "), flags))
}

/// The single value of a flag, when the command takes it at most once.
fn flag_value<'a>(flags: &'a HashMap<String, Vec<String>>, name: &str) -> Option<&'a str> {
    flags
        .get(name)
        .and_then(|values| values.first())
        .map(String::as_str)
}

/// Whether a token is an option attempt: starts with a dash and is not a
/// lone dash (a prose separator like `—` stays in the description).
pub(crate) fn is_flag_like(text: &str) -> bool {
    text.starts_with('-') && !text.trim_start_matches('-').is_empty()
}

/// Parses `WxH` (or `W×H`); bounds are enforced here so a bad size fails at
/// parse time instead of on the first regenerate.
fn parse_size(value: &str) -> Result<(u32, u32), String> {
    let normalized = value.replace('×', "x");
    let (width, height) = normalized
        .split_once('x')
        .ok_or_else(|| format!("expected WxH (e.g. 512x768), got {value:?}"))?;
    let width = width
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("invalid width {width:?}"))?;
    let height = height
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("invalid height {height:?}"))?;
    if !is_valid_size(width, height) {
        return Err(format!(
            "invalid size {width}x{height}: dimensions must be multiples of 32 within 256..=2048"
        ));
    }
    Ok((width, height))
}

/// A byte range into the original input.
///
/// Spans are what make an editing surface possible: highlighting, `c:`
/// chips, completion and error underlines all need to know *where* a token
/// sits, not just what it says. They index the input as typed — before
/// smart-punctuation normalization — so `—` occupies its own three bytes
/// even though its token text is `--`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn empty_at(offset: usize) -> Self {
        Self {
            start: offset,
            end: offset,
        }
    }

    /// Whether `cursor` sits inside the span or at either edge. Both edges
    /// count: a caret touching the end of a token is still "in" it, which is
    /// what typing at the end of a word means.
    pub fn contains(&self, cursor: usize) -> bool {
        cursor >= self.start && cursor <= self.end
    }

    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// One token of the command line.
#[derive(Clone, Debug)]
pub struct Token {
    pub text: String,
    /// Whether the token was wrapped in quotes (quoted tokens are content,
    /// never options, even when they start with a dash).
    pub quoted: bool,
    /// Where the token sits in the original input, quotes included.
    pub span: Span,
}

/// Splits on whitespace, grouping `"quoted segments"` (straight or curly
/// quotes) into single tokens. Smart punctuation is normalized: `—` → `--`,
/// `–` → `-`. Unbalanced quotes parse as plain text.
///
/// This is the only tokenizer. The command bar highlights with it rather
/// than carrying its own, so the two can never disagree about where a
/// token starts or whether it was quoted.
pub fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut in_quotes = false;
    let mut escaped = false;
    let mut start: Option<usize> = None;
    let mut end = 0usize;

    for (index, character) in input.char_indices() {
        let after = index + character.len_utf8();
        // Inside quotes a backslash escapes the next character, so a
        // description can contain a quote. Without this the language cannot
        // express `a "storm" coat` at all.
        if escaped {
            start.get_or_insert(index);
            end = after;
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' && in_quotes {
            start.get_or_insert(index);
            end = after;
            escaped = true;
            continue;
        }
        match character {
            '"' | '“' | '”' => {
                start.get_or_insert(index);
                end = after;
                if in_quotes {
                    in_quotes = false;
                    quoted = true;
                } else {
                    in_quotes = true;
                }
            }
            '—' => {
                start.get_or_insert(index);
                end = after;
                current.push_str("--");
            }
            '–' => {
                start.get_or_insert(index);
                end = after;
                current.push('-');
            }
            character if character.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    tokens.push(Token {
                        text: std::mem::take(&mut current),
                        quoted,
                        span: Span {
                            start: start.unwrap_or(index),
                            end,
                        },
                    });
                    quoted = false;
                }
                start = None;
            }
            character => {
                start.get_or_insert(index);
                end = after;
                current.push(character);
            }
        }
    }
    if !current.is_empty() {
        tokens.push(Token {
            text: current,
            quoted,
            span: Span {
                start: start.unwrap_or(end),
                end,
            },
        });
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ops::Operation;

    #[test]
    fn quotes_can_be_escaped_inside_a_quoted_argument() {
        // The inspector emits descriptions with `{:?}`, which escapes
        // embedded quotes; without escape handling the text came back
        // mangled as `\storm\`.
        let tokens = tokenize(r#"/describe c:mara "a \"storm\" coat""#);
        let texts: Vec<&str> = tokens.iter().map(|token| token.text.as_str()).collect();
        assert_eq!(texts, ["/describe", "c:mara", r#"a "storm" coat"#]);
        assert!(tokens[2].quoted);
    }

    #[test]
    fn a_backslash_outside_quotes_is_literal() {
        // Escaping applies only inside quotes; unquoted prose keeps its
        // backslashes rather than silently losing them.
        let tokens = tokenize(r"/create character a back\slash");
        let texts: Vec<&str> = tokens.iter().map(|token| token.text.as_str()).collect();
        assert_eq!(texts, ["/create", "character", "a", r"back\slash"]);
    }

    #[test]
    fn flags_match_parsers() {
        // Every flag advertised in OPS must be one the parser accepts.
        // Offering a flag that fails on submit would make completion a trap,
        // and the two lists live in different places, so pin them together.
        let base = |op: &str| match op {
            "/create" => "/create character a description",
            "/variant" => "/variant c:x a description",
            "/regenerate" => "/regenerate c:x",
            "/modify" => "/modify c:x a description",
            "/attach-lora" => "/attach-lora c:x adapter",
            "/set" => "/set c:x",
            other => panic!("{other} advertises flags but has no base command here"),
        };
        let value = |flag: &str| match flag {
            "--name" => "mia",
            "--size" => "512x768",
            "--steps" => "8",
            "--lora" => "adapter@0.8",
            "--axis" => "outfit",
            "--multiplier" => "0.8",
            "--seed" => "4471",
            other => panic!("no test value for {other}"),
        };

        for op in OPS {
            for flag in op.flags {
                let flag_value = if op.name == "/set" && *flag == "--size" {
                    "auto"
                } else {
                    value(flag)
                };
                let input = format!("{} {flag} {flag_value}", base(op.name));
                if let Err(error) = parse_slash(&input) {
                    assert!(
                        !error.0.starts_with("unknown option"),
                        "{} advertises {flag}, but the parser rejects it: {}",
                        op.name,
                        error.0
                    );
                }
            }
        }
    }

    #[test]
    fn every_operation_in_the_table_parses() {
        // A name in OPS that parse_slash does not dispatch would be offered
        // by completion and then fail as "unknown command".
        for op in OPS {
            let error = parse_slash(op.name).unwrap_err();
            assert!(
                !error.0.starts_with("unknown command"),
                "{} is in OPS but parse_slash does not handle it",
                op.name
            );
        }
    }

    #[test]
    fn create_parses_kind_description_and_optional_name() {
        assert_eq!(
            parse_slash("/create character Mia, a lighthouse keeper --name mia").unwrap(),
            Operation::Create {
                kind: ArtifactKind::Character,
                description: "Mia, a lighthouse keeper".to_owned(),
                name: Some("mia".to_owned()),
                size: None,
                steps: None,
                loras: Vec::new(),
            }
        );
        assert_eq!(
            parse_slash("/create environment \"storm harbor at night\"").unwrap(),
            Operation::Create {
                kind: ArtifactKind::Environment,
                description: "storm harbor at night".to_owned(),
                name: None,
                size: None,
                steps: None,
                loras: Vec::new(),
            }
        );
    }

    #[test]
    fn create_parses_an_explicit_size() {
        assert_eq!(
            parse_slash("/create character \"Mia\" --name mia --size 512x768").unwrap(),
            Operation::Create {
                kind: ArtifactKind::Character,
                description: "Mia".to_owned(),
                name: Some("mia".to_owned()),
                size: Some((512, 768)),
                steps: None,
                loras: Vec::new(),
            }
        );
        // The `×` sign (smart-punctuation autocorrect) is accepted too.
        assert_eq!(
            parse_slash("/create object lantern —size 768×768").unwrap(),
            Operation::Create {
                kind: ArtifactKind::Object,
                description: "lantern".to_owned(),
                name: None,
                size: Some((768, 768)),
                steps: None,
                loras: Vec::new(),
            }
        );
    }

    #[test]
    fn sizes_outside_contract_bounds_are_rejected() {
        let error = parse_slash("/create character Mia --size 100x100").unwrap_err();
        assert!(error.0.contains("multiples of 32"), "{error}");
        let error = parse_slash("/create character Mia --size 512x10").unwrap_err();
        assert!(error.0.contains("multiples of 32"), "{error}");
        let error = parse_slash("/create character Mia --size square").unwrap_err();
        assert!(error.0.contains("expected WxH"), "{error}");
    }

    #[test]
    fn smart_punctuation_is_normalized() {
        // WebKit text input autocorrects `--` to `—` and `"` to `“ ”`; the
        // parser must recover the intended command.
        assert_eq!(
            parse_slash("/create character —name mia “a woman wearing a hat”").unwrap(),
            Operation::Create {
                kind: ArtifactKind::Character,
                description: "a woman wearing a hat".to_owned(),
                name: Some("mia".to_owned()),
                size: None,
                steps: None,
                loras: Vec::new(),
            }
        );
        assert_eq!(
            parse_slash("/variant c:mia “older” —axis age").unwrap(),
            Operation::Variant {
                target: Ref::new("c:mia"),
                description: "older".to_owned(),
                name: None,
                axis: Some(VariantAxis::Age),
                size: None,
                steps: None,
                loras: Vec::new(),
            }
        );
    }

    #[test]
    fn variant_parses_an_explicit_name() {
        assert_eq!(
            parse_slash("/variant c:mia \"in yellow rain gear\" --name mia-rain").unwrap(),
            Operation::Variant {
                target: Ref::new("c:mia"),
                description: "in yellow rain gear".to_owned(),
                name: Some("mia-rain".to_owned()),
                axis: None,
                size: None,
                steps: None,
                loras: Vec::new(),
            }
        );
    }

    #[test]
    fn variant_parses_axis_and_quoted_description() {
        assert_eq!(
            parse_slash("/variant c:mia \"in yellow rain gear\" --axis outfit").unwrap(),
            Operation::Variant {
                target: Ref::new("c:mia"),
                description: "in yellow rain gear".to_owned(),
                name: None,
                axis: Some(VariantAxis::Outfit),
                size: None,
                steps: None,
                loras: Vec::new(),
            }
        );
        assert_eq!(
            parse_slash("/variant c:lantern rusted --axis weather").unwrap(),
            Operation::Variant {
                target: Ref::new("c:lantern"),
                description: "rusted".to_owned(),
                name: None,
                axis: Some(VariantAxis::Weather),
                size: None,
                steps: None,
                loras: Vec::new(),
            }
        );
    }

    #[test]
    fn regenerate_and_modify_parse() {
        assert_eq!(
            parse_slash("/regenerate c:mia make it warmer").unwrap(),
            Operation::Regenerate {
                target: Ref::new("c:mia"),
                prompt: Some("make it warmer".to_owned()),
                size: None,
                steps: None,
                loras: Vec::new(),
            }
        );
        assert_eq!(
            parse_slash("/modify c:mia \"change her hair\" --steps 25").unwrap(),
            Operation::Modify {
                target: Ref::new("c:mia"),
                description: "change her hair".to_owned(),
                size: None,
                steps: Some(25),
                loras: Vec::new(),
            }
        );
    }

    #[test]
    fn bare_targets_and_uuids_are_accepted_as_refs() {
        let parsed = parse_slash("/variant mia \"older\"").unwrap();
        assert_eq!(
            parsed,
            Operation::Variant {
                target: Ref::new("mia"),
                description: "older".to_owned(),
                name: None,
                axis: None,
                size: None,
                steps: None,
                loras: Vec::new(),
            }
        );
    }

    #[test]
    fn unknown_commands_and_kinds_are_rejected() {
        let error = parse_slash("/frobnicate c:mia").unwrap_err();
        assert!(error.0.contains("unknown command"), "{error}");
        let error = parse_slash("/create dragon green").unwrap_err();
        assert!(error.0.contains("unknown artifact kind"), "{error}");
    }

    #[test]
    fn revert_parses_to_a_target() {
        assert_eq!(
            parse_slash("/revert c:mia").unwrap(),
            Operation::Revert {
                target: Ref::new("c:mia"),
            }
        );
        assert!(parse_slash("/revert").unwrap_err().0.contains("target"));
    }

    #[test]
    fn missing_parts_are_rejected_with_guidance() {
        assert!(parse_slash("/create").unwrap_err().0.contains("kind"));
        assert!(parse_slash("/variant c:mia")
            .unwrap_err()
            .0
            .contains("description"));
        assert!(parse_slash("/modify c:mia")
            .unwrap_err()
            .0
            .contains("description"));
        assert!(parse_slash("/regenerate").unwrap_err().0.contains("target"));
    }

    #[test]
    fn flags_need_values() {
        let error = parse_slash("/variant c:mia older --axis").unwrap_err();
        assert!(error.0.contains("--axis needs a value"), "{error}");
        let error = parse_slash("/create character Mia --name").unwrap_err();
        assert!(error.0.contains("--name needs a value"), "{error}");
    }

    #[test]
    fn unknown_options_are_rejected_instead_of_becoming_description() {
        // The failure mode this guards: a mistyped flag silently creating an
        // artifact whose name/description swallow the flag text.
        let error = parse_slash("/create character Mia --nam mia").unwrap_err();
        assert!(error.0.contains("unknown option"), "{error}");
        assert!(error.0.contains("--name"), "{error}");
        // A single-dash typo (or an en-dash autocorrect) gets the same hint.
        let error = parse_slash("/variant c:mia older –axis outfit").unwrap_err();
        assert!(error.0.contains("unknown option"), "{error}");
        assert!(error.0.contains("--axis"), "{error}");
        // Regenerate accepts its own option set; anything else is rejected.
        let error = parse_slash("/regenerate c:mia --seed 7").unwrap_err();
        assert!(error.0.contains("unknown option"), "{error}");
        assert!(error.0.contains("--steps"), "{error}");
    }

    #[test]
    fn lone_dashes_and_quoted_dash_words_stay_in_descriptions() {
        // A prose separator is not an option.
        assert_eq!(
            parse_slash("/create environment storm harbor — at night").unwrap(),
            Operation::Create {
                kind: ArtifactKind::Environment,
                description: "storm harbor -- at night".to_owned(),
                name: None,
                size: None,
                steps: None,
                loras: Vec::new(),
            }
        );
        // Quoted dash-prefixed words are content, not options.
        assert_eq!(
            parse_slash("/create character \"--shadow\" --name umbra").unwrap(),
            Operation::Create {
                kind: ArtifactKind::Character,
                description: "--shadow".to_owned(),
                name: Some("umbra".to_owned()),
                size: None,
                steps: None,
                loras: Vec::new(),
            }
        );
    }

    #[test]
    fn repeated_lora_flags_accumulate() {
        let operation = parse_slash(
            "/create character Ada --lora characters/ada-v2.safetensors@0.8 --lora styles/noir.safetensors@1.0",
        )
        .unwrap();
        let Operation::Create { loras, .. } = operation else {
            panic!("expected create");
        };
        assert_eq!(
            loras,
            vec![
                LoraSelection {
                    path: "characters/ada-v2.safetensors".to_owned(),
                    multiplier: 0.8,
                },
                LoraSelection {
                    path: "styles/noir.safetensors".to_owned(),
                    multiplier: 1.0,
                },
            ]
        );
    }

    #[test]
    fn lora_commands_parse() {
        assert_eq!(
            parse_slash("/attach-lora c:mia character-main --multiplier 0.8").unwrap(),
            Operation::AttachLora {
                target: Ref::new("c:mia"),
                lora: "character-main".to_owned(),
                multiplier: Some(0.8),
            }
        );
        assert_eq!(
            parse_slash("/attach-lora c:mia character-main").unwrap(),
            Operation::AttachLora {
                target: Ref::new("c:mia"),
                lora: "character-main".to_owned(),
                multiplier: None,
            }
        );
        assert_eq!(
            parse_slash("/lora-scale character-main 0.9").unwrap(),
            Operation::LoraScale {
                lora: "character-main".to_owned(),
                multiplier: 0.9,
            }
        );
        assert_eq!(
            parse_slash("/lora-remove character-main").unwrap(),
            Operation::LoraRemove {
                lora: "character-main".to_owned(),
            }
        );
        // Bounds and structure are enforced.
        assert!(parse_slash("/attach-lora c:mia --multiplier 0.8")
            .unwrap_err()
            .0
            .contains("adapter"));
        assert!(parse_slash("/lora-scale character-main 5")
            .unwrap_err()
            .0
            .contains("within -2..=2"));
        assert!(parse_slash("/lora-remove")
            .unwrap_err()
            .0
            .contains("adapter"));
    }

    #[test]
    fn resize_parses_a_size_and_rejects_extra_arguments() {
        assert_eq!(
            parse_slash("/resize c:mia 1024x1024").unwrap(),
            Operation::Resize {
                target: Ref::new("c:mia"),
                width: 1024,
                height: 1024,
            }
        );
        assert!(parse_slash("/resize c:mia").unwrap_err().0.contains("WxH"));
        assert!(parse_slash("/resize c:mia 100x100")
            .unwrap_err()
            .0
            .contains("multiples of 32"));
        assert!(parse_slash("/resize c:mia 512x512 extra")
            .unwrap_err()
            .0
            .contains("unexpected argument"));
    }

    #[test]
    fn visual_ops_parse_size_steps_and_lora_flags() {
        assert_eq!(
            parse_slash(
                "/regenerate c:mia warmer --size 1024x1024 --steps 8 --lora styles/noir@0.8"
            )
            .unwrap(),
            Operation::Regenerate {
                target: Ref::new("c:mia"),
                prompt: Some("warmer".to_owned()),
                size: Some((1024, 1024)),
                steps: Some(8),
                loras: vec![LoraSelection {
                    path: "styles/noir".to_owned(),
                    multiplier: 0.8,
                }],
            }
        );
        assert_eq!(
            parse_slash("/variant c:mia older --size 768x768").unwrap(),
            Operation::Variant {
                target: Ref::new("c:mia"),
                description: "older".to_owned(),
                name: None,
                axis: None,
                size: Some((768, 768)),
                steps: None,
                loras: Vec::new(),
            }
        );
        // Bounds are enforced at parse time.
        assert!(parse_slash("/regenerate c:mia warmer --steps 99")
            .unwrap_err()
            .0
            .contains("1..=50"));
    }

    #[test]
    fn non_command_input_is_rejected() {
        let error = parse_slash("hello there").unwrap_err();
        assert!(error.0.contains("does not start with /"), "{error}");
        assert!(!is_slash_command("hello"));
        assert!(is_slash_command("  /create"));
    }
}

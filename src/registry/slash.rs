//! Slash-command parsing: free text from the canvas prompt bar → typed
//! operations (the closed vocabulary from [`crate::registry::ops`]).
//!
//! Supported syntax (the `svs` CLI flags map 1:1):
//!
//! ```text
//! /create character Mia, a lighthouse keeper --name mia --size 512x768
//! /variant c:mia "in yellow rain gear" --axis outfit
//! /regenerate c:mia "make it warmer"
//! /modify c:mia "change her hair" --mask "her hair" --inpaint "a bob cut"
//! ```
//!
//! Double quotes group words into one argument; every flag takes exactly one
//! value. Descriptions are free text, so flags may appear anywhere. `--size`
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
        "modify" => parse_modify(&tokens),
        other => Err(SlashError(format!(
            "unknown command /{other}; expected /create, /variant, /regenerate, or /modify"
        ))),
    }
}

fn parse_create(tokens: &[Token]) -> Result<Operation, SlashError> {
    let kind = tokens.get(1).ok_or_else(|| {
        SlashError("create needs a kind: /create character <description>".to_owned())
    })?;
    let kind = ArtifactKind::from_str(&kind.text).map_err(SlashError)?;
    let (description, flags) = split_flags(tokens, 2, &["--name", "--size"])?;
    let name = flags.get("--name").cloned();
    let size = match flags.get("--size") {
        Some(value) => Some(parse_size(value).map_err(SlashError)?),
        None => None,
    };
    if description.trim().is_empty() {
        return Err(SlashError("create needs a description".to_owned()));
    }
    Ok(Operation::Create {
        kind,
        description,
        name,
        size,
    })
}

fn parse_variant(tokens: &[Token]) -> Result<Operation, SlashError> {
    let target = parse_ref(tokens, "variant")?;
    let (description, axis) = split_flags(tokens, 2, &["--axis"])?;
    let axis = match axis.get("--axis") {
        Some(value) => Some(VariantAxis::from_str(value).map_err(SlashError)?),
        None => None,
    };
    if description.trim().is_empty() {
        return Err(SlashError("variant needs a description".to_owned()));
    }
    Ok(Operation::Variant {
        target,
        description,
        axis,
    })
}

fn parse_regenerate(tokens: &[Token]) -> Result<Operation, SlashError> {
    let target = parse_ref(tokens, "regenerate")?;
    let (description, _) = split_flags(tokens, 2, &[])?;
    let prompt = if description.trim().is_empty() {
        None
    } else {
        Some(description)
    };
    Ok(Operation::Regenerate { target, prompt })
}

fn parse_modify(tokens: &[Token]) -> Result<Operation, SlashError> {
    let target = parse_ref(tokens, "modify")?;
    let (description, flags) = split_flags(tokens, 2, &["--mask", "--inpaint"])?;
    if description.trim().is_empty() {
        return Err(SlashError("modify needs a description".to_owned()));
    }
    let mask_prompt = flags.get("--mask").cloned();
    let inpaint_prompt = flags.get("--inpaint").cloned();
    if mask_prompt.is_some() != inpaint_prompt.is_some() {
        return Err(SlashError(
            "modify needs both --mask and --inpaint (or neither, when an LLM plans them)"
                .to_owned(),
        ));
    }
    Ok(Operation::Modify {
        target,
        description,
        mask_prompt,
        inpaint_prompt,
    })
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
) -> Result<(String, HashMap<String, String>), SlashError> {
    let mut description = Vec::new();
    let mut flags = HashMap::new();
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
        flags.insert(token.text.clone(), value.text.clone());
        index += 2;
    }
    Ok((description.join(" "), flags))
}

/// Whether a token is an option attempt: starts with a dash and is not a
/// lone dash (a prose separator like `—` stays in the description).
fn is_flag_like(text: &str) -> bool {
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

/// One token of the command line.
struct Token {
    text: String,
    /// Whether the token was wrapped in quotes (quoted tokens are content,
    /// never options, even when they start with a dash).
    quoted: bool,
}

/// Splits on whitespace, grouping `"quoted segments"` (straight or curly
/// quotes) into single tokens. Smart punctuation is normalized: `—` → `--`,
/// `–` → `-`. Unbalanced quotes parse as plain text.
fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut in_quotes = false;
    for character in input.trim().chars() {
        match character {
            '"' | '“' | '”' => {
                if in_quotes {
                    in_quotes = false;
                    quoted = true;
                } else {
                    in_quotes = true;
                }
            }
            '—' => current.push_str("--"),
            '–' => current.push('-'),
            character if character.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    tokens.push(Token {
                        text: std::mem::take(&mut current),
                        quoted,
                    });
                    quoted = false;
                }
            }
            character => current.push(character),
        }
    }
    if !current.is_empty() {
        tokens.push(Token {
            text: current,
            quoted,
        });
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ops::Operation;

    #[test]
    fn create_parses_kind_description_and_optional_name() {
        assert_eq!(
            parse_slash("/create character Mia, a lighthouse keeper --name mia").unwrap(),
            Operation::Create {
                kind: ArtifactKind::Character,
                description: "Mia, a lighthouse keeper".to_owned(),
                name: Some("mia".to_owned()),
                size: None,
            }
        );
        assert_eq!(
            parse_slash("/create environment \"storm harbor at night\"").unwrap(),
            Operation::Create {
                kind: ArtifactKind::Environment,
                description: "storm harbor at night".to_owned(),
                name: None,
                size: None,
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
            }
        );
        assert_eq!(
            parse_slash("/variant c:mia “older” —axis age").unwrap(),
            Operation::Variant {
                target: Ref::new("c:mia"),
                description: "older".to_owned(),
                axis: Some(VariantAxis::Age),
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
                axis: Some(VariantAxis::Outfit),
            }
        );
        assert_eq!(
            parse_slash("/variant c:lantern rusted --axis weather").unwrap(),
            Operation::Variant {
                target: Ref::new("c:lantern"),
                description: "rusted".to_owned(),
                axis: Some(VariantAxis::Weather),
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
            }
        );
        assert_eq!(
            parse_slash(
                "/modify c:mia \"change her hair\" --mask \"her hair\" --inpaint \"a bob cut\""
            )
            .unwrap(),
            Operation::Modify {
                target: Ref::new("c:mia"),
                description: "change her hair".to_owned(),
                mask_prompt: Some("her hair".to_owned()),
                inpaint_prompt: Some("a bob cut".to_owned()),
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
                axis: None,
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
    fn missing_parts_are_rejected_with_guidance() {
        assert!(parse_slash("/create").unwrap_err().0.contains("kind"));
        assert!(parse_slash("/variant c:mia")
            .unwrap_err()
            .0
            .contains("description"));
        assert!(parse_slash("/modify c:mia nothing --mask x")
            .unwrap_err()
            .0
            .contains("--inpaint"));
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
        // Commands without options reject flag-like tokens too.
        let error = parse_slash("/regenerate c:mia --seed 7").unwrap_err();
        assert!(error.0.contains("takes no options"), "{error}");
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
            }
        );
    }

    #[test]
    fn non_command_input_is_rejected() {
        let error = parse_slash("hello there").unwrap_err();
        assert!(error.0.contains("does not start with /"), "{error}");
        assert!(!is_slash_command("hello"));
        assert!(is_slash_command("  /create"));
    }
}

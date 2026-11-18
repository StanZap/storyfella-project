//! What a model's answer to a writing request means.
//!
//! Models answer in more shapes than the prompt asks for. An edit arrives as
//! a fenced block, sometimes with a sentence about what changed around it,
//! sometimes bare. A request that is not an edit — a greeting, a question
//! about the story, something the model will not do — is answered in prose.
//! Reasoning models put their thinking first in `<think>` tags. This module
//! turns any of those into an [`Reply`], and gives a UI the readable part of
//! an answer while it is still arriving.

/// A model's answer, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// Replacement source for the passage, and what the model said about it.
    Edit {
        source: String,
        note: Option<String>,
    },
    /// An answer in prose; the passage is not edited.
    Message(String),
}

/// What an answer shows while it streams.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preview {
    /// The model is still reasoning, before any answer text.
    pub reasoning: bool,
    /// The answer so far, without reasoning or fence lines.
    pub text: String,
}

const REASONING: [(&str, &str); 2] = [("<think>", "</think>"), ("<thinking>", "</thinking>")];

/// Reads a complete answer. `inline` says the passage is part of one line,
/// where a bare word or phrase is the edit rather than a message.
pub fn interpret(raw: &str, inline: bool) -> Reply {
    let (answer, _) = without_reasoning(raw);
    if let Some((source, outside)) = fenced(&answer) {
        let note = outside.trim();
        return Reply::Edit {
            source,
            note: (!note.is_empty()).then(|| note.to_owned()),
        };
    }
    if answer.trim().is_empty() || inline || looks_like_source(&answer) {
        return Reply::Edit {
            source: answer,
            note: None,
        };
    }
    Reply::Message(answer.trim().to_owned())
}

/// A complete answer without its reasoning, for a caller that reads its own
/// format from what is left.
pub fn without_thinking(raw: &str) -> String {
    without_reasoning(raw).0
}

/// The readable part of an answer that is still arriving.
pub fn preview(raw: &str) -> Preview {
    let (answer, reasoning) = without_reasoning(raw);
    let text = match fenced(&answer) {
        Some((source, _)) => source,
        // An opening fence whose language line has not finished yet.
        None if answer.trim_start().starts_with("```") => String::new(),
        None => answer,
    };
    Preview {
        reasoning: reasoning && text.trim().is_empty(),
        text,
    }
}

/// The answer with reasoning blocks removed, and whether a block is still
/// open — the model is thinking.
fn without_reasoning(raw: &str) -> (String, bool) {
    let mut text = raw.to_owned();
    for (open, close) in REASONING {
        while let Some(start) = text.find(open) {
            match text[start..].find(close) {
                Some(end) => text.replace_range(start..start + end + close.len(), ""),
                None => {
                    text.truncate(start);
                    return (text, true);
                }
            }
        }
        // Some servers strip the opening tag and send only the closing one.
        if let Some(end) = text.find(close) {
            text.replace_range(..end + close.len(), "");
        }
    }
    (text.trim_start_matches(['\n', '\r']).to_owned(), false)
}

/// The first fenced block's contents, and the text around it. An unclosed
/// fence runs to the end, as it does while streaming.
fn fenced(text: &str) -> Option<(String, String)> {
    let mut offset = 0;
    let mut open = None;
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with("```") {
            match open {
                None => open = Some((offset, offset + line.len())),
                Some((start, body)) => {
                    let source = text[body..offset].to_owned();
                    let outside = format!("{}{}", &text[..start], &text[offset + line.len()..]);
                    return Some((source, outside));
                }
            }
        }
        offset += line.len();
    }
    let (start, body) = open?;
    Some((text[body..].to_owned(), text[..start].to_owned()))
}

/// Whether unfenced text reads as Storyfella source rather than prose: most
/// of its lines are statements.
fn looks_like_source(text: &str) -> bool {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return false;
    }
    let statements = lines.iter().filter(|line| is_statement(line)).count();
    statements * 5 >= lines.len() * 3
}

fn is_statement(line: &str) -> bool {
    const KEYWORDS: [&str; 8] = [
        "scene", "beat", "goto", "end", "choice", "option", "include", "//",
    ];
    const KEYS: [&str; 8] = [
        "narrate:",
        "shot:",
        "mood:",
        "style:",
        "cast:",
        "character:",
        "location:",
        "prop:",
    ];
    let trimmed = line.trim_start();
    let word = trimmed.split_whitespace().next().unwrap_or("");
    if KEYWORDS.contains(&word) || trimmed.starts_with("//") {
        return true;
    }
    if KEYS.iter().any(|key| trimmed.starts_with(key)) {
        return true;
    }
    // Dialogue and thoughts sit inside a beat, so they are indented.
    let indented = line.len() > trimmed.len();
    let speaker = trimmed
        .split_once(':')
        .map(|(name, _)| name.trim_end_matches(" think"))
        .is_some_and(|name| {
            !name.is_empty()
                && name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
    indented && speaker
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENE: &str = "scene harbor\n  narrate: Night.\n  end\n";

    #[test]
    fn a_fenced_edit_keeps_what_was_said_around_it_as_a_note() {
        let raw = format!("I darkened the narration.\n\n```storyfella\n{SCENE}```\nAnything else?");
        assert_eq!(
            interpret(&raw, false),
            Reply::Edit {
                source: SCENE.into(),
                note: Some("I darkened the narration.\n\nAnything else?".into()),
            }
        );
    }

    #[test]
    fn bare_source_is_an_edit_and_prose_is_a_message() {
        assert_eq!(
            interpret(SCENE, false),
            Reply::Edit {
                source: SCENE.into(),
                note: None
            }
        );
        assert_eq!(
            interpret("Hello! What would you like to change?", false),
            Reply::Message("Hello! What would you like to change?".into())
        );
        // Prose that happens to contain a colon is still prose.
        assert!(matches!(
            interpret(
                "Note: the scene already ends.\nShall I add a choice?",
                false
            ),
            Reply::Message(_)
        ));
        // Inside one line, a phrase is the edit.
        assert!(matches!(interpret("Night falls", true), Reply::Edit { .. }));
        // An empty answer is an edit that will fail its check and be repaired.
        assert!(matches!(interpret("", false), Reply::Edit { .. }));
    }

    #[test]
    fn reasoning_is_never_part_of_the_answer() {
        let raw = format!("<think>The user wants it darker.</think>\n```\n{SCENE}```");
        assert_eq!(
            interpret(&raw, false),
            Reply::Edit {
                source: SCENE.into(),
                note: None
            }
        );
        assert_eq!(
            interpret("Plan the edit.</think>Hello!", false),
            Reply::Message("Hello!".into())
        );
    }

    #[test]
    fn a_streaming_answer_previews_only_its_readable_part() {
        assert_eq!(
            preview("<think>Considering the mood"),
            Preview {
                reasoning: true,
                text: String::new()
            }
        );
        assert_eq!(preview("Sure.\n```story").text, "");
        assert_eq!(
            preview("Sure.\n```storyfella\nscene harbor\n  narr").text,
            "scene harbor\n  narr"
        );
        assert_eq!(preview("Hello!").text, "Hello!");
    }
}

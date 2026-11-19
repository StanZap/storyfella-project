//! The readable part of a model's answer. Reasoning models put their thinking
//! first in `<think>` tags, and some servers strip the opening tag and send
//! only the closing one; neither is part of what the writer reads.

/// What an answer shows while it streams.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preview {
    /// The model is still reasoning, before any answer text.
    pub reasoning: bool,
    /// The answer so far, without reasoning.
    pub text: String,
}

const REASONING: [(&str, &str); 2] = [("<think>", "</think>"), ("<thinking>", "</thinking>")];

/// A complete answer without its reasoning.
pub fn without_thinking(raw: &str) -> String {
    without_reasoning(raw).0
}

/// The readable part of an answer that is still arriving.
pub fn preview(raw: &str) -> Preview {
    let (text, reasoning) = without_reasoning(raw);
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
        if let Some(end) = text.find(close) {
            text.replace_range(..end + close.len(), "");
        }
    }
    (text.trim_start_matches(['\n', '\r']).to_owned(), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasoning_is_never_part_of_the_answer() {
        assert_eq!(
            without_thinking("<think>The user wants it darker.</think>\nDone."),
            "Done."
        );
        assert_eq!(without_thinking("Plan the edit.</think>Hello!"), "Hello!");
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
        assert_eq!(preview("Hello!").text, "Hello!");
    }
}

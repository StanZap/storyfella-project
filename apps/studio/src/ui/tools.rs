//! The canvas tool rail — the canvas's non-command controls.
//!
//! Every other control in the app is a command (`src/ui/inspector.rs`); a
//! tool is different because it has *state between gestures* — which is why
//! the tools live in a rail of their own rather than beside the icon actions
//! in the chrome bar.
//!
//! The rail is built to grow. [`Tool`] carries every tool the canvas is
//! planned to have, each with its own icon, shortcut and tooltip, and the
//! ones without an implementation are listed as disabled rather than hidden:
//! a visible, dimmed tool tells you the canvas is going to have it, where an
//! absent one tells you nothing.

use crate::ui::icons::IconName;

/// A canvas tool. `Pointer` is the resting state — no tool active.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Pointer,
    /// Not yet implemented — see [`Tool::available`].
    Crop,
    Scale,
    Colour,
}

impl Tool {
    /// Every tool in rail order.
    pub const ALL: [Tool; 4] = [Tool::Pointer, Tool::Crop, Tool::Scale, Tool::Colour];

    /// Whether the tool does anything yet. Unavailable tools still appear in
    /// the rail, disabled, so the shape of the toolset is visible.
    pub fn available(&self) -> bool {
        matches!(self, Tool::Pointer)
    }

    /// The stable action name, matching the shortcut registry.
    pub fn action(&self) -> &'static str {
        match self {
            Tool::Pointer => "tool-pointer",
            Tool::Crop => "tool-crop",
            Tool::Scale => "tool-scale",
            Tool::Colour => "tool-colour",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Tool::Pointer => "Pointer",
            Tool::Crop => "Crop",
            Tool::Scale => "Scale",
            Tool::Colour => "Colour",
        }
    }

    pub fn icon(&self) -> IconName {
        match self {
            Tool::Pointer => IconName::Arrow,
            Tool::Crop => IconName::Crop,
            Tool::Scale => IconName::Scale,
            Tool::Colour => IconName::Colour,
        }
    }

    /// The single key that selects it, matching the shortcut registry.
    pub fn key(&self) -> char {
        match self {
            // `v` and `e` are already the artifact scope's new-variant and
            // toggle-history; a tool rail is not worth rebinding either.
            Tool::Pointer => 'p',
            Tool::Crop => 'c',
            // `t` for transform: `s` is the tail of the `g s` chord.
            Tool::Scale => 't',
            Tool::Colour => 'k',
        }
    }

    /// The tool a plan names, by its lowercase label.
    pub fn parse(name: &str) -> Option<Tool> {
        Tool::ALL
            .iter()
            .copied()
            .find(|tool| tool.label().eq_ignore_ascii_case(name))
    }

    /// The tool a key selects, if it selects one and that tool works.
    pub fn from_key(key: char) -> Option<Tool> {
        Tool::ALL
            .iter()
            .copied()
            .find(|tool| tool.key() == key && tool.available())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::shortcuts;

    #[test]
    fn every_tool_has_a_shortcut_row_that_matches_its_key() {
        for tool in Tool::ALL {
            let keys = shortcuts::keys(tool.action()).unwrap_or_else(|| {
                panic!("{} has no shortcut row", tool.action());
            });
            assert_eq!(
                keys,
                tool.key().to_string(),
                "{} advertises a different key than the rail binds",
                tool.action()
            );
        }
    }

    #[test]
    fn tool_keys_are_unique() {
        let mut keys: Vec<char> = Tool::ALL.iter().map(|tool| tool.key()).collect();
        keys.sort_unstable();
        let count = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), count, "two tools share a key");
    }

    #[test]
    fn unimplemented_tools_are_not_selectable_by_key() {
        assert_eq!(Tool::from_key('p'), Some(Tool::Pointer));
        assert_eq!(
            Tool::from_key('c'),
            None,
            "crop is listed but not implemented, so its key must not select it"
        );
    }
}

//! The keyboard map — one table, read by both the handlers and the UI.
//!
//! A shortcut that exists in a `match` arm but not in any tooltip is a
//! shortcut nobody finds; one that appears in a tooltip but no handler is a
//! lie. Both happen when the binding and its label live apart, so they live
//! here together and every surface reads from this table:
//!
//! - tooltips, via [`tooltip`] — a control's label with its key appended;
//! - the status bar, via [`for_scope`] — the hints for wherever you are;
//! - the key handlers, which match on [`Shortcut::keys`] rather than on
//!   loose string literals.
//!
//! Adding a binding means adding a row. `no_orphan_bindings` fails if a
//! scoped shortcut has no label, so a row cannot be half-added.

/// Where a shortcut applies. A key can mean different things in different
/// scopes — `j` walks tiles in the grid and revisions in detail — which is
/// exactly why the scope is part of the record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Works anywhere.
    Global,
    /// The artifact grid.
    Grid,
    /// An artifact is open.
    Detail,
    /// The story editor.
    Writer,
}

impl Scope {
    pub fn label(&self) -> &'static str {
        match self {
            Scope::Global => "Anywhere",
            Scope::Grid => "Grid",
            Scope::Detail => "Artifact",
            Scope::Writer => "Story",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    /// The stable name a control refers to, e.g. `"regenerate"`.
    pub action: &'static str,
    /// As shown to a person: `⌃K`, `hjkl`, `[ ]`.
    pub keys: &'static str,
    /// What it does, in the imperative.
    pub label: &'static str,
    pub scope: Scope,
}

pub const SHORTCUTS: &[Shortcut] = &[
    // ------------------------------------------------------------- global
    Shortcut {
        action: "palette",
        keys: "⌃K",
        label: "Open the command palette",
        scope: Scope::Global,
    },
    Shortcut {
        action: "save",
        keys: "⌃S",
        label: "Save the project",
        scope: Scope::Global,
    },
    Shortcut {
        action: "goto-story",
        keys: "g s",
        label: "Go to the story",
        scope: Scope::Global,
    },
    Shortcut {
        action: "goto-canvas",
        keys: "g c",
        label: "Go to the canvas",
        scope: Scope::Global,
    },
    Shortcut {
        action: "focus-bar",
        keys: "/",
        label: "Jump to the command bar",
        scope: Scope::Global,
    },
    Shortcut {
        action: "undo",
        keys: "⌃Z",
        label: "Undo",
        scope: Scope::Global,
    },
    Shortcut {
        action: "redo",
        keys: "⌃⇧Z",
        label: "Redo",
        scope: Scope::Global,
    },
    // --------------------------------------------------------------- grid
    Shortcut {
        action: "move",
        keys: "hjkl",
        label: "Move between artifacts",
        scope: Scope::Grid,
    },
    Shortcut {
        action: "open",
        keys: "⏎",
        label: "Open the artifact",
        scope: Scope::Grid,
    },
    Shortcut {
        action: "new",
        keys: "n",
        label: "New character",
        scope: Scope::Grid,
    },
    // ------------------------------------------------------------- detail
    Shortcut {
        action: "back",
        keys: "esc",
        label: "Back to the grid",
        scope: Scope::Detail,
    },
    Shortcut {
        action: "variant-step",
        keys: "[ ]",
        label: "Previous / next variant",
        scope: Scope::Detail,
    },
    Shortcut {
        action: "new-variant",
        keys: "v",
        label: "New variant",
        scope: Scope::Detail,
    },
    Shortcut {
        action: "regenerate",
        keys: "r",
        label: "Regenerate",
        scope: Scope::Detail,
    },
    Shortcut {
        action: "modify",
        keys: "m",
        label: "Modify",
        scope: Scope::Detail,
    },
    Shortcut {
        action: "toggle-history",
        keys: "e",
        label: "Show or hide the history",
        scope: Scope::Detail,
    },
    Shortcut {
        action: "toggle-variants",
        keys: "f",
        label: "Show or hide the variants",
        scope: Scope::Detail,
    },
    // ------------------------------------------------------- canvas tools
    Shortcut {
        action: "tool-pointer",
        keys: "p",
        label: "Put the canvas tools away",
        scope: Scope::Detail,
    },
];

/// The shortcut for an action, if it has one.
pub fn keys(action: &str) -> Option<&'static str> {
    SHORTCUTS
        .iter()
        .find(|shortcut| shortcut.action == action)
        .map(|shortcut| shortcut.keys)
}

/// A control's tooltip: its own text, with the key appended when there is
/// one. `tooltip("Regenerate", "regenerate")` → `"Regenerate · r"`.
pub fn tooltip(text: &str, action: &str) -> String {
    match keys(action) {
        Some(keys) => format!("{text} · {keys}"),
        None => text.to_owned(),
    }
}

/// Every shortcut in a scope, for the status bar.
pub fn for_scope(scope: Scope) -> impl Iterator<Item = &'static Shortcut> {
    SHORTCUTS
        .iter()
        .filter(move |shortcut| shortcut.scope == scope)
}

/// A one-line hint strip: `hjkl move · ⏎ open · n new`.
pub fn hint_line(scope: Scope) -> String {
    for_scope(scope)
        .map(|shortcut| format!("{} {}", shortcut.keys, short_label(shortcut.label)))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The first word or two of a label, for the status bar where the full
/// sentence would not fit.
fn short_label(label: &str) -> String {
    label
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_are_unique() {
        // `keys()` returns the first match, so a duplicate action would make
        // one of the two silently unreachable.
        let mut seen: Vec<&str> = Vec::new();
        for shortcut in SHORTCUTS {
            assert!(
                !seen.contains(&shortcut.action),
                "duplicate action {:?}",
                shortcut.action
            );
            seen.push(shortcut.action);
        }
    }

    #[test]
    fn no_orphan_bindings() {
        // Every row must be complete: a key with no label cannot be shown to
        // anyone, and a label with no key is not a shortcut.
        for shortcut in SHORTCUTS {
            assert!(!shortcut.keys.is_empty(), "{} has no key", shortcut.action);
            assert!(
                !shortcut.label.is_empty(),
                "{} has no label",
                shortcut.action
            );
            assert!(!shortcut.action.is_empty(), "a shortcut has no action name");
        }
    }

    #[test]
    fn a_key_may_repeat_across_scopes_but_not_within_one() {
        // `j` legitimately means different things in the grid and in detail;
        // meaning two things in the *same* scope is a conflict.
        for scope in [Scope::Global, Scope::Grid, Scope::Detail, Scope::Writer] {
            let mut seen: Vec<&str> = Vec::new();
            for shortcut in for_scope(scope) {
                assert!(
                    !seen.contains(&shortcut.keys),
                    "{:?} binds {:?} twice",
                    scope,
                    shortcut.keys
                );
                seen.push(shortcut.keys);
            }
        }
    }

    #[test]
    fn tooltips_carry_the_key_when_there_is_one() {
        assert_eq!(tooltip("Regenerate", "regenerate"), "Regenerate · r");
        // An action with no binding still gets a usable tooltip.
        assert_eq!(
            tooltip("Attach an adapter", "attach-lora"),
            "Attach an adapter"
        );
    }

    #[test]
    fn the_hint_line_reads_as_a_strip() {
        let hint = hint_line(Scope::Grid);
        assert!(hint.contains("hjkl move"));
        assert!(hint.contains("⏎ open"));
        assert!(hint.contains('·'));
    }
}

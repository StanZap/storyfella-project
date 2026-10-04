//! App-wide keyboard navigation.
//!
//! One listener, one place. The alternative — an `onkeydown` per component —
//! is how an app ends up with thirty handlers and no coherent answer to
//! "what does `j` do right now". Here the answer comes from one place: the
//! canvas has exactly two states, grid and detail, and which one it is in is
//! already recorded by whether an artifact is selected. No separate mode
//! enum, because a second source of truth could disagree with the first.
//!
//! ## Why this is a bridge rather than `onkeydown`
//!
//! A key press has to be ignored while a text field has focus — typing
//! `j` into the command bar must insert a `j`, not move the grid cursor.
//! Dioxus events do not expose the focused element, so the filter lives in
//! JavaScript, where `document.activeElement` does.
//!
//! The same message carries the grid's column count, read from the live
//! `grid-template-columns`. `j`/`k` move by a row, and only the browser
//! knows how many columns `auto-fill` produced at the current width.

use dioxus::prelude::*;

/// A key press that reached the app rather than a text field.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyPress {
    pub key: String,
    /// Columns the artifact grid is currently laid out in; 1 when no grid
    /// is on screen.
    pub columns: usize,
}

/// Streams key presses that no text field claimed.
///
/// `Ctrl`/`Cmd` chords are left alone so the global shortcuts (`Ctrl+K`,
/// `Ctrl+S`) keep working, and the navigation keys are prevented from
/// scrolling the page. The one exception is `Ctrl/Cmd+Z` (and `+Shift+Z`
/// for redo): undo is a whole-app action with a button in the header, so the
/// bridge claims it here rather than leaving it to the webview.
const KEY_BRIDGE: &str = r#"
    if (!window.__svsKeys) {
        window.__svsKeys = true;
        const NAV = ["j", "k", "h", "l", "g", "n", "/", "Enter", "Escape",
                     "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"];
        document.addEventListener("keydown", (event) => {
            const active = document.activeElement;
            const tag = active ? active.tagName : "";
            // A focused text field owns every key it receives.
            if (tag === "INPUT" || tag === "TEXTAREA" ||
                (active && active.isContentEditable)) {
                return;
            }
            if ((event.ctrlKey || event.metaKey) &&
                (event.key === "z" || event.key === "Z")) {
                event.preventDefault();
                dioxus.send([event.shiftKey ? "redo" : "undo", 1]);
                return;
            }
            if (event.metaKey || event.ctrlKey || event.altKey) {
                return;
            }
            if (!NAV.includes(event.key)) {
                return;
            }
            event.preventDefault();
            const grid = document.querySelector("[data-grid]");
            const columns = grid
                ? getComputedStyle(grid).gridTemplateColumns.split(" ").length
                : 1;
            dioxus.send([event.key, columns]);
        });
    }
"#;

/// A press plus a sequence number.
///
/// The number is what makes a repeat observable: two identical presses are
/// two different values, so a signal watching this fires for both. Without
/// it, holding `j` would move the cursor once.
pub type KeyEvent = (u64, KeyPress);

/// Installs the bridge and calls `on_key` for every press that reaches the
/// app.
///
/// Call this **once**. The listener is registered against `window`, so a
/// second bridge would find the guard already set and receive nothing;
/// screens observe the broadcast signal instead of installing their own.
pub fn use_keys(on_key: impl FnMut(KeyPress) + 'static) {
    // The future outlives the closure's caller, and `use_future` needs a
    // `FnMut` it can move repeatedly, so the handler lives behind a shared
    // cell rather than being captured directly.
    let handler = use_hook(|| {
        std::rc::Rc::new(std::cell::RefCell::new(
            Box::new(on_key) as Box<dyn FnMut(KeyPress)>
        ))
    });
    use_future(move || {
        let handler = handler.clone();
        async move {
            let mut bridge = document::eval(KEY_BRIDGE);
            while let Ok((key, columns)) = bridge.recv::<(String, usize)>().await {
                (handler.borrow_mut())(KeyPress {
                    key,
                    columns: columns.max(1),
                });
            }
        }
    });
}

/// Moves a cursor within `count` items laid out in `columns` columns.
///
/// Returns `None` for keys that are not movement. Clamps rather than
/// wrapping: wrapping in a grid means the cursor jumps across the screen,
/// which is disorienting when the tiles are pictures.
pub fn move_cursor(key: &str, cursor: usize, count: usize, columns: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let last = count - 1;
    let next = match key {
        "l" | "ArrowRight" => cursor.saturating_add(1).min(last),
        "h" | "ArrowLeft" => cursor.saturating_sub(1),
        "j" | "ArrowDown" => cursor.saturating_add(columns).min(last),
        "k" | "ArrowUp" => cursor.saturating_sub(columns),
        _ => return None,
    };
    Some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_movement_clamps_at_the_ends() {
        assert_eq!(move_cursor("l", 0, 9, 3), Some(1));
        assert_eq!(
            move_cursor("h", 0, 9, 3),
            Some(0),
            "must not wrap backwards"
        );
        assert_eq!(move_cursor("l", 8, 9, 3), Some(8), "must not wrap forwards");
    }

    #[test]
    fn vertical_movement_steps_by_a_row() {
        assert_eq!(move_cursor("j", 0, 9, 3), Some(3));
        assert_eq!(move_cursor("k", 4, 9, 3), Some(1));
        // A partial last row still lands on a real tile rather than past it.
        assert_eq!(move_cursor("j", 6, 8, 3), Some(7));
    }

    #[test]
    fn a_single_column_makes_j_and_k_step_one() {
        assert_eq!(move_cursor("j", 0, 4, 1), Some(1));
        assert_eq!(move_cursor("k", 3, 4, 1), Some(2));
    }

    #[test]
    fn non_movement_keys_and_empty_grids_are_ignored() {
        assert_eq!(move_cursor("Enter", 0, 9, 3), None);
        assert_eq!(move_cursor("x", 0, 9, 3), None);
        assert_eq!(move_cursor("j", 0, 0, 3), None, "no tiles, nowhere to go");
    }
}

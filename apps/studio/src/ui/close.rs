//! Closing the window or quitting asks first while work would be lost.
use gpui_kit::{prelude::*, *};

use super::Studio;
use crate::{commands, state::AppState};

/// Unsaved writing, unapplied description drafts, or an operation still
/// running would all be lost by quitting now.
fn needs_close_confirmation(state: &AppState) -> bool {
    state.has_unsaved_changes || state.has_description_drafts() || state.operation_in_progress()
}

impl Studio {
    /// Whether the window may close now. When work would be lost it asks
    /// instead, and quits itself once answered.
    pub(super) fn can_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.closing || !needs_close_confirmation(&self.state) {
            return true;
        }
        let busy = self.state.operation_in_progress();
        let (detail, buttons): (&str, &'static [&'static str]) = if busy {
            (
                "An operation is still running. Quitting stops it and loses its result.",
                &["Cancel", "Discard and Quit"],
            )
        } else {
            (
                "This story has changes that are not saved.",
                &["Cancel", "Discard and Quit", "Save and Quit"],
            )
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            "Quit Storyfella Studio?",
            Some(detail),
            buttons,
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let Ok(choice) = answer.await else {
                return;
            };
            this.update_in(cx, |studio, window, cx| match buttons[choice] {
                "Discard and Quit" => studio.quit(cx),
                "Save and Quit" => studio.save_and_quit(window, cx),
                _ => {}
            })
            .ok();
        })
        .detach();
        false
    }

    fn quit(&mut self, cx: &mut Context<Self>) {
        self.closing = true;
        cx.quit();
    }

    fn save_and_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(message) = self.state.apply_description_drafts() {
            self.report_message(message, cx);
            return;
        }
        if self.state.project_path.is_none() {
            self.save_as(window, cx, |studio, _, cx| studio.quit(cx));
            return;
        }
        match commands::save_project(&mut self.state) {
            Ok(()) => self.quit(cx),
            Err(error) => self.report(error, cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::needs_close_confirmation;
    use crate::state::AppState;

    #[test]
    fn quitting_protects_source_drafts_and_active_operations() {
        let mut state = AppState::default();
        assert!(!needs_close_confirmation(&state));
        state.set_story_source("// unsaved writing".into());
        assert!(needs_close_confirmation(&state));
        let mut state = AppState::default();
        state.set_description_draft(uuid::Uuid::new_v4(), "Unfinished description".into());
        assert!(needs_close_confirmation(&state));
        let mut state = AppState::default();
        state.begin_operation();
        assert!(needs_close_confirmation(&state));
    }
}

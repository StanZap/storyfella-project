//! The project's saved conversations: the column beside the transcript that
//! lists them newest first, and choosing, starting, renaming and deleting
//! them. The column can be resized and hidden; the transcript stays in view
//! beside it.
use design_system::{
    kits::chat::{Conversation as Row, ConversationList},
    ButtonKind, ContextMenu, Dialog, Glyph, Menu, MenuItem, TypeScale, Typography,
};
use gpui_kit::{prelude::*, *};
use storyfella_studio_core::{errors::ErrorCode, workflows::assistant::Conversation};

use super::super::Studio;

/// What the confirmation dialog asks.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Confirm {
    Delete(String),
    Clear,
}

impl Studio {
    /// Starts a new conversation, or goes back to the empty one shown.
    pub(in crate::ui) fn new_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.assistant.current().turns.is_empty() {
            let conversation = Conversation::new();
            let id = conversation.id.clone();
            self.assistant.conversations.insert(0, conversation);
            self.select_conversation(id, cx);
        }
        self.assistant
            .prompt
            .update(cx, |prompt, cx| prompt.focus(window, cx));
    }

    /// Shows the conversation `id`. A new conversation left empty goes.
    pub(in crate::ui) fn select_conversation(&mut self, id: String, cx: &mut Context<Self>) {
        let assistant = &mut self.assistant;
        assistant
            .conversations
            .retain(|conversation| conversation.id == id || !conversation.turns.is_empty());
        if !assistant
            .conversations
            .iter()
            .any(|conversation| conversation.id == id)
        {
            return;
        }
        assistant.current = id;
        let (turns, title) = {
            let shown = assistant.current();
            (shown.turns.len(), shown.title.clone())
        };
        assistant.transcript.reset(turns);
        assistant.transcript.scroll_to_end();
        assistant
            .title
            .update(cx, |field, cx| field.set_value(title, cx));
        assistant.open_activity.clear();
        assistant.notice = None;
        assistant.menu = None;
        cx.notify();
    }

    pub(super) fn toggle_conversations(&mut self, cx: &mut Context<Self>) {
        self.assistant.conversations_open = !self.assistant.conversations_open;
        cx.notify();
    }

    /// Renames the shown conversation.
    pub(super) fn rename_conversation(&mut self, title: String, cx: &mut Context<Self>) {
        let id = self.assistant.current.clone();
        self.assistant.current_mut().title = title;
        self.save_conversation(&id, cx);
        cx.notify();
    }

    /// Shows the conversation at `index` in the list with its title ready
    /// to edit.
    fn start_renaming(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.assistant.menu = None;
        if let Some(conversation) = self.assistant.conversations.get(index) {
            let id = conversation.id.clone();
            self.select_conversation(id, cx);
        }
        self.assistant
            .title
            .update(cx, |field, cx| field.edit(window, cx));
    }

    fn ask_to_confirm(&mut self, confirm: Confirm, cx: &mut Context<Self>) {
        self.assistant.menu = None;
        self.assistant.confirm = Some(confirm);
        self.assistant.confirm_open = true;
        cx.notify();
    }

    fn confirmed(&mut self, cx: &mut Context<Self>) {
        self.assistant.confirm_open = false;
        match self.assistant.confirm.clone() {
            Some(Confirm::Delete(id)) => self.delete_conversation(&id, cx),
            Some(Confirm::Clear) => self.clear_conversations(cx),
            None => {}
        }
        cx.notify();
    }

    /// Deletes the conversation `id`, stopping its reply if one runs. What
    /// the reply changed stays in the story.
    pub(in crate::ui) fn delete_conversation(&mut self, id: &str, cx: &mut Context<Self>) {
        let stored = self.assistant.store().map(|store| {
            store.and_then(|(store, project)| store.delete_conversation(&project, id))
        });
        if let Some(Err(error)) = stored {
            return self.fail(
                ErrorCode::AppConversation,
                format!("Could not delete the conversation: {error}"),
                cx,
            );
        }
        if self.assistant.running_in(id) {
            self.stop_assistant(cx);
        }
        self.assistant
            .conversations
            .retain(|conversation| conversation.id != id);
        if self.assistant.current == id {
            self.show_newest(cx);
        }
    }

    /// Deletes every saved conversation of the project, stopping a reply if
    /// one runs.
    fn clear_conversations(&mut self, cx: &mut Context<Self>) {
        let stored = self
            .assistant
            .store()
            .map(|store| store.and_then(|(store, project)| store.clear_conversations(&project)));
        if let Some(Err(error)) = stored {
            return self.fail(
                ErrorCode::AppConversation,
                format!("Could not clear the conversations: {error}"),
                cx,
            );
        }
        self.stop_assistant(cx);
        self.assistant.conversations.clear();
        self.show_newest(cx);
    }

    /// Shows the most recent conversation, or a new one when none is left.
    pub(super) fn show_newest(&mut self, cx: &mut Context<Self>) {
        if self.assistant.conversations.is_empty() {
            self.assistant.conversations.push(Conversation::new());
        }
        let id = self.assistant.conversations[0].id.clone();
        self.select_conversation(id, cx);
    }

    pub(super) fn render_conversation_column(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.theme;
        let assistant = &self.assistant;
        let now = now();
        let rows: Vec<Row> = assistant
            .conversations
            .iter()
            .map(|conversation| {
                let (group, age) = age(conversation.updated / 1000, now);
                let messages = match conversation.turns.len() {
                    0 => "empty".to_owned(),
                    1 => "1 message".to_owned(),
                    count => format!("{count} messages"),
                };
                Row {
                    title: conversation.title.clone().into(),
                    detail: format!("{age} · {messages}").into(),
                    group: group.into(),
                    running: assistant.running_in(&conversation.id),
                }
            })
            .collect();
        let selected = assistant
            .conversations
            .iter()
            .position(|conversation| conversation.id == assistant.current);
        let ids: Vec<String> = assistant
            .conversations
            .iter()
            .map(|conversation| conversation.id.clone())
            .collect();
        let menu = assistant.menu.and_then(|(index, at)| {
            let id = ids.get(index)?.clone();
            let (rename, delete) = (cx.entity().downgrade(), cx.entity().downgrade());
            let menu = Menu::new("conversation-menu", c)
                .item(MenuItem::new("Rename").on_click(move |window, cx| {
                    rename
                        .update(cx, |studio, cx| studio.start_renaming(index, window, cx))
                        .ok();
                }))
                .separator()
                .item(MenuItem::new("Delete…").on_click(move |_, cx| {
                    delete
                        .update(cx, |studio, cx| {
                            studio.ask_to_confirm(Confirm::Delete(id.clone()), cx)
                        })
                        .ok();
                }));
            Some((at, menu))
        });
        let saved = assistant
            .conversations
            .iter()
            .any(|conversation| !conversation.turns.is_empty());
        let (select, open, dismiss) = (
            cx.entity().downgrade(),
            cx.entity().downgrade(),
            cx.entity().downgrade(),
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(8.))
            .border_r_1()
            .border_color(c.hairline)
            .child(
                div()
                    .id("assistant-conversations")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        ContextMenu::new(
                            "assistant-conversations-context",
                            ConversationList::new("assistant-conversation-list", c)
                                .conversations(rows)
                                .selected(selected)
                                .on_select(move |index, _, cx| {
                                    if let Some(id) = ids.get(index).cloned() {
                                        select
                                            .update(cx, |studio, cx| {
                                                studio.select_conversation(id, cx)
                                            })
                                            .ok();
                                    }
                                })
                                .on_secondary(move |index, at, _, cx| {
                                    open.update(cx, |studio, cx| {
                                        studio.assistant.menu = Some((index, at));
                                        cx.notify();
                                    })
                                    .ok();
                                }),
                        )
                        .menu(menu)
                        .on_dismiss(move |_, cx| {
                            dismiss
                                .update(cx, |studio, cx| {
                                    studio.assistant.menu = None;
                                    cx.notify();
                                })
                                .ok();
                        }),
                    ),
            )
            .child(
                div().child(
                    c.button("assistant-clear", "Clear history…", ButtonKind::Quiet)
                        .disabled(!saved)
                        .on_click(cx.listener(|studio, _, _, cx| {
                            studio.ask_to_confirm(Confirm::Clear, cx)
                        })),
                ),
            )
    }

    /// The button in the assistant's header that shows or hides the column.
    pub(super) fn render_conversations_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.assistant.conversations_open;
        self.theme
            .icon_action(
                "assistant-conversations-toggle",
                Glyph::Sidebar,
                if open {
                    "Hide conversations"
                } else {
                    "Show conversations"
                },
                if open {
                    ButtonKind::Secondary
                } else {
                    ButtonKind::Quiet
                },
            )
            .on_click(cx.listener(|studio, _, _, cx| studio.toggle_conversations(cx)))
    }

    /// Asks before deleting a conversation or all of them, over the whole
    /// window.
    pub(super) fn render_conversation_confirm(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.theme;
        let assistant = &self.assistant;
        let saved = assistant
            .conversations
            .iter()
            .filter(|conversation| !conversation.turns.is_empty())
            .count();
        let (title, body, action) = match &assistant.confirm {
            Some(Confirm::Delete(id)) => {
                let title = assistant
                    .conversations
                    .iter()
                    .find(|conversation| conversation.id == *id)
                    .map_or_else(String::new, |conversation| conversation.title.clone());
                (
                    "Delete this conversation?",
                    format!(
                        "“{title}” is deleted for good. What the assistant changed stays in the story."
                    ),
                    "Delete",
                )
            }
            Some(Confirm::Clear) | None => (
                "Clear the history?",
                match saved {
                    1 => "The saved conversation is deleted for good.".to_owned(),
                    count => format!("All {count} saved conversations are deleted for good."),
                } + " What the assistant changed stays in the story.",
                "Clear history",
            ),
        };
        let close = cx.entity().downgrade();
        Dialog::new("conversation-confirm", title, c)
            .open(assistant.confirm_open)
            .width(px(380.))
            .child(div().type_scale(TypeScale::Small).child(body))
            .action(
                c.button(
                    "conversation-confirm-cancel",
                    "Cancel",
                    ButtonKind::Secondary,
                )
                .on_click(cx.listener(|studio, _, _, cx| {
                    studio.assistant.confirm_open = false;
                    cx.notify();
                })),
            )
            .action(
                c.button("conversation-confirm-go", action, ButtonKind::Danger)
                    .on_click(cx.listener(|studio, _, _, cx| studio.confirmed(cx))),
            )
            .on_dismiss(move |_, cx| {
                close
                    .update(cx, |studio, cx| {
                        studio.assistant.confirm_open = false;
                        cx.notify();
                    })
                    .ok();
            })
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// The heading a conversation is listed under, and how long ago it changed.
fn age(updated: u64, now: u64) -> (&'static str, String) {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    const WEEK: u64 = 7 * DAY;
    let since = now.saturating_sub(updated);
    let group = match since {
        0..DAY => "Past day",
        DAY..=WEEK => "Past week",
        _ => "Older",
    };
    let label = match since {
        0..MINUTE => "just now".to_owned(),
        MINUTE..HOUR => format!("{} min ago", since / MINUTE),
        HOUR..DAY => format!("{} h ago", since / HOUR),
        _ => match since / DAY {
            1 => "1 day ago".to_owned(),
            days => format!("{days} days ago"),
        },
    };
    (group, label)
}

#[cfg(test)]
mod tests {
    use super::age;

    #[test]
    fn conversations_are_grouped_by_how_long_ago_they_changed() {
        let now = 1_000_000;
        assert_eq!(age(now - 20, now), ("Past day", "just now".into()));
        assert_eq!(age(now - 600, now), ("Past day", "10 min ago".into()));
        assert_eq!(age(now - 3 * 3600, now), ("Past day", "3 h ago".into()));
        assert_eq!(age(now - 86_400, now), ("Past week", "1 day ago".into()));
        assert_eq!(age(now - 9 * 86_400, now), ("Older", "9 days ago".into()));
    }
}

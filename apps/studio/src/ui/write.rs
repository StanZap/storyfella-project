//! Write: the story's files on the left, the open file in the middle, and
//! its problems beneath it on request.
//!
//! The editor holds the text while someone types; [`AppState`] holds the
//! story. Each edit goes from the editor to the state through
//! [`Studio::changed`], and anything else that changes the story (undo, a
//! plan, the live MCP bridge, a rename) reaches the editor the next time
//! Write draws, by comparing the two.
use std::rc::Rc;

use design_system::{
    kits::code::{
        CodeEditor, CodeEditorEvent, DiagnosticsList, Problem, ProblemItem, Severity, Suggestion,
        SuggestionKind, Token, TokenKind,
    },
    ButtonKind, Dialog, Glyph, ListBox, ListRow, TextField, TextFieldEvent, Theme, TypeScale,
    Typography,
};
use gpui_kit::{prelude::*, *};
use storyfella_language::{CompletionKind, Position};

use super::Studio;
use crate::{
    state::AppState,
    status::{Check, FileDiagnostic},
    writing::{self, Highlight},
};

actions!(
    write,
    [GoToDefinition, RenameSymbol, FormatFile, ToggleProblems]
);

pub(super) const CONTEXT: &str = "Write";

pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("f12", GoToDefinition, Some(CONTEXT)),
        KeyBinding::new("f2", RenameSymbol, Some(CONTEXT)),
        KeyBinding::new("shift-alt-f", FormatFile, Some(CONTEXT)),
        KeyBinding::new(super::key!("shift-m"), ToggleProblems, Some(CONTEXT)),
    ]);
}

pub(super) struct Write {
    pub(super) editor: Entity<CodeEditor>,
    /// The file the editor shows.
    file: Option<String>,
    /// The story's files, refreshed when the story changes.
    files: Vec<String>,
    /// The story changed since the editor last matched it.
    stale: bool,
    /// The last compile, for problems in the open file.
    check: Option<Check>,
    problems_open: bool,
    problems_scroll: UniformListScrollHandle,
    files_scroll: UniformListScrollHandle,
    rename: Entity<TextField>,
    /// The symbol being renamed: its file and offset.
    renaming: Option<(String, usize)>,
    /// Where to put the caret once the editor shows its file.
    caret: Option<usize>,
}

impl Write {
    pub(super) fn new(theme: Theme, window: &mut Window, cx: &mut Context<Studio>) -> Self {
        let editor = cx.new(|cx| CodeEditor::new(theme, window, cx).label("Story source"));
        let studio = cx.entity().downgrade();
        editor.update(cx, |editor, cx| {
            editor.set_suggestions(
                Rc::new(move |text: &str, offset, cx: &mut App| {
                    studio
                        .upgrade()
                        .map(|studio| studio.read(cx).suggest(text, offset))
                        .unwrap_or_default()
                }),
                &[' ', ':'],
                cx,
            );
        });
        cx.subscribe_in(
            &editor,
            window,
            |studio, editor, event, _, cx| match event {
                CodeEditorEvent::Changed(text) => studio.edit_open_file(text.to_string(), cx),
                CodeEditorEvent::CursorMoved(offset) => {
                    let (line, character) = editor.read(cx).line_column(*offset, cx);
                    studio.caret = Position {
                        line: line as u32,
                        character: character as u32,
                    };
                    cx.notify();
                }
            },
        )
        .detach();
        let rename = cx.new(|cx| TextField::new("New name", theme, window, cx));
        cx.subscribe_in(&rename, window, |studio, _, event, window, cx| {
            if let TextFieldEvent::Submitted(name) = event {
                studio.finish_rename(name.to_string(), window, cx);
            }
        })
        .detach();
        Self {
            editor,
            file: None,
            files: Vec::new(),
            stale: true,
            check: None,
            problems_open: false,
            problems_scroll: UniformListScrollHandle::new(),
            files_scroll: UniformListScrollHandle::new(),
            rename,
            renaming: None,
            caret: None,
        }
    }

    /// The story changed; the editor catches up when Write next draws.
    pub(super) fn story_changed(&mut self) {
        self.stale = true;
    }

    pub(super) fn set_check(&mut self, check: Option<Check>, cx: &mut Context<Studio>) {
        self.check = check;
        self.push_problems(cx);
    }

    pub(super) fn set_theme(&mut self, theme: Theme, cx: &mut Context<Studio>) {
        self.editor
            .update(cx, |editor, cx| editor.set_theme(theme, cx));
        self.rename
            .update(cx, |field, cx| field.set_theme(theme, cx));
    }

    /// Moves the caret to a line and character of the open file.
    pub(super) fn place_caret(&mut self, position: Position, state: &AppState) {
        let Some(text) = self
            .file
            .as_deref()
            .and_then(|file| writing::source(state, file))
        else {
            return;
        };
        let source = storyfella_language::Source::new(text);
        let offset = source
            .line_span(position.line as usize)
            .map_or(text.len(), |line| {
                let column: usize = text[line.start..line.end]
                    .chars()
                    .take(position.character as usize)
                    .map(char::len_utf8)
                    .sum();
                line.start + column
            });
        self.caret = Some(offset);
    }

    /// Brings the editor up to date with the story: the open file, its text,
    /// its colors and any problems the last compile placed in it.
    pub(super) fn sync(&mut self, state: &AppState, window: &mut Window, cx: &mut Context<Studio>) {
        if !self.stale && self.caret.is_none() {
            return;
        }
        self.stale = false;
        self.files = writing::files(state, crate::preview::workspace(state).ok().as_ref());
        let file = writing::active(state);
        let text = file
            .as_deref()
            .and_then(|file| writing::source(state, file))
            .unwrap_or_default()
            .to_owned();
        let switched = file != self.file;
        self.file = file;
        let tokens = tokens(state, &text);
        let caret = self.caret.take();
        self.editor.update(cx, |editor, cx| {
            if switched || editor.text(cx).as_ref() != text {
                let kept = (!switched).then(|| editor.cursor(cx));
                editor.set_text(text.clone(), window, cx);
                if let Some(offset) = caret.or(kept) {
                    editor.go_to(offset.min(text.len()), window, cx);
                }
            } else if let Some(offset) = caret {
                editor.go_to(offset.min(text.len()), window, cx);
            }
            editor.set_tokens(tokens, cx);
        });
        self.push_problems(cx);
    }

    /// Underlines the last compile's problems in the open file, when the
    /// file still reads as it did when compiled.
    fn push_problems(&self, cx: &mut Context<Studio>) {
        let problems: Vec<Problem> = match (&self.check, &self.file) {
            (Some(check), Some(file)) => {
                let current = self.editor.read(cx).text(cx);
                if check.files.get(file).map(String::as_str) == Some(current.as_ref()) {
                    check
                        .diagnostics
                        .iter()
                        .filter(|found| &found.file == file)
                        .map(|found| {
                            let span = found.diagnostic.span;
                            Problem::new(
                                span.start..span.end,
                                severity(found.diagnostic.severity),
                                found.diagnostic.message.clone(),
                            )
                        })
                        .collect()
                } else {
                    return;
                }
            }
            _ => Vec::new(),
        };
        self.editor
            .update(cx, |editor, cx| editor.set_problems(&problems, cx));
    }

    #[cfg(test)]
    pub(super) fn check_for_tests(&self) -> Option<&Check> {
        self.check.as_ref()
    }

    #[cfg(test)]
    pub(super) fn renaming_for_tests(&self) -> bool {
        self.renaming.is_some()
    }

    fn diagnostics(&self) -> &[FileDiagnostic] {
        self.check
            .as_ref()
            .map_or(&[], |check| check.diagnostics.as_slice())
    }
}

fn severity(severity: storyfella_language::Severity) -> Severity {
    match severity {
        storyfella_language::Severity::Error => Severity::Error,
        storyfella_language::Severity::Warning => Severity::Warning,
        storyfella_language::Severity::Information => Severity::Info,
    }
}

fn tokens(state: &AppState, text: &str) -> Vec<Token> {
    writing::highlights(state, text)
        .into_iter()
        .map(|(span, highlight)| {
            let kind = match highlight {
                Highlight::Keyword => TokenKind::Keyword,
                Highlight::Text => TokenKind::String,
                Highlight::Comment => TokenKind::Comment,
                Highlight::Reference => TokenKind::Name,
                Highlight::Symbol => TokenKind::Function,
            };
            Token::new(span.start..span.end, kind)
        })
        .collect()
}

/// "first.sf 3:7", where a diagnostic is.
fn place(found: &FileDiagnostic, check: &Check) -> String {
    let position = check
        .files
        .get(&found.file)
        .and_then(|text| {
            storyfella_language::Source::new(text).position(found.diagnostic.span.start)
        })
        .unwrap_or_default();
    format!(
        "{} {}:{}",
        found.file,
        position.line + 1,
        position.character + 1
    )
}

impl Studio {
    /// The person typed in the editor.
    fn edit_open_file(&mut self, text: String, cx: &mut Context<Self>) {
        let Some(file) = self.write.file.clone() else {
            return;
        };
        if writing::source(&self.state, &file) == Some(text.as_str()) {
            return;
        }
        match writing::type_into(&mut self.state, &file, text, std::time::Instant::now()) {
            Ok(()) => self.changed(cx),
            Err(message) => self.report_message(message, cx),
        }
    }

    /// Completion for the open file while it reads `text`.
    pub(super) fn suggest(&self, text: &str, offset: usize) -> Vec<Suggestion> {
        let Some(file) = &self.write.file else {
            return Vec::new();
        };
        writing::completions(&self.state, file, text, offset)
            .into_iter()
            .map(|completion| {
                let (kind, detail) = match completion.kind {
                    CompletionKind::Keyword => (SuggestionKind::Keyword, "keyword"),
                    CompletionKind::Scene => (SuggestionKind::Name, "scene"),
                    CompletionKind::Flag => (SuggestionKind::Name, "flag"),
                    CompletionKind::Asset => (SuggestionKind::Value, "asset"),
                };
                Suggestion::new(
                    completion.label,
                    kind,
                    completion.replace.start..completion.replace.end,
                )
                .detail(detail)
            })
            .collect()
    }

    /// Shows `file` in the editor, with the caret at `offset`.
    pub(super) fn open_file(&mut self, file: &str, offset: Option<usize>, cx: &mut Context<Self>) {
        if let Err(message) = writing::open(&mut self.state, file) {
            self.report_message(message, cx);
            return;
        }
        self.write.caret = offset;
        self.write.story_changed();
        cx.notify();
    }

    fn caret_offset(&self, cx: &App) -> usize {
        let editor = self.write.editor.read(cx);
        editor.cursor(cx)
    }

    pub(super) fn go_to_definition(&mut self, cx: &mut Context<Self>) {
        let Some(file) = self.write.file.clone() else {
            return;
        };
        let found = writing::navigation(&self.state, &file, self.caret_offset(cx));
        match found.definition {
            Some(place) => self.open_file(&place.file, Some(place.offset), cx),
            None => {
                self.report_message("Put the caret on a name to go to where it is declared.", cx)
            }
        }
    }

    pub(super) fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.write.file.clone() else {
            return;
        };
        let offset = self.caret_offset(cx);
        let Some(name) = writing::navigation(&self.state, &file, offset).rename else {
            self.report_message("Put the caret on a scene, beat or flag to rename it.", cx);
            return;
        };
        self.write.renaming = Some((file, offset));
        self.write.rename.update(cx, |field, cx| {
            field.set_text(name, window, cx);
            field.set_error(None, cx);
            field.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn finish_rename(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((file, offset)) = self.write.renaming.clone() else {
            return;
        };
        match writing::rename(&mut self.state, &file, offset, &name) {
            Ok(()) => {
                self.write.renaming = None;
                self.write.caret = Some(offset);
                self.write.story_changed();
                self.changed(cx);
                self.write
                    .editor
                    .update(cx, |editor, cx| editor.focus(window, cx));
            }
            Err(message) => self
                .write
                .rename
                .update(cx, |field, cx| field.set_error(Some(message.into()), cx)),
        }
    }

    fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.write.renaming = None;
        self.write
            .editor
            .update(cx, |editor, cx| editor.focus(window, cx));
        cx.notify();
    }

    pub(super) fn format_open_file(&mut self, cx: &mut Context<Self>) {
        let Some(file) = self.write.file.clone() else {
            return;
        };
        match writing::format(&mut self.state, &file) {
            Ok(()) => {
                self.write.story_changed();
                self.changed(cx);
            }
            Err(message) => self.report_message(message, cx),
        }
    }

    pub(super) fn toggle_problems(&mut self, cx: &mut Context<Self>) {
        self.write.problems_open = !self.write.problems_open;
        cx.notify();
    }

    /// Asks for a new story file beside the entry and opens it.
    fn prompt_new_file(&mut self, cx: &mut Context<Self>) {
        let Some(folder) = self
            .state
            .story
            .path()
            .and_then(|path| path.parent())
            .map(std::path::Path::to_path_buf)
        else {
            return;
        };
        let answer = cx.prompt_for_new_path(&folder, Some("chapter.sf"));
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(Ok(Some(path))) = answer.await else {
                return;
            };
            let path = if path.extension().is_none() {
                path.with_extension("sf")
            } else {
                path
            };
            this.update(cx, |studio, cx| {
                match studio.state.create_additional_source(&path) {
                    Ok(()) => {
                        studio.write.story_changed();
                        studio.changed(cx);
                    }
                    Err(error) => studio.report_message(error.to_string(), cx),
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn render_write(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.write.sync(&self.state, window, cx);
        div()
            .key_context(CONTEXT)
            .size_full()
            .flex()
            .on_action(cx.listener(|studio, _: &GoToDefinition, _, cx| studio.go_to_definition(cx)))
            .on_action(
                cx.listener(|studio, _: &RenameSymbol, window, cx| studio.start_rename(window, cx)),
            )
            .on_action(cx.listener(|studio, _: &FormatFile, _, cx| studio.format_open_file(cx)))
            .on_action(cx.listener(|studio, _: &ToggleProblems, _, cx| studio.toggle_problems(cx)))
            .child(self.render_files(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(self.render_file_bar(cx))
                    .child(div().flex_1().min_h_0().child(self.write.editor.clone()))
                    .children(self.write.problems_open.then(|| self.render_problems(cx))),
            )
    }

    fn render_files(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.theme;
        let files = self.write.files.clone();
        let shown = self
            .write
            .file
            .as_ref()
            .and_then(|file| files.iter().position(|candidate| candidate == file));
        let errors: Vec<bool> = files
            .iter()
            .map(|file| {
                self.write.diagnostics().iter().any(|found| {
                    &found.file == file
                        && found.diagnostic.severity == storyfella_language::Severity::Error
                })
            })
            .collect();
        let rows = files.clone();
        let studio = cx.entity().downgrade();
        div()
            .w(px(200.))
            .flex_none()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(c.hairline)
            .bg(c.surface)
            .child(
                div()
                    .h(px(36.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .pl(px(12.))
                    .pr(px(4.))
                    .child(
                        div()
                            .flex_1()
                            .type_scale(TypeScale::Caption)
                            .text_color(c.text_muted)
                            .child("Story files"),
                    )
                    .child(
                        c.icon_action("new-file", Glyph::Plus, "New story file", ButtonKind::Quiet)
                            .on_click(cx.listener(|studio, _, _, cx| studio.prompt_new_file(cx))),
                    ),
            )
            .child(
                div().flex_1().min_h_0().px(px(4.)).child(
                    ListBox::new(
                        "story-files",
                        "Story files",
                        c,
                        &self.write.files_scroll,
                        files.len(),
                        move |index, _, _| {
                            let row = ListRow::new(rows[index].clone());
                            if errors[index] {
                                row.trailing(Glyph::Warning.render(c.error, px(12.)))
                            } else {
                                row
                            }
                        },
                    )
                    .highlighted(shown)
                    .on_activate(move |index, _, cx| {
                        let file = files[index].clone();
                        studio
                            .update(cx, |studio, cx| studio.open_file(&file, None, cx))
                            .ok();
                    }),
                ),
            )
    }

    fn render_file_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.theme;
        let file = self.write.file.clone().unwrap_or_default();
        let count = self
            .write
            .diagnostics()
            .iter()
            .filter(|found| found.diagnostic.severity != storyfella_language::Severity::Information)
            .count();
        div()
            .h(px(36.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(2.))
            .pl(px(12.))
            .pr(px(4.))
            .border_b_1()
            .border_color(c.hairline)
            .bg(c.surface)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .type_scale(TypeScale::Small)
                    .child(file),
            )
            .child(
                c.icon_action(
                    "definition",
                    Glyph::Reference,
                    "Go to definition (F12)",
                    ButtonKind::Quiet,
                )
                .on_click(cx.listener(|studio, _, _, cx| studio.go_to_definition(cx))),
            )
            .child(
                c.icon_action(
                    "rename",
                    Glyph::Pencil,
                    "Rename symbol (F2)",
                    ButtonKind::Quiet,
                )
                .on_click(cx.listener(|studio, _, window, cx| studio.start_rename(window, cx))),
            )
            .child(
                c.icon_action("format", Glyph::AlignLeft, "Format file", ButtonKind::Quiet)
                    .on_click(cx.listener(|studio, _, _, cx| studio.format_open_file(cx))),
            )
            .child(
                c.button(
                    "problems",
                    if count == 0 {
                        "No problems".to_owned()
                    } else {
                        format!("Problems {count}")
                    },
                    ButtonKind::Quiet,
                )
                .on_click(cx.listener(|studio, _, _, cx| studio.toggle_problems(cx))),
            )
    }

    fn render_problems(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.theme;
        let places: Vec<(String, usize)> = self
            .write
            .diagnostics()
            .iter()
            .map(|found| (found.file.clone(), found.diagnostic.span.start))
            .collect();
        let items: Vec<ProblemItem> = match &self.write.check {
            Some(check) => check
                .diagnostics
                .iter()
                .map(|found| {
                    ProblemItem::new(
                        severity(found.diagnostic.severity),
                        found.diagnostic.message.clone(),
                        place(found, check),
                    )
                })
                .collect(),
            None => Vec::new(),
        };
        let studio = cx.entity().downgrade();
        div()
            .h(px(160.))
            .flex_none()
            .border_t_1()
            .border_color(c.hairline)
            .bg(c.surface)
            .p(px(4.))
            .child(
                DiagnosticsList::new("problems", c, &self.write.problems_scroll, items)
                    .on_activate(move |index, _, cx| {
                        let (file, offset) = places[index].clone();
                        studio
                            .update(cx, |studio, cx| studio.open_file(&file, Some(offset), cx))
                            .ok();
                    }),
            )
    }

    /// The rename dialog, drawn over the whole window.
    pub(super) fn render_rename(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.theme;
        let studio = cx.entity().downgrade();
        let cancel = studio.clone();
        let dismiss = studio.clone();
        Dialog::new("rename-symbol", "Rename symbol", c)
            .open(self.write.renaming.is_some())
            .width(px(380.))
            .child(self.write.rename.clone())
            .action(
                c.button("cancel-rename", "Cancel", ButtonKind::Secondary)
                    .on_click(move |_, window, cx| {
                        cancel
                            .update(cx, |studio, cx| studio.cancel_rename(window, cx))
                            .ok();
                    }),
            )
            .action(
                c.button("apply-rename", "Rename", ButtonKind::Primary)
                    .on_click(move |_, window, cx| {
                        studio
                            .update(cx, |studio, cx| {
                                let name = studio.write.rename.read(cx).text(cx).to_string();
                                studio.finish_rename(name, window, cx);
                            })
                            .ok();
                    }),
            )
            .on_dismiss(move |window, cx| {
                dismiss
                    .update(cx, |studio, cx| studio.cancel_rename(window, cx))
                    .ok();
            })
    }
}

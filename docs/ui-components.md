# Studio UI components

Studio draws every control from the shared GPUI design system in
`../gpui-design-system/crates/components`. Use those components before writing
another one; when a case needs a variant, extend the component there so Studio,
the Player and Atelier stay alike. [DESIGN.md](../DESIGN.md) owns interaction
and visual rules; [ROADMAP](ROADMAP.md) owns planned work.

## Where Studio uses what

| Need | Component | Example in Studio |
| --- | --- | --- |
| One main action, its alternatives, quiet toolbar actions | `Theme::button(id, label, ButtonKind)` | Home's New story and Open… |
| An action shown only as a glyph | `Theme::icon_action(id, Glyph, label, kind)`: the label is its accessible name and tooltip | The workspace bar's Home, Commands and Settings |
| A short single choice | `SegmentedControl` over a `&'static [Segment]` | Write/Play/Board; Settings' appearance, autosave and artwork fit |
| A setting that applies at once | `Theme::switch(id, label, on)`, which draws its own label | Settings' diagnostics and separate-vision switches |
| A name or a search | `TextField::new` / `TextField::search`, emitting `TextFieldEvent` | Home's story name and recent-story search |
| A list to browse and open | `ListBox` of `ListRow`s, with `.focus`, `.highlighted`, `.on_activate` | Home's recent stories |
| A choice from a set the app knows | `ComboBox`, emitting `ComboSelection` | Settings' providers and models |
| A bounded number | `NumericControl` with a `NumericSpec` range and unit; it draws its own label | Autoplay delay and text size |
| A command anywhere | `CommandPalette` of `PaletteCommand`s, disabled where they cannot run | ⌘K / Ctrl+K |
| A decision | `Dialog` with actions, the primary one last | Home's unreadable story |
| A failure | `details::failure_indication` (glyph and the code's label) opening `details::details_dialog`; see DESIGN.md §2 | The status bar's and Settings' failures |
| Source text | `kits::code::CodeEditor`, given tokens, problems and suggestions by the host | Write's editor |
| Problems to jump to | `kits::code::DiagnosticsList` of `ProblemItem`s | Write's problems list |
| A conversation | `kits::chat::Transcript` over a `TranscriptState`, of `UserMessage`s and `AssistantMessage`s | The assistant column |
| Steps a reply took | `kits::chat::ToolActivity` of `ToolCall`s | An assistant reply's phases |
| A reply that failed | `AssistantMessage::error` with the code's label and `on_error_details` | An assistant reply |
| A prompt with options | `PromptBar` over a `TextArea`, with chips and a send/stop action | The assistant's prompt and scope |
| Changes to review | `kits::code::DiffView`, read-only, deciding each change | A suggestion under review in Write |

Window-level prompts (quitting with unsaved work, file pickers) use GPUI's
native `window.prompt`, `prompt_for_paths` and `prompt_for_new_path`.

## Theme

Colors, spacing and type sizes are `Theme` tokens (`surface`, `text_muted`,
`metrics.control`, `TypeScale::Small`, …). Studio picks `Theme::dark()` or
`Theme::light()` from the persisted appearance and follows the window for
System. Components that hold a theme (`TextField`, `ComboBox`,
`NumericControl`, `CommandPalette`, the playback panel) are given the new one
with their `set_theme`/`configure` when appearance changes.

## Testing a view

Window behaviour is tested with `#[gpui::test]` and a `VisualTestContext`: open
the window, focus Studio, send keystrokes, and read `Studio` state. See
[`apps/studio/src/ui/tests.rs`](../apps/studio/src/ui/tests.rs).

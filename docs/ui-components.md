# Studio reusable UI components

Use these building blocks before adding markup or another control. This is a
component catalog, not a backlog. [DESIGN.md](../DESIGN.md) owns interaction and
visual rules; [ROADMAP](ROADMAP.md) owns planned work. These are Dioxus desktop
components in `apps/studio/src/ui`; Atelier has its own GPUI vocabulary.

## Single-line inputs

Import from `super::inputs` inside a sibling UI module. Implementation:
[`inputs.rs`](../apps/studio/src/ui/inputs.rs). Composition is:

```text
TextInput → TextInputWithAction → SearchInput
```

| Component | Use | Required props |
| --- | --- | --- |
| `TextInput` | Plain single-line text and named form fields | `value: String`, `label: String`, `oninput: EventHandler<String>` |
| `TextInputWithAction` | Text with one icon action inside the right edge | TextInput's required props, `action_icon: IconName`, `action_label: String`, `onaction: EventHandler<MouseEvent>` |
| `SearchInput` | Filtering/search with one × action and Escape cancellation | TextInput's required props, `oncancel: EventHandler<()>` |

Common options:

| Prop | Default | Behavior |
| --- | --- | --- |
| `placeholder` | Empty; `Search…` for SearchInput | Visible hint; never replaces the accessible label. |
| `size` | `InputSize::Regular` | Regular is 36px high; Large is 48px. Width fills the parent. |
| `disabled` | `false` | Disables the native field and its trailing action. |
| `autofocus` | `false` | Focuses this field when mounted; does not steal focus on every render. |
| `id` | Generated once | Optional stable native input ID for an external label. Keep it fixed for the mounted component. |
| `controls` | None | Sets `aria-controls`, for example to identify a filtered list. |
| `title` | None | Optional native field tooltip. |

`TextInput` and `TextInputWithAction` also support `read_only`, `kind`
(`InputKind::Text` or `Search`) and an optional `onkeydown` handler. Read-only
text stays selectable; its action may still run, for example to copy a value.
`TextInputWithAction` adds `action_disabled` and `action_tooltip`; the action's
accessible name is always required and supplies its tooltip by default. Its
button is `type="button"`, so it cannot accidentally submit an enclosing form.
`TextInput`'s children are a trailing composition slot; prefer the named action
component for the standard one-icon pattern.

`SearchInput` selects the native search type and composes `TextInputWithAction`
with `IconName::Close`. Its × remains available for an empty query so a temporary
search row can still be dismissed. Clicking × or pressing Escape in the field or
on its button calls `oninput("")`, then `oncancel(())`. Escape is consumed locally.
`cancel_label` defaults to `Cancel search`; the tooltip adds `(Esc)`. The owner
decides whether cancellation closes the search row, and restores focus if it
does. Disabled searches do not emit cancellation. Browser-provided duplicate
search-clear buttons are suppressed by the shared stylesheet.

All three retain native typing, selection and editing undo through
[`use_text_value`](../apps/studio/src/ui/text_input.rs). They do not use a
controlled native `value` attribute: delayed state updates must not overwrite
newer native keystrokes. Programmatic values and initial values still synchronize.
Validation, persistence and filtering remain with the caller. Do not put provider
calls, project mutations or file reads inside these controls.

Examples below assume signals named `name`, `query` and `searching` in a UI
component, and imports from `super::{inputs::*, icons::IconName}`.

```rust
TextInput {
    id: "story-name",
    label: "Story name",
    value: name(),
    placeholder: "Story name",
    autofocus: true,
    oninput: move |value| name.set(value),
}

TextInputWithAction {
    label: "Story name",
    value: name(),
    oninput: move |value| name.set(value),
    action_icon: IconName::Close,
    action_label: "Clear story name",
    onaction: move |_| name.set(String::new()),
}

SearchInput {
    label: "Search projects",
    value: query(),
    placeholder: "Search names, story content or assets…",
    controls: "studio-project-list",
    size: InputSize::Large,
    autofocus: true,
    oninput: move |value| query.set(value),
    oncancel: move |_| searching.set(false), // owner also restores trigger focus
}
```

For a visible label, put a native `label` beside the component with `for` matching
its `id`; the New story form in [`projects.rs`](../apps/studio/src/ui/projects.rs)
is the reference. The required `label` prop supplies an accessible name in all
cases. Do not wrap an input/action pair inside one native label, since its button
is a separate interactive control. Shared input styling lives under `.studio-input`
in [`tailwind.css`](../apps/studio/tailwind.css); extend that surface for a new
variant rather than copying its border, focus or action-positioning rules.

These components are not replacements for multiline source editors, CodeMirror,
numeric steppers or specialized token/completion editors.

## Other shared building blocks

Unless listed otherwise, import from `super::components`:

| Component | Purpose / important options |
| --- | --- |
| `IconButton` | Shared named action. Required `label`, `icon`, `onclick`; supports `disabled`, `active`, `show_label`, `primary`, custom `tooltip`, `expanded` and `controls`. Its label supplies a tooltip by default. |
| `SearchSelect` / `SelectOption` | Searchable, keyboard-operated selection from discovered options; selected `value`, optional placeholder, `on_select`. The owner renders loading/unavailable states. Use a native select for a short fixed list. |
| `SettingRow` | Accessible setting label/description, control slot and optional last-row treatment. |
| `SectionHeading` | Consistent settings section eyebrow, title and description. |
| `StatusDot` | Shared `HealthStatus` color indicator; pair with text that explains the status. |
| `EmptyVisual` | Icon, title and description for an empty workspace region. |
| `RecoveryNotice` | Local recovery title/message, optional `alert`, action slot. |
| `ErrorDetailsButton` | Opens a supplied `ReportedError` directly in the shared technical report. |
| `ErrorToast` / `ErrorDetailsHost` | Workspace-level reporting hosts. Mount once at the shell; local surfaces use recovery feedback and Details. |
| `Dialog` (`super::dialog`) | Native modal containment, Escape, inert background and return focus. Required title, close handler and content. |
| `Icon` / `IconName` (`super::icons`) | Shared icon vocabulary; prefer a named IconButton for an interactive icon. |

The workspace sidebar toggle is the reference for `IconButton` with `expanded`
and `controls`: its accessible name/tooltip changes between Show sidebar and
Hide sidebar while the controlled navigation retains a stable ID.

Reuse [the text-value bridge](../apps/studio/src/ui/text_input.rs) directly only
for specialized native controls whose event behavior the input components do not
cover. When extending this vocabulary, update this catalog and use the component
in its requesting surface so its API has a real caller.

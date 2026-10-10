# Storyfella Studio design standard and UX audit

Atelier acceptance · 2026-10-10: the image workspace uses named icon actions for
export, canvas placement, edit mode, comparison and masking. Details and provenance
are disclosed on demand; the prompt has more space and the version strip is compact.
Isolated macOS checks exercised those controls and resizing to about 1058 × 732
logical pixels. A live Replicate forest-bookshop generation and evening edit were
saved as a portable project, original PNGs and approved artwork.
Provider keys now save in user app data on all desktop platforms. File tests cover
provider isolation, replacement, environment precedence, malformed storage and
Unix owner-only permissions. Keychain access and certificate setup are deferred.
Linux/Windows native interaction and assistive-technology checks remain unverified.

The follow-up [iteration acceptance](docs/atelier-iteration-acceptance.md)
completed two further real Replicate edits, including a branch from the original,
comparison, naming, full-resolution export and a four-reference saved project.
The follow-up fixes bind unsent prompts to each source and New image, save them
in project/recovery files, and restore the active source after restart. Saved
version names replace positional numbers, parent tooltips explain lineage, and
a named icon reuses the inspected prompt. Live macOS navigation and restart
checks pass; native Open was repeated successfully. The actual-size control
says 100%. First-time-user and cross-platform acceptance remain outstanding.
Local Qwen Image 2.1 generation/editing and Krea 2 Turbo generation now use
Clipforge's managed image client; Linux CUDA inference remains unverified here.

This is the central UI and UX reference for Studio, including the shared playback
surface used by Preview. Read it before changing a user-facing flow or component.
It defines required behavior, the visual vocabulary, and the findings from
the 2026-10-07 sweep. The remediation record in §9 describes the implementation
and its verification through 2026-10-10; the original observations remain in §7.
Requirements are ongoing review criteria, not a claim of accessibility certification.

[ROADMAP §12](docs/ROADMAP.md#12-suggested-implementation-order) owns delivery
status. This document owns interaction and presentation rules.
[The walkthrough](docs/ux-walkthrough.md) supplies authoring scenarios;
[the power-user guide](docs/studio-power-user-guide.md) documents current workflows
and the 2026-10-10 follow-up assessment, including still-unimplemented improvements;
[the earlier UI plan](docs/design/ui-plan.md) records prototype decisions and
implementation history. If an earlier prototype conflicts with this standard,
use this standard and update the affected documentation with the implementation.

## 1. Product principles

Studio helps a person write a story, create its artwork, and test its playback.
The current project, current selection, pending work, and saved state must be
understandable without knowing Rust, the CLI, or the backend process model.

- Keep feedback beside the action and object it explains.
- A control's label promises its immediate result. One activation reaches it.
- Protect drafts and completed work across failures, navigation, and shutdown.
- Make available actions discoverable with both a pointer and a keyboard.
- Share terminology, state, and behavior across equivalent surfaces.
- Show useful information first; technical context is optional detail.
- Keep routine guidance out of the workspace. Use concise labels, named icons,
  tooltips and disclosures; reserve visible supporting copy for decisions,
  errors and meaningful state.
- Ask for a decision when it prevents a meaningful mistake. Routine navigation,
  validation and reversible editing should remain fluid.
- Empty, loading, pending, offline, failed and complete are distinct states.
  Every actionable state has an understandable next step.

These are Studio's application of the established usability heuristics for
visibility, consistency, control, recognition, error prevention and recovery.
[Nielsen Norman Group](https://www.nngroup.com/articles/ten-usability-heuristics/)

## 2. Feedback and error handling

### Choose the right surface

| Situation | Required surface | Interaction |
| --- | --- | --- |
| Invalid input | Message beside the field | Retain input, identify the problem and give a valid example. |
| An action fails | Persistent message in its task or object | Explain impact and offer Retry, correction or a relevant destination. |
| A background event matters elsewhere | Nonblocking notification | One direct action to the object or complete report. |
| Successful routine action | Quiet local status | Acknowledge the result without interrupting work. |
| A decision blocks safe continuation | Dialog | Explain the choice and its consequences; use explicit action labels. |
| Optional diagnostics | Inline disclosure or one report panel/dialog | Open directly from the originating error; allow selection and copying. |

For the reported example, the required flow is **Status error → Details → full
report**, in one activation. An inline disclosure is also acceptable. A toast
must never serve as an intermediate navigation step. Seeing an error inline
must not generate another copy merely to expose its details.

The summary must say what happened, what was affected, and what to do next.
Show remediation before technical diagnostics. Codes remain stable and available
for support, but must not visually outrank the human explanation. Keep context
such as file paths and requests expandable, wrapped and selectable. A single
report component must serve both local and global entry points.

Do not expire unresolved actionable errors. Clearing a notification must not
remove the underlying task's failure state. Repeated failures must not overwrite
another report that the user is reading; associate reports with their operations.
Announce important asynchronous failures without stealing focus. Use a polite
status announcement for ordinary completion/progress, and reserve alerts for
urgent failures. [W3C status messages](https://www.w3.org/WAI/WCAG22/Understanding/status-messages.html)

Copy must report success only after the clipboard operation succeeds, and show
failure with a recovery option. Clicking selectable diagnostic text must not
unexpectedly copy it. A visible Copy report action is sufficient.

## 3. Navigation, overlays and keyboard behavior

Keep primary authoring destinations stable. Clearly distinguish project-wide
navigation, artifact selection, and local panels. Expose the selected destination
programmatically. Returning from artwork to its source must restore the caret;
returning to the gallery must preserve useful filter, selection and scroll state.

The palette is available wherever its shortcut is advertised, and has a visible
button/menu entry. Scope actions to the current task and offer navigation across
screens. An action that needs arguments should lead to a focused, populated
editor; it must not execute an incomplete command.

Shortcuts work while Studio has focus. Do not reserve OS-wide shortcuts for
ordinary Save or palette actions. Display Cmd on macOS and Ctrl on other
platforms. The shortcut registry, listener, UI hints and actual action must agree.
Test the event path, not just the list of bindings. Text entry and native form
controls own their editing keys. Navigation chords must have a timeout/cancel
path and must not consume unrelated keystrokes.

Tab moves focus. A completion may claim Tab only when a visible, valid suggestion
is available; Shift+Tab remains a way to move backward. Enter/Space activate
buttons using native semantics. Define completion acceptance separately for code
editing and command submission, and display the active convention.

### Dialog contract

Use a shared dialog primitive with a name, appropriate dialog semantics, initial
focus inside, contained Tab/Shift+Tab navigation, inert background, Escape to
close, a visible close/cancel control, and focus return to the invoking control.
While a dialog is active, background navigation and mutation shortcuts are
suspended. Keep long content scrollable within the available window. A dialog's
primary action names its consequence; destructive actions receive appropriate
confirmation. [W3C modal dialog pattern](https://www.w3.org/WAI/ARIA/apg/patterns/dialog-modal/)

A modal is justified for proposal approval or a consequential decision. Ordinary
status detail can use a local disclosure or nonmodal panel. Small popovers remain
anchored to their trigger and reposition to stay visible; long searchable pickers
may use a named dialog with Cancel and focus return to avoid viewport clipping.
Manage overlay
priority centrally: one active modal, no unrelated competing corner cards, and
no hidden backdrop that blocks unrelated controls.

## 4. Editing, persistence and background work

Show explicit Saved, Saving, Unsaved changes, and Save failed states. Distinguish
source, project artwork, settings and temporary inspector drafts. A dirty dot
alone cannot explain what remains unsaved. Autosave mode must have a documented
scope and persistence policy.

- Preserve submitted text until validation succeeds. Failed submissions remain
  editable; asynchronous failures offer an exact retry or recoverable history.
- Native typing, caret, selection and composition take precedence over delayed
  renders. Synchronize explicit app changes without overwriting newer keystrokes;
  exercise rapid typing as well as paste when reviewing an editor.
- Preserve per-artifact description drafts when switching artifacts/screens,
  or offer Save/Discard/Cancel before losing them.
- Protect dirty source and artwork on window close and Quit, including failed
  autosave and autosave Off/Every minute. Cancel must leave work intact.
- Apply the same protection to reload, project replacement and new-project flows.
- Keep undo scope visible. Text fields retain expected editing undo; project
  actions expose what they undo and keep source/registry invariants together.
- Setting changes distinguish applied-for-this-session from saved-to-disk. A
  write failure must remain visible with Retry; never imply persistence from a
  successful in-memory update.
- A control that starts generation must state that consequence before activation.
  Separate changing a future default from immediately rendering a new image.

Long work belongs to a persistent task owner, with its target, phase, progress
when known, and recoverable final outcome. Navigation must not lose a result or
leave the app permanently busy. Disable conflicting mutations; allow unrelated
reading and navigation. Describe cancellation truthfully: stopping a guide is
not cancelling an already running generation. If cancellation is unsupported,
state that and show the continuing task rather than hiding it.

For downloads, show total remaining bytes, destination, per-file progress,
verification, failure and retry. Retain partial files for supported resumption.
Missing local overrides are configuration problems, distinct from downloadable
files. Download completion does not mean runtime readiness; validate the entire
effective deployment before displaying Model ready.

## 5. Forms and action controls

Every input has a persistent visible label and a programmatic association.
Placeholders show examples, not the only label or a silently assumed value.
Messages identify the field and explain the accepted format and bounds. Keep
validation state per field; associate it with the control and preserve invalid
text for correction. [W3C labels and instructions](https://www.w3.org/WAI/WCAG22/Understanding/labels-or-instructions.html)

Show the effective value and whether it comes from a project override, global
setting or built-in default. Inherited values should read, for example,
“Default · 25 steps,” with a way to restore inheritance. Offer useful presets
without making the full valid range inaccessible. Use consistent dimensions and
units across Settings and Inspector.

Use native controls when they fit. A custom searchable picker needs an accessible
name, expanded/selected state, keyboard navigation, an announced active option,
focus return, visible keyboard highlight and scrolling that follows it. Follow
the applicable [W3C combobox pattern](https://www.w3.org/WAI/ARIA/apg/patterns/combobox/)
when implementing it as a combobox; choose coherent semantics rather than mixing
button lists and an undocumented text input.

Ship only functioning controls. A product roadmap belongs in documentation,
not an enabled no-op button or a permanently disabled tool rail. Disable a
working control only for a current constraint, and explain the reason nearby.
Readonly configuration should look like information, with an explicit edit/open
configuration route when users need to repair it.

Use specific labels: Create entry, Render image, Save description, Download
missing files, or Stop image engine. Avoid ambiguous “Run,” “Create” and “Go”
when the result or interruption is unclear. Show one primary action
per task region, consistent secondary actions, and a distinct destructive style.
Do not require hover to discover essential actions.

## 6. Visual and accessibility standard

These numeric choices are Studio defaults. They are product design decisions,
not claims that WCAG specifies a particular font or spacing scale.

| Element | Studio default |
| --- | --- |
| Main content / editing text | 14–16 CSS px; comfortable 1.5–1.7 line height |
| Controls / essential help | 13–14 CSS px |
| Secondary metadata | At least 12 CSS px; never the only place an essential instruction appears |
| Page / section headings | 24–28 / 18–20 CSS px; clear semantic heading order |
| Spacing | Shared 4, 8, 12, 16, 24, 32 px scale |
| Control height | 36–40 px; compact toolbar targets at least 28×28 px |
| Rounding | Consistent small control and larger panel radii; avoid per-screen variants |
| Semantic colors | Neutral surfaces/text, violet selection, amber pending/warning, red failure, green success |

Use semantic tokens for surface, border, primary/secondary text, focus, selection
and status. Reuse shared buttons, fields, notices, empty states, popovers and
dialogs. Avoid repeating long utility strings with slightly different behavior.
Use the established icon family, give icon buttons accessible names, and hide
purely decorative icons from assistive technology. Pair color with text/icon
meaning; a gray dot cannot explain service health.

Measure text contrast against the actual composited background: at least 4.5:1
for normal text and 3:1 for qualifying large text. Do not fade actionable text
into the background. [W3C contrast guidance](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html)

All interactive controls need a visible focus indicator that remains unobscured.
A hover style is not a keyboard focus style. Never remove native outlines without
an equivalent replacement. [W3C focus guidance](https://www.w3.org/WAI/WCAG22/Understanding/focus-visible.html)

Use at least 24×24 CSS px pointer targets or qualifying spacing; WCAG provides
specific exceptions. Studio's normal controls should exceed that minimum.
[W3C target size guidance](https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum.html)

Adapt to desktop window resizing and enlarged text. Panels may collapse or move;
critical controls remain reachable. Story stage/canvas content may need spatial
layout, but surrounding forms and navigation should reflow. Verify at 900×640
and 1280×800, below the current minimum, and at 200% text scaling. Honor reduced
motion; loading pulses and animated transitions need quiet alternatives. Keep
full paths, prompts and descriptions available without depending on hover-only
tooltips. Image crops must not misrepresent landscape artwork or transparent
assets; use aspect-aware previews and accessible names.

## 7. Audit scope and evidence

Audit date: **2026-10-07**. Source baseline: **189d745**, with the pre-existing
local documentation edits retained. Reviewed Studio's app bootstrap, shell,
Projects, Writer, Canvas/gallery/detail/Inspector/composer/palette, all five
Settings sections, Plans/guide, shared Preview controls, command execution,
persistence entry points, shortcuts and styling.

Live inspection used the running macOS Studio with the existing **Playback
fixture** project and its isolated `.svs-e2e/` configuration. Visited Projects,
Writer, Canvas, artifact detail, Preview, Plans, and every Settings section.
Opened the existing error report and tested Escape, `g s`, and Storage Reveal.
The supplied screenshots and source independently establish the two-step
Details chain. Returned the app to Settings → Status. No story or settings were
edited; no project/plan was started and no model download or generation was run.
The running binary was not rebuilt, so source and live evidence are identified
separately rather than assumed to be the same build.

**P1** means draft-loss risk, blocked keyboard access or materially misleading
saved/ready state. **P2** means avoidable confusion, discovery or presentation
friction. **Live + source** means both were observed; **source** means the
implementation establishes the behavior but the failure scenario was not run.
The descriptions below preserve the **original audit observations**, rather than
describe the remediated app. All 25 have implementation changes recorded in §9;
remaining manual acceptance coverage is identified there.

### SF-UX-01 · P1 · Details opens another Details entry point

**Live + source; supplied screenshots.** In
[components.rs](apps/studio/src/ui/components.rs), `ErrorDetailsButton` writes
only the global report signal; `ErrorToast` separately owns `show_details`.
Status, ModelPicker and ModelFiles therefore send the user to a corner toast
before the full report. Open the report directly from the originating row,
using one shared report surface. **Acceptance:** each local Details action shows
the complete report in one activation and creates no redundant toast.

### SF-UX-02 · P1 · Dialogs do not enforce modal keyboard behavior

**Live + source.** Escape left the error dialog open and its initial focus
remained on the page. `ErrorDetailsModal`, `StackProposalModal` and `Palette` in
[components.rs](apps/studio/src/ui/components.rs),
[canvas.rs](apps/studio/src/ui/canvas.rs) and
[palette.rs](apps/studio/src/ui/palette.rs) lack the shared focus/inert/dialog
contract. Replace independent backdrops with the common primitive.
**Acceptance:** keyboard opening, focus containment, Escape, focus return, and
blocked background shortcuts work for every dialog.

### SF-UX-03 · P1 · Command entry traps Tab even without completion

**Source.** `CommandBar`'s `onkeydown` in
[cmdbar.rs](apps/studio/src/ui/cmdbar.rs) always prevents Tab; it only conditionally
inserts a suggestion. Thus no suggestions still means no forward focus movement,
and Shift+Tab is also caught. Claim completion keys only for a valid visible
suggestion and preserve backward traversal. **Acceptance:** Tab reaches Run
when completion is unavailable; Shift+Tab exits the field; valid completion
still works and does not submit.

### SF-UX-04 · P1 · Dirty work lacks a close/Quit protection flow

**Source.** [main.rs](apps/studio/src/main.rs) closes the event loop then stops
the runtime; the save paths in [ui/mod.rs](apps/studio/src/ui/mod.rs) handle
changes, timed autosave and explicit Save, not a dirty close decision. Off,
Every minute, unbound drafts and failed autosave expose work to loss. Add a
unified Save/Discard/Cancel close flow and hold the window if saving fails.
**Acceptance:** close and Cmd/Ctrl+Q preserve work for each state; cancelling
returns to the unchanged draft. Existing project-switch saving should remain.

### SF-UX-05 · P1 · Settings can change without being saved

**Source.** Planner-model, prewarm, residency and generation-default persistence
failures in [settings.rs](apps/studio/src/ui/settings.rs) only log after updating
live values. Residency restart failure also only logs. Show applied/saved/error
state with retry or rollback; refresh health after deployment changes.
**Acceptance:** an unwritable config file or failed restart gives visible feedback
and the UI accurately distinguishes the live value from the saved value.

### SF-UX-06 · P1 · Failed commands erase the editable request

**Source.** `Canvas::submit` in [canvas.rs](apps/studio/src/ui/canvas.rs) clears
the composer before slash parsing/validation or planner success. A typo or
unavailable planner forces reconstruction from a report. Preserve failed input
and its target; clear only on accepted submission and keep recoverable request
history for asynchronous failures. **Acceptance:** invalid commands and planner
failure retain the exact text and allow correction without opening diagnostics.

### SF-UX-07 · P2 · Advertised shortcuts never reach their handlers

**Live + source.** `g s` did not switch Canvas to Story.
[keys.rs](apps/studio/src/ui/keys.rs)'s `NAV` excludes `s`, `c`, `[`, `]`, `v`,
`r`, `m`, `e`, `f`, `p` and `t`, although the shell/Canvas handlers and
[shortcuts.rs](apps/studio/src/ui/shortcuts.rs) advertise several of them. Add
all working bindings through one verified dispatch path; remove unavailable
bindings. **Acceptance:** exercise every advertised shortcut through the live
event bridge, including text-field focus and dialog precedence.

### SF-UX-08 · P2 · The global palette is mounted only on Canvas

**Source; Settings shortcut attempt showed no palette.**
[ui/mod.rs](apps/studio/src/ui/mod.rs) advertises/toggles palette state globally,
but [canvas.rs](apps/studio/src/ui/canvas.rs) is its only rendering host. There
is no visible palette button in the shell; OS-global registration can also fail.
Mount the palette at workspace level, add a visible entry and use focused-window
shortcuts. **Acceptance:** pointer and keyboard can open it on every advertised
screen, and actions route to the correct editor/context.

### SF-UX-09 · P2 · Controls advertise actions they cannot perform

**Live + source.** Storage's Reveal click did nothing: `PathRow` has no handler.
General's Background generation is `Toggle { enabled: true }` with a default
no-op callback; Appearance offers only Dark. Canvas shows three unimplemented
tools (the former `ui/tools.rs` module was removed in the 2026-10-10 pass).
See [settings.rs](apps/studio/src/ui/settings.rs). Implement real controls or present
readonly information; remove planned tools from the production rail.
**Acceptance:** every enabled control produces its labeled result, and disabled
controls describe a current actionable constraint.

### SF-UX-10 · P1 · Accessible names and dynamic-state semantics are missing

**Source; live tree shows unnamed editing fields/switches.** `SettingRow` uses
paragraphs rather than associated labels; Writer/composer/description fields
lack explicit names, and `Toggle` has a role/state but no field-specific name.
SearchSelect/palette lack coherent expanded/option semantics; global notices
lack announcement roles. See [components.rs](apps/studio/src/ui/components.rs),
[settings.rs](apps/studio/src/ui/settings.rs) and
[writer.rs](apps/studio/src/ui/writer.rs). **Acceptance:** VoiceOver announces
each field, selection and meaningful asynchronous result without duplicate
highlight-layer text; navigation exposes its active destination.

### SF-UX-11 · P2 · Essential text and focus are visually too subdued

**Source; visually evident in supplied/live error screens.** Studio extensively
uses 9–11 px metadata/actions with zinc-600/700/800 on dark surfaces.
[tailwind.css](apps/studio/tailwind.css) removes all control outlines, while most
buttons have only hover styles. The shell's 900×640 minimum and fixed sidebars
also constrain enlarged text. Adopt the shared scale, readable semantic tokens,
visible focus and adaptive panels. **Acceptance:** measure actual contrast and
target sizes, traverse by keyboard, and verify resizing/text scaling. This audit
has not measured every rendered color pair or certified WCAG conformance.

### SF-UX-12 · P2 · Dimension errors leak between unrelated fields

**Source.** `GenerationSettings` passes one `size_error` signal to all four
`SizeField`s in [settings.rs](apps/studio/src/ui/settings.rs). One bad input
shows the same error beside other sizes; fixing another clears it. The
“Scene / beat size” row updates only `sizes.scene`. Give fields independent
validation and either update both kinds or display separate settings.
**Acceptance:** a character error remains local; scene/beat behavior matches
its label and the effective defaults used by generation.

### SF-UX-13 · P2 · Inspector hides effective defaults and restricts presets

**Source; live detail shows auto steps and portrait-only sizes.**
[inspector.rs](apps/studio/src/ui/inspector.rs) falls back to kind defaults rather
than the configured generation defaults, shows steps as “auto,” offers only
portrait dimensions and step presets ending at 24 while Settings defaults to 25.
Show effective inherited values, kind-appropriate sizes, custom valid inputs
and reset-to-default. **Acceptance:** changing global defaults is reflected in
an inheriting artifact's display, and landscape/custom sizes are reachable
without memorizing slash syntax.

### SF-UX-14 · P1 · Unsaved description drafts disappear on navigation

**Source.** Inspector description lives in a local `draft` signal and is reset
when `artifact.description` changes in
[inspector.rs](apps/studio/src/ui/inspector.rs); leaving Canvas unmounts it.
Neither navigation nor the shell's saved state accounts for that buffer. Keep
per-artifact drafts or protect departure with Save/Discard/Cancel.
**Acceptance:** type a description, switch artifacts/screens and return; the
text survives or the user explicitly chose to discard it.

### SF-UX-15 · P2 · Recent is a directory list sorted by name

**Source; live list contains indistinguishable sample names.** `scan_projects`
in [projects.rs](apps/studio/src/ui/projects.rs) scans configured storage and
sorts by descending name, without a last-opened record; an externally opened
project is not registered there. Failed directory/database reads silently become
an empty/partial list. Track actual recent opens with time/location context,
and separate load failure from no projects. **Acceptance:** an externally
opened story appears first, duplicates are distinguishable, and unreadable
storage offers Retry with an explanation.

### SF-UX-16 · P2 · An unmatched gallery filter leaves unexplained space

**Source.** [gallery.rs](apps/studio/src/ui/gallery.rs) checks whether the entire
registry is empty, not whether filtered groups are empty. No matching artifacts
therefore has no dedicated message or Clear filter action. Preserve the filter
and show a result count with explicit recovery. **Acceptance:** a nonmatching
query shows No matching artifacts and Clear filter; an empty project still
shows its creation guidance.

### SF-UX-17 · P2 · Promote is discoverable only by hover

**Source.** Revision Promote in [canvas.rs](apps/studio/src/ui/canvas.rs) starts
at `opacity-0` and appears only on `group-hover`. A keyboard user can reach an
invisible action. Keep the action visible in context or reveal it on keyboard
focus as well, and explain that it changes canonical description.
**Acceptance:** every experiment's Promote action is discoverable and readable
without a pointer; its resulting canon change is clear and undoable.

### SF-UX-18 · P2 · Runtime failures send users to developer setup

**Live + source.** The missing-model error recommends `model_setup` while
Settings already has Model files. Status offered process actions even for
missing artifacts; the footer always renders an Idle dot and “NORMAL.” See
[settings.rs](apps/studio/src/ui/settings.rs),
[ui/mod.rs](apps/studio/src/ui/mod.rs),
[health.rs](crates/studio-core/src/runtime/health.rs) and
[errors.rs](crates/studio-core/src/errors.rs). Route to relevant setup directly,
show real health with text, and keep the image engine's actions tied to actual
health (Start only when it can succeed). **Acceptance:** missing models lead
to their repair screen without terminal knowledge or redundant start failures.

### SF-UX-19 · P2 · Corner overlays compete for the same space

**Source.** `ErrorToast` and `GuideCard` both use `fixed bottom-6 right-6 z-40`
in [components.rs](apps/studio/src/ui/components.rs) and
[guide.rs](apps/studio/src/ui/guide.rs). Guide is rendered later by the workspace,
so concurrent content can cover error actions. Centralize placement/priority;
keep task errors local and make both essential controls reachable.
**Acceptance:** a guide plus background error remains legible without overlap
and cannot hide Stop, Close or recovery.

### SF-UX-20 · P2 · Applying a draft discards working playback immediately

**Source.** Apply draft compiles and calls `PlaybackModel::install` through
[preview.rs](apps/studio/src/ui/preview.rs). In
[lib.rs](crates/playback-ui/src/lib.rs), success resets session/bookmark and
failure clears the previous session/bookmark. Explain replacement/restart,
validate before replacing, and retain working playback when the new draft is
invalid. **Acceptance:** a failed apply shows actionable diagnostics while the
previous snapshot/bookmark remains available; successful replacement has a
clear result and deliberate progress-reset policy.

### SF-UX-21 · P2 · Plans obscure shared settings and stopping semantics

**Source; inspected live without running a plan.** Each card renders a mode
control backed by one global `mode` in [plans.rs](apps/studio/src/ui/plans.rs).
`start_plan` in [ui/mod.rs](apps/studio/src/ui/mod.rs) silently ignores Run while
any session remains, including a finished report. Guide Stop clears its card,
but [driver/mod.rs](apps/studio/src/driver/mod.rs)'s wait cancellation does not
cancel work already dispatched by [commands.rs](apps/studio/src/commands.rs).
Show shared mode once, explain disabled Run, allow a new run after completion,
and distinguish Stop guide from Cancel render. **Acceptance:** every Run click
starts or explains why not; stopping leaves continuing work visible.

### SF-UX-22 · P1 · Downloadable-file completion is presented as model readiness

**Live missing-override state + source; download not executed.** `ModelFiles`
in [settings.rs](apps/studio/src/ui/settings.rs) computes `missing` with
`needs_download`, then uses zero to say Model ready. A missing local override
is deliberately excluded from downloads by
[provision.rs](crates/studio-core/src/runtime/provision.rs), but still required
for runtime. The live UI shows three missing files, two downloadable. Recheck
all effective files, name configuration blockers separately, and avoid declaring
ready after only download completion. **Acceptance:** with default files present
and a missing override, show Blocked: local model file missing and an edit/locate
route; do not show Model ready.

### SF-UX-23 · P2 · Copy acknowledges an operation before it succeeds

**Source.** `ErrorDetailsModal` sets `copied` immediately; `copy_to_clipboard`
in [components.rs](apps/studio/src/ui/components.rs) neither awaits the clipboard
result nor checks `execCommand` success. It also binds Copy to diagnostic text,
which interferes with predictable selection. Await success, expose failures,
and keep text selectable. **Acceptance:** a failed clipboard operation never
shows Copied; normal selection does not trigger Copy.

### SF-UX-24 · P2 · Configuration failure is a startup dead end

**Source.** [app/mod.rs](apps/studio/src/app/mod.rs)'s failure branch displays
Configuration error and raw text only; the failed configuration signal has no
retry or repair action. Provide the affected file, plain explanation, Copy
report, Open configuration and Retry, using the same error vocabulary as the
workspace. **Acceptance:** correcting configuration allows retry without
restarting; a missing/unreadable file has a specific recovery route.

### SF-UX-25 · P2 · Writer hides diagnostics after the first four

**Source.** The diagnostic loop in
[writer.rs](apps/studio/src/ui/writer.rs) uses `compilation.diagnostics.iter().take(4)`
without a total count or Show all action. A longer error list is inaccessible
from the authoring screen, and later blockers can remain unexplained. Show
error/warning totals with an expandable, navigable Problems list and useful
filtering. **Acceptance:** a story with more than four diagnostics exposes every
message, and each item returns the caret to the correct source position.

### Additional scenarios requiring live validation

These are risks to test, not additional confirmed defects:

- **Task lifetime:** screens and Settings sections now remain mounted while
  hidden, preserving task owners and drafts. Still exercise a real generation
  and model download across navigation; reject stuck busy flags or lost results.
- **Stale preview assets:** Preview now compares the compiled plan, including
  artwork bindings, with the pinned snapshot. Exercise an artwork revision
  without source edits to verify the refresh prompt and deliberate restart.
- **Editor undo and composition:** verify native editing undo versus project undo,
  IME/composition, Unicode, long lines, and transparent highlight-layer alignment.
- **Popover geometry and list movement:** verify Settings pickers near viewport
  edges and keyboard selection below the fold, with a long model/adapter list.

## 8. Delivery order and verification

1. **Protect work and keyboard access:** SF-UX-02–06, 10, 14, 22. Include
   window close/Quit, failed disk writes, focus traversal and missing overrides.
2. **Make feedback/navigation coherent:** SF-UX-01, 07–09, 18–19, 23–24.
   Share report/dialog primitives and remove all enabled no-op controls.
3. **Clarify editing and discovery:** SF-UX-12–13, 15–17, 20–21, 25. Address task
   lifetime risks before implying that background work survives navigation.
4. **Apply the visual standard throughout:** SF-UX-11, with rendered contrast,
   target-size, resize, focus and text-scaling verification on each screen.

For implementation, each finding needs an owner, a linked change, and recorded
acceptance evidence before its status becomes resolved. Update this document
and ROADMAP together. Do not add a second competing design-rules file.

A UI change is complete only when the relevant path works with pointer and
keyboard, normal/empty/loading/failure states, and the supported desktop window
sizes. Verify screen-reader names/statuses for changed controls and inspect the
rendered result. State-driven Guide/Check plans supplement this review: they do
not prove real focus order, native event delivery, dialog behavior or contrast.

Preserve the useful existing behaviors: shared typed commands, explicit approval
of planner proposals, valid-source/artwork reconciliation, undoable revisions,
project-switch saving, reload confirmation and deterministic pinned playback.
Improve their presentation and failure boundaries without duplicating domain
rules in UI code.

The first audit delivered documentation only. The follow-up implementation and
verification are recorded below; use this record when reviewing current status.

## 9. Remediation record · 2026-10-08

Owner: Studio implementation. **Implemented** means the original defect has a
code change; it does not imply that every platform, failure injection or assistive
technology scenario has been exercised. File links identify the change in this
checkout. Keep the original acceptance criteria as regression checks.

| Finding | Implemented behavior | Change / evidence |
| --- | --- | --- |
| SF-UX-01 | Local Details opens the shared full report directly; no duplicate toast. | [components](apps/studio/src/ui/components.rs); live Settings discovery failure. |
| SF-UX-02 | Native modal dialog provides initial focus, background inertness, Escape, containment and focus restoration. | [dialog](apps/studio/src/ui/dialog.rs); live report and palette keyboard checks. |
| SF-UX-03 | Tab accepts a valid completion only; other Tab/Shift+Tab traverse normally. | [composer](apps/studio/src/ui/cmdbar.rs); live invalid-command Tab reaches Run. |
| SF-UX-04 | Window close and native Quit share Save/Discard/Cancel protection for dirty projects, descriptions, unsent requests and active operations. New drafts save to the exact chosen destination; failures keep the window open. | [close guard](apps/studio/src/ui/close.rs), [commands](apps/studio/src/commands.rs); live native window close/Cancel, Cancel/Discard Quit, persistence tests. |
| SF-UX-05 | Model, prewarm and residency settings persist before application; defaults roll back on write failure. Restart failures are visible and health refreshes. | [settings](apps/studio/src/ui/settings.rs); source review, disk/restart failure injection remains manual. |
| SF-UX-06 | Invalid commands and failed planning retain text and target; accepted requests have recoverable history. | [Canvas](apps/studio/src/ui/canvas.rs); live invalid command retains exact text. |
| SF-UX-07 | Navigation dispatch includes implemented bindings and a timed `g` prefix; unavailable bindings are removed. Native field undo keeps precedence. | [keyboard bridge](apps/studio/src/ui/keys.rs), [shortcuts](apps/studio/src/ui/shortcuts.rs); live `g s`, Cmd+K and field Tab. |
| SF-UX-08 | Workspace owns the palette, with a visible Commands button and focused-window shortcut on every screen. | [workspace](apps/studio/src/ui/mod.rs), [palette](apps/studio/src/ui/palette.rs); live Settings Cmd+K. |
| SF-UX-09 | Reveal opens the path and reports failure; theme/background behavior is readonly information. The remaining Pointer button and its unused driver/shortcut model were removed on 2026-10-10 because selecting it performed no editing action. | [settings](apps/studio/src/ui/settings.rs), [canvas](apps/studio/src/ui/canvas.rs), [components](apps/studio/src/ui/components.rs). |
| SF-UX-10 | Fields/switches have names, Settings descriptions are associated, combobox/listbox state is exposed, notices announce state, and decorative highlights are hidden from accessibility. | [components](apps/studio/src/ui/components.rs), [Writer](apps/studio/src/ui/writer.rs), [palette](apps/studio/src/ui/palette.rs); live accessibility tree shows named editors and inert dialog background. |
| SF-UX-11 | Small metadata has a 12px floor, muted colors are brighter, enabled targets have a 24px floor, focus is visible, reduced motion is respected and narrow layouts adapt. | [styles](apps/studio/tailwind.css), [workspace](apps/studio/src/ui/mod.rs); rendered macOS inspection. Full contrast/zoom coverage remains below. |
| SF-UX-12 | Each size field keeps its own invalid draft and error; Scene and Beat have separate defaults. | [settings](apps/studio/src/ui/settings.rs); source review. |
| SF-UX-13 | Inspector shows effective configured size/steps, includes landscape/square and 25-step presets, validates custom sizes and allows inherited-size reset. | [Inspector](apps/studio/src/ui/inspector.rs), [operations](crates/studio-core/src/registry/ops.rs); live inherited defaults, parser/reset tests. |
| SF-UX-14 | AppState owns per-artifact descriptions; navigation retains them, explicit Save applies them and autosave preserves editable drafts. Drafts count as unsaved work. | [state](apps/studio/src/state/mod.rs), [Inspector](apps/studio/src/ui/inspector.rs); live screen navigation and Quit protection, draft tests. |
| SF-UX-15 | Successful opens are recorded in order, including external paths. Recent shows location; scan/read/write failures are visible with Refresh. | [Projects](apps/studio/src/ui/projects.rs); recent-order test and live external fixture first in Recent. |
| SF-UX-16 | Filtered results have a count and distinct No matching artifacts / Clear filter recovery. Cursor clamps to available results. | [Gallery](apps/studio/src/ui/gallery.rs); live no-match query, Clear restores all seven fixture artifacts and clears the field. |
| SF-UX-17 | Promote remains visible with a tooltip explaining the canonical description change. | [Canvas](apps/studio/src/ui/canvas.rs); source review. |
| SF-UX-18 | Missing-model reports lead to Image generation setup. Actions depend on actual health — Start/Stop only, hidden when files are missing. Footer reports working/saved/unsaved state. | [settings](apps/studio/src/ui/settings.rs), [health](crates/studio-core/src/runtime/health.rs); live missing-file state, action tests. |
| SF-UX-19 | Notifications sit below the upper-right header; Guide remains lower-right. Report modal has higher native priority. | [components](apps/studio/src/ui/components.rs), [Guide](apps/studio/src/ui/guide.rs); live Guide plus command error. |
| SF-UX-20 | Failed compilation retains the previous playback session/bookmark. Successful replacement explicitly restarts with the new snapshot. | [playback model](crates/playback-ui/src/lib.rs), [panel](crates/playback-ui/src/panel.rs); procedural playback recovery test. |
| SF-UX-21 | Plans show shared mode once, disable Run with a reason during work, permit another run after completion and explain Stop after current operation. | [Plans](apps/studio/src/ui/plans.rs), [Guide](apps/studio/src/ui/guide.rs), [workspace](apps/studio/src/ui/mod.rs); isolated two-step guide completes. |
| SF-UX-22 | Downloadable files, missing local overrides and missing executable are separate states. Download completion never substitutes for runtime readiness; configuration repair and recheck remain available. | [settings](apps/studio/src/ui/settings.rs); live missing-file inspection, no large download performed. |
| SF-UX-23 | Copy reports success only after clipboard success; failures invite retry and report text remains selectable. | [components](apps/studio/src/ui/components.rs); live Copy → Copied after completion, async failure result review. |
| SF-UX-24 | Startup configuration failure offers full report, affected-file opening and Retry, with visible opening errors. | [App](apps/studio/src/app/mod.rs); source review. |
| SF-UX-25 | Writer exposes every positioned diagnostic with error/warning totals, severity filters, scrolling and source-jump actions. | [Writer](apps/studio/src/ui/writer.rs); source review and workspace tests. |

The follow-up also found delayed controlled-input renders overwriting rapid
typing. Writer and composer keep native text ownership; descriptions, names,
filters and generation defaults use the shared [text input hook](apps/studio/src/ui/text_input.rs).
Search dialogs keep their own native text while open. App-driven clears and
draft restoration still synchronize without replacing newer native keystrokes.
Live rapid typing preserved the exact command, description, palette search and
gallery filter. Description text survived screen navigation and detail remount;
Discard restored canon without saving verification edits.

Verification uses the rebuilt Dioxus desktop bundle and the isolated procedural
fixture under `.svs-e2e/`; it does not require image generation or large downloads.
Workspace tests include draft retention, exact-path new-project saving and
overwrite refusal, recent ordering, size inheritance reset and failed-preview
recovery. Run `cargo check --workspace --all-targets`, `cargo test --workspace`,
`cargo clippy -p storyfella-studio -p storyfella-studio-core -p storyfella-playback-ui
--all-targets -- -D warnings`, `cargo fmt --all -- --check`, `npm run css:build`
and `cargo xtask build` when changing these paths.
All listed checks passed for the remediation. The final rebuilt bundle also
passed modal forward/backward Tab wraparound and Escape focus return, named
field checks, direct local report opening, Copy success, failed-input retention,
filter recovery, and native close/Quit draft protection on macOS.

Pre-commit review consolidated command-bar text synchronization and native
close/Quit protection, made draft application atomic, and blocked Save New when
either the chosen database or its neighboring `.sf` source exists. Regression
tests cover both failure boundaries. Status recovery uses typed core probe
results rather than parsing human-readable error messages. Native Quit also
works on the startup configuration-error screen.
The final desktop Smoke plan passed 7 checks and Story authoring walkthrough
passed 25 checks in Check mode. A native macOS launch with deliberately invalid,
isolated configuration confirmed the error screen and successful Cmd+Q exit.

Remaining acceptance coverage: full VoiceOver reading/announcements; IME and
composition; Windows/Linux close/menu/focus and path-opening behavior; every
advertised shortcut; full contrast and 200% text/resize coverage; injected overlay
write/clipboard/restart failures; real downloads and generation across navigation.
These are verification limits, not claims that the original defects remain
unchanged. Do not mark accessibility conformance from source checks or Guide
results alone.

### Model storage and general configuration · 2026-10-08

Application settings live in `studio.toml` and inference settings in the shared
`clipforge.toml`; labels and repair actions use Application configuration and
Inference configuration. Model downloads
default to OS-local application data in all builds, never the checkout or bundle.
Model files provides Change location and Open folder. Save the selected folder
before reporting success; clearly state that restart applies it and pause new
downloads until restart. Preserve existing files and allow selecting their tree
to reuse downloads. Keep explicit executable/adapter/weight paths independent.
Old user settings migrate once without deleting sources or overwriting the new
configuration. Atomic writes prevent a failed settings update truncating it.

Verification: workspace tests cover the OS-local default, the platform app config,
preserved API-key settings/comments, existing model-file reuse, malformed
provider sections, and one-time migration without overwriting or deleting its
sources. Strict workspace linting and the runnable desktop build pass. Native
macOS checks confirmed the Application Support destination, folder chooser,
saved-location notice, disabled preparation until restart, Finder reveal, and
the selected destination with preparation re-enabled after restart. These checks
used isolated settings under `.svs-e2e/`; Windows/Linux folder controls and a
full multi-gigabyte download remain outside this manual check.

### Models and Services: what to use, and how it runs · 2026-10-08

Settings is General / **Models** / **Services** / Storage — the separate Status
and Image-generation sections are gone. The old single "Models & services" page
is split so a user setting up an endpoint never meets a model path or a service
action:

- **Models** is the "what to use" page: **Text (LLM)** and **Vision (LLM)** are
  the same card — a **Provider**, an editable **Base URL**, a **Model** (a
  dropdown of the ids the endpoint serves — only served ids are offered — with
  a refetch action) and an **API key**. A managed (`llamacpp`) server is not a
  different shape: the app may
  start its `llama-server` internally, but the card shows the same endpoint
  fields, and editing the base URL moves the server it launches. A managed
  provider that is not running reads `Idle · starts on demand` and offers no
  model until it is up, because the app starts it on demand.
  **Services** gives each managed provider **Start/Stop/Restart**, so it can be
  booted and its `/v1/models` list fetched.
  Both pickers share one list — every chat provider, whatever it declares —
  because selecting a provider for a capability records it
  (`persist_provider_capabilities`), so any provider can serve text, vision, or
  both; the user decides. Provider options are labelled `id · kind · endpoint`
  (the picker's `SelectOption` separates the id it stores from the text it
  shows), so they read as endpoints rather than bare config ids. The capability
  labels are Text/Vision, not the internal "planner" name. The image model's
  profile, generation quality (steps, per-kind sizes) and LoRA bindings live on
  the same page, because they describe the model rather than the process. A
  **Providers** section lists every configured chat provider and can **add** one
  — validated against the shared registry before it is written — or
  **remove** one it defines (`AppConfig::user_providers` marks the removable
  ones).
- **Services** is the "how it runs" page: the image engine's status and
  **Start/Stop** (or **Set up image engine**, which scrolls to the weights), its
executable and runtime endpoint, residency, prewarm, the model-files
provisioning list, and each managed `llama-server` provider with its model file
and **Start/Stop/Restart**. The image engine shares one native server with
Atelier and the Clipforge apps through the machine's shared lifecycle owner
(`clipforge_inference::control`), so it offers Start/Stop rather than a private
restart: **Start** ensures the configured model is resident, replacing a server
the shared owner started.

An edit persists before it applies, writing `base_url`/`model`/`api_key` (or
`[defaults] text`/`vision` and the provider's `capabilities` for a provider
switch) to the shared `clipforge.toml`, then rebuilds the planner client, re-runs the health
check and re-discovers the endpoint's models. Adding or removing a provider
reloads the effective configuration from disk, so the registry is validated as a
whole. A failed save keeps the previous value, and clearing the key removes the
line so a provider's `api_key_env` applies again. **Storage** names every
resolved path and both configuration files (Application configuration and
Inference configuration).

Verification: workspace tests cover the per-capability provider list, a
persisted `[defaults]` switch taking effect on reload, line-preserving endpoint
writes (including clearing an API key), adding a provider and rejecting a broken
one before it is written, removing a user provider together with its `[defaults]`
entry, the in-memory re-point of a provider and its active deployment, editing a
managed provider's base URL moving its launch address, the service-action table,
the managed/external split and the planner-client rebuild;
`cargo check --workspace --all-targets`, `cargo test --workspace`, `cargo clippy
-p storyfella-studio -p storyfella-studio-core --all-targets -- -D warnings` and
`cargo fmt --all -- --check` pass. Live field and action interaction remains a
manual check.

### Command bar: every prefill focuses the field · 2026-10-08

A prefill that does not focus the command field is a trap: the description is
typed into nothing, and the skeleton command is then submitted unchanged —
`SVS-APP-103` with the detail "create needs a description". Only the `v`/`r`/`m`
and `/` shortcuts focused the bar; the `n` key, the canvas/layout buttons, the
inspector's adapter button, the Writer's mention "Render…" and the palette did
not. `cmdbar::prefill` is now the one way a prefilled command reaches the bar: it
sets the text, retries focus across frames until the bar is visible (a Writer or
palette handoff switches to the canvas first) and puts the caret after the text;
`cmdbar::focus_field` is the `/` shortcut's focus-only variant.

### Atelier image workspace and approved Studio artwork · 2026-10-09

Atelier preserves original image bytes and exposes a large fitted/100% workspace
with zoom/pan, immutable edited versions, branching and parent comparison. Its
shared library has searchable names/tags, favorites, original import, and explicit
Open / Reference / Use actions. Use embeds a project copy and its preview; global
library access remains available after switching studies. Image-backed layers
support paint, erasing, undo/redo, placement, scaling and normal canvas export.
Mask drafts have independent stroke undo/redo and durable saves. A failed mask
write retains the draft, exposes Retry, and blocks image/project replacement and
Quit until saving succeeds. Model presets have unique control IDs even when
their generation routes match. Settings uses provider dropdowns and a full-height
scrolling body with a pinned Save action; successful save closes the panel.

Clipforge owns ordered-reference/mask capability checks and provider payloads.
Unsupported combinations fail before submission. Masked output preserves source
dimensions and original pixels outside the painted area. Capabilities differ by
route; this does not imply every provider/model supports masks or multiple inputs.

Studio's new Import Atelier artwork action previews an approved `.sfart` package,
lets the person choose its name and Character/Environment/Object role, and adds
a completed revision through AppState. It performs no inference. Errors remain
in the shared dialog; successful import closes it. Project identity is checked
across background work. Normal undo/redo and project persistence own the result.
Both original image versions and painted canvases can produce approved packages.
This is the implemented Studio handoff UX; its hosted-provider controls and a
full Studio art-editing workspace remain separate future work.

Verification used isolated `.svs-e2e/atelier-workflow` data and a disposable macOS
test bundle. Native interactions exercised full-image fit/100%/pan, parent
comparison, mask paint/erase/undo/reopen, focused prompt and model typing,
searchable saved tags, favorites, new-study library reuse, image-layer painting,
erasing/moving/scaling, provider presets and settings save/close. A read-only test
library verified visible save failure, retained mask, blocked study switch/Quit,
and successful Retry after restoring access. At about 1,010 logical pixels wide,
toolbars wrapped, contained library thumbnails remained accurate, and settings
kept its footer visible while scrolling. Studio's runnable Dioxus build displayed
the approved preview, imported an Environment, and removed/restored it with
undo/redo. Workspace persistence tests cover reopening the saved import.

Automated coverage includes original resolution/alpha, library and project
round trips, lineage, image-layer transforms/erasing, mask polarity/protected
pixels, failed draft-write retry, approved packages, and Studio import persistence.
Ten physical Metal tests include paint/erase image-layer pixels and approved
canvas export. Clipforge mock HTTP tests verify ordered inputs, mask pixels,
provider field names, and rejection before submission. Paid-provider output and
Linux/Windows native UX were not exercised in this pass. See `docs/atelier.md`
for the bounded image/project/library limits and remaining scope.

Repository checks passed: workspace all-target check and tests, strict Clippy
for Atelier/Studio/Studio core, formatting, the runnable Studio build through
xtask, and the signed Atelier macOS bundle. Clipforge's `xtask check`, `xtask ci`
(including MCP smoke), formatting and strict inference-crate Clippy also passed.

### Studio workspace and visual actions · 2026-10-10

The workspace now has stable icon-and-label navigation, a project switcher and
named icon actions for commands, save, undo and redo. Authoring requires an open
project; the prerequisite offers a direct Projects action while keeping task
owners mounted. Routine explanatory copy was removed from the gallery,
inspector, project cards and action bars. Generation settings and provenance
use disclosures. Artwork actions, the variant strip and the command composer
occupy separate layout regions so they do not cover the image.

New artifact, variant, edit, regenerate and attach-adapter actions open focused
forms, validate locally and compile through the shared typed operation layer.
The palette offers these actions, workspace navigation, save/undo/redo, artifacts
and every slash operation. Common operations open forms; advanced operations
prefill the composer and never submit incomplete operations. Visual target
actions use artifact identity, and contextual slash prefixes/completion safely
quote spaced names and smart punctuation. `/` reaches the composer from other screens outside
text entry. Failed commands retain input with local feedback and direct Details.

Live testing found and fixed two event bugs: delayed caret events restored
submitted commands, and an import effect subscribed to its own loading state,
reopening the native picker. Native text input now owns command text; explicit
prefill/clear synchronizes the field. The picker effect reads task state without
subscribing to it.

macOS acceptance used an isolated bundle and project data. Checks exercised
project creation and prerequisites; form and slash creation of text artifacts;
quoted-name description updates and Tab completion; palette navigation from an
input; invalid-command retention; accepted-command clearing; approved artwork
preview, editable import name and completion without repeated pickers; edit
validation, ordinary typing, contained Tab navigation, Escape/focus return;
variant and adapter dialogs; generation inheritance, sampling selection, seed
pinning and toolbar undo/redo. At approximately 1,203 × 644 logical pixels,
the workspace actions, composer and dialog footers remained reachable and the
inspector scrolled independently. Smaller widths, 200% text scaling, assistive
technology, Linux/Windows native interaction and fresh inference output are
not covered by this acceptance pass. Studio's hosted image-provider UI remains
planned; this pass preserves the existing inference path.

Final verification passed: workspace all-target check, workspace tests, strict
Studio/Studio-core Clippy, formatting and the runnable Dioxus build. The final
isolated build reopened saved artwork and retained generation settings; ordinary
typing of a quoted-name command updated the description, cleared its native
field and disabled submission. Palette navigation and global command entry
also passed on that build. Logs are under `.svs-e2e/logs/studio-ux-*`.

### Compact action entry and fields · 2026-10-10

The bottom entry now occupies one compact row with a search icon and a named
arrow control. It searches the same action catalog as the palette. Typing
`new story`, `edit`, `variant` or an artifact name offers actions; Up/Down selects
and Enter opens the action or artifact. Common actions use focused forms with
the selected artwork already bound. No arguments need to be remembered. Tab
traverses normally in action mode; Escape dismisses suggestions and preserves
the query. Plain-language requests have an explicit Ask assistant choice and
Cmd+Enter (Ctrl+Enter off macOS). A slash operation followed by whitespace
enters the existing argument editor, where Tab completes valid tokens. Recent
requests still restore their text and original target.

Field focus uses a thin inset treatment rather than an external rectangle;
the palette indicates focus on its search surface. Native selects have the
same 36px height, padding, surface and rounding as single-line textboxes, with
a consistent chevron. Buttons retain visible keyboard focus. Task forms have
a viewport-bounded 540px width instead of shrinking to intrinsic text width.

Isolated macOS acceptance verified compact resting/active entry, arrow-key
artifact selection, Enter to open, new Story creation without slash syntax,
kind inference for Scene, contextual edit/variant forms, Tab to the arrow
control, Escape/focus return, Cmd+K search without the outer outline, palette
`/create` opening a form, quoted-name command execution/clearing, and request
failure preserving text when the test LLM endpoint was unavailable. Live
testing caught stale action dispatch when typing and pressing Enter rapidly;
activation now reads current signals, and the rebuilt app passed that exact
rapid-Enter check. A regression also protects artifact-name matching from
contextual action hints. Fresh inference and Linux/Windows native styling
remain outside this acceptance pass. Logs use `.svs-e2e/logs/studio-quick-*`.

Verification passed: workspace all-target check and tests, strict Studio
Clippy, formatting, stylesheet generation and the runnable Dioxus build.

### Edit prerequisites in the action entry · 2026-10-10

An explicit `edit`, `modify`, `/modify` or `edit image` query now exposes an
unmet prerequisite instead of silently falling back to Ask assistant. Selected
artwork without an image shows “This artwork has no image to edit yet” and
offers Generate image first for that same artifact. Enter opens the generation
form; generation still requires submitting that form. Its title and primary
action say Generate image when there is no existing image. With no visual
selection, the entry asks the user to select artwork, retains the query and
disables submission. Ordinary Enter does not send that blocked action to the
planner. Completed artwork continues to open Edit image. Longer instructions
remain assistant requests; Cmd/Ctrl+Enter is still an explicit request shortcut.
The general palette's unavailable Edit image row explains the missing image.
Keyboard hints describe the actual choice count and request/action behavior.

Isolated macOS checks used `harbor_dock` without an image and a completed
`Cabin library copy`: prerequisite feedback, Enter to the generation form,
Escape/focus return, no-selection Enter retaining input without planner calls,
and rapid typing/Enter to the completed artwork's edit form passed. A regression
covers missing/available images, aliases, original target identity, no selection
and no project. Workspace all-target check/tests, all 95 Studio tests, strict
Studio Clippy, formatting, stylesheet generation and runnable build passed.
Logs use `.svs-e2e/logs/studio-edit-entry-*`. No fresh image generation was needed.

### Writer controls and quitting · 2026-10-10

Rename is a focused native dialog with the current scene/beat name prefilled,
Enter to submit, local validation and Escape/focus return. The toolbar enables
it only for resolved scene or beat symbols. Renaming still uses the shared
source/catalog mutation and joint undo. Problems starts collapsed with visible
error/warning totals; its bounded list has readable messages, secondary codes
and locations, severity markers, filtering and click-to-source navigation.
The redundant readiness paragraph and caret-following footer copy are removed.

Quit uses the shared task layout and styled Cancel, Discard and quit, and Save
and quit controls. Save is visibly disabled for an active operation or unsent
request, with a concise reason. Recognized action searches no longer block a
clean quit; assistant prose and explicit slash commands remain protected, as
do unsaved project changes and description drafts.

Isolated macOS acceptance verified invalid/valid rename, rapid Enter, source
and artwork identity changes, undo, Escape/focus return, collapsed Problems,
error/warning filters and navigation to the sixth diagnostic. Quit checks
verified focus containment, request preservation after cancellation, clean
quitting with an `edit` search, and pending description persistence through
Save and quit and reopening. Workspace all-target check/tests (97 Studio
tests), strict Studio Clippy, formatting, stylesheet generation and runnable
build passed. Logs use `.svs-e2e/logs/studio-writer-controls-*`. Cross-platform
native styling and screen-reader acceptance remain outside this pass.

### One shared native server owner · 2026-10-10

The image engine's lifecycle now belongs to the machine, not the app. Studio
drives the same `clipforge_inference::control` owner Atelier and the Clipforge
apps use, over one shared state directory (`paths::state_dir()`, the platform
data dir, `$SVS_STATE_DIR` override), so a server one front end started is
reused instead of a second one launched on the same port. `CreativeRuntime`
keeps the deployment and client but no longer owns a process: `ensure_ready` is
`control::load` (start when down, replace a server the owner started that holds
a different model), `Stop` is `control::stop`.

The Services image-engine card is **status + Start/Stop**: a separate Restart
and Take over are gone because the owner applies residency changes and replaces
a server it started. A server the owner did not start is reported as `Degraded`
with the resident model named and the CLI remedy (`svs runtime stop --force`),
which is also SF-UX-18's fate for the old Replace action. Because the server is
detached and shared, quitting the app no longer stops it — it stays resident for
the next front end and is stopped from Settings or `svs runtime stop`.

Verification: `cargo check --workspace --all-targets`, `cargo test --workspace`,
`cargo clippy -p storyfella-studio-core -p svs-cli -p storyfella-studio
--all-targets -- -D warnings` and `cargo fmt --all -- --check` pass. Live
multi-front-end reuse and the Settings Start/Stop/residency interactions remain a
manual check.

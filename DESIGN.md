# Storyfella Studio design standard and UX audit

### Atelier drawing and painting · 2026-11-11

The drawing rail opens the Shapes and Brushes panels. Both use the same color
picker, canvas sampling and project swatches. Shape edits apply to one item
inside its layer. A closed shape can create a clipped paint target above it.
The drawing composer previews the accepted canvas source and can place a
variation on a new artboard.

In an isolated macOS build, a rectangle was drawn, recolored, and used to
clip a soft brush stroke dragged across both edges. The composer displayed
that composition in New artboard variation mode. Automated checks exercise
editable save and reopen, shape transforms and undo, Metal and CPU clipping
across tile boundaries, Studio publication, and generation with a provider
fixture.

### Focus treatment and explorer rename

Avoid bulky, offset focus outlines and bright purple focus rings. Replace them
with a compact, neutral inset focus indicator that remains visible for keyboard
navigation in both themes. Icon actions use no padded background; hover and active
states use the icon color. Every icon action has a descriptive hover tooltip,
including disabled actions. On macOS, Enter starts renaming a focused project
file, folder or catalog asset; on other platforms, F2 does so. The entry story
file remains protected. Space and pointer activation continue to open items.
Native visual and keyboard acceptance remains open.

### Compact asset metadata

The asset inspector presents the reference name in a shared read-only input and
the description as plain text, each with a compact edit action. Editing retains explicit confirmation,
draft recovery and multiline support. Native visual and keyboard acceptance remains open.

### Asset uses and variations

The asset toolbar puts story uses in an icon menu and places the Variation
action beside Open in Atelier. A single asset no longer has a redundant
variation row; the row appears when there are multiple family members. The
variation dialog identifies its action as creating a variation. Native visual
and keyboard acceptance remains open.

### Contextual playback actions

The Story toolbar exposes Play story and Play scene as named icon actions. The
scene action is disabled when the caret does not identify a scene. Shared
playback controls expose their labels and available shortcuts as tooltips. Native
visual and keyboard acceptance remains open.

Studio General settings persist playback autoplay timing, text size, location
artwork fit and diagnostic visibility for Preview and the standalone Player.
Playback failures show a compact notice;
the full diagnostic opens in the playback details panel. Settings launch and
reveal failures use Studio's shared error report. Native interaction, visual
and accessibility acceptance for these changes remains open.

### Playback navigation

The shared playback stage offers previous and next passage controls and optional
automatic advancement at three-second intervals. Previous restores earlier
playback states; restart, jump and restore clear that history. Automatic playback
stops when a passage cannot advance or an error occurs. Native visual and
accessibility acceptance of these controls remains open.

### Published illustrations in Preview

`show illustration NAME` fits the full published illustration into the
shared playback stage. The illustration takes precedence over the location
backdrop. The deterministic Atelier MCP fixture checks the published pixels,
and a native Studio Preview run verified the exact published revision UUID.
Evidence is recorded in `.svs-e2e/logs/atelier-mcp-studio-preview.log`.

### Isolated Preview capture and Atelier blend masks

Studio can render an MCP playtest through the same playback panel in a
separate macOS WebKit view. The acceptance run captured both branches at
wide and near-square sizes, checked staged portrait pixels, and showed
the placeholder for Ivo's pending expression. Atelier saves a single drawing
target per artboard, including the rectangle for Blend into artboard, and
includes it in normal undo.
Evidence is recorded in `.svs-e2e/coord/live-edits-preview.md`.

### Story project explorer · 2026-11-08

The Story workspace presents user-facing files and catalog assets together at
the project root, with no extra Files or Assets group. New shared projects store
those files in `assets/`; a minimal project shows only `story.sf`. Internal
database, manifest and working-artwork files are hidden. Standalone projects
show files beside their entry source, including the entry itself when its name
starts with a dot. Enter on macOS and F2 on other platforms rename eligible
items; Space opens them. Studio protects the entry file and story files with unsaved edits or saved or unsaved include
references. Renaming or deleting an unreferenced chapter clears source undo
history. Empty and nested folders are visible. Users select a folder and create
files or folders there, or drop external files into it. File imports copy without
overwriting. New `.sf` files are included in the entry draft; imported `.sf`
files remain unlinked until the author includes them. Images open in the work
area. Opening a catalog asset replaces the editor content while retaining the
explorer and the open source draft. Returning to a file restores its draft.
Cmd+B (Ctrl+B elsewhere) toggles the explorer and workspace navigation together.
Assets navigation opens the gallery. The Story file toolbar offers Play story
and Play scene, starting at the scene containing the caret; the latter continues
according to normal story flow. Preview is no longer a primary sidebar destination.
Additional source files use the shared language analysis for highlighting, completion and file-specific
problems; included files use the project snapshot for completion and diagnostics.
This is shared language-service integration, not a separate LSP client. Native
interaction and accessibility acceptance remain open.

The New asset dialog uses the shared task-form spacing, grouped field labels,
close control and aligned actions, preserving drafts and validation feedback.
Add to project opens a compact anchored action menu. File and folder forms open
inline at their destination. Asset creation opens a task dialog.

### Initial visual directions

The shared playback stage renders close framing, a tense overlay and
monochrome artwork, with explicit normal/reset values. Effects are scoped to a
scene; unknown directions remain visible as unsupported. Runtime restart and
restore are tested. An isolated Studio check verified close framing, tense
overlay and monochrome artwork together, including clipping without an extra
scrollbar. Broader direction and staging acceptance remains open.

### Multi-file Preview foundation

Preview compiles open drafts with saved included files. The explorer lists
sources and assets, keeps independent drafts, creates linked chapters, and
shows project diagnostics. Preview opens the originating file when its
snapshot is current; scene, beat and asset renames preserve identities across
files. An isolated native check covered chapter creation, writing, preview,
return to source, scene rename and undo/redo. File rename/removal, other editor
services, accessibility and narrow-window acceptance remain open.

### Studio Preview without artwork · 2026-11-07

Valid stories play in Studio with missing or unreadable artwork. Missing active
locations, characters and props appear as translucent labelled placeholders in
approximate positions; a visible notice identifies the approximation. Preview
also reports unsupported visual directions that it skips. Named character
instances render in groups with scale, depth and optional precise positioning;
native visual acceptance remains tracked by SF-PLAN-18–19.

### Atelier Layers sidebar · 2026-11-03

The Layers sidebar has a compact, left-aligned layer list above a contextual
inspector. Selection and the paint target are independent; group opacity and
candidate review remain distinct from layer properties. Alignment distinguishes
positioning a selection as a unit from aligning each item to the artboard.
The proposed interaction and acceptance criteria are in
[the sidebar specification](../storyfella-canvas/docs/atelier-layers-sidebar-spec.md). Native review
has covered a paint layer, exact position entry, and importing an image while
keeping the Sketch layer selected and targeted for paint. Selecting the imported
image kept Sketch as the target. Setting the image as the target and pressing
⌘Z restored Sketch as the target while the image stayed selected.
An automated regression covers undoing and recreating a destination artboard
while image work still refers to the original. Group, candidate, broader
keyboard, accessibility and constrained-window acceptance remain open.

### In-scope artwork links · 2026-11-02

Scene and beat shortcuts are compact full-width rows with a shared type icon,
type label and name. The whole row opens Assets; a tooltip explains the target.
Repeated Open/artwork/View asset text is removed. The same AssetIcon vocabulary
is used for gallery group headings and asset placeholders. Native visual
verification remains with the user.

### Appearance, shared Vision configuration and Assets naming · 2026-11-02

General offers Dark, Light and System, persisted in `studio.toml`. Theme changes
apply immediately to workspace surfaces, controls, dialogs and writing/review
editors and native window appearance. System follows live OS appearance changes. Semantic colors preserve
readable text, feedback and focus rings without resetting editor state. Artwork
preview backgrounds remain an independent viewing preference. A failed setting
write retains the previous selection/theme and reports the failure.

Models exposes **Use a separate Vision LLM**. Off hides Vision's fields and
routes image-understanding calls through the Text provider, URL, model and key;
the selected model must accept images. On restores the separately configured
Vision selection and exposes its fields. An existing distinct Vision setup stays
enabled unless the user changes it. Credentials and provider definitions remain
in the shared registry; the app preference does not overwrite them. A provider
selected for both roles shares its settings, which the UI explains. Model lists
and health checks discard responses for superseded requests.

When a provider cannot be reached, the model row shows an actionable error code
and refresh button on one line. The code opens the themed error report. The row
does not offer unverified model choices or change height for explanatory text.
Models keeps explanatory copy out of the rows; status, linked Vision behavior,
and the provider refresh action have tooltips. **Refresh providers** reloads the
shared catalog and rechecks availability and model lists.
The refresh action shares the Models heading row. Vision's heading stays visible
with its separate-model switch on the right; its fields appear when enabled.

The former Canvas screen is **Assets**, including navigation, palette, tooltips,
guide steps and the `g a` shortcut. Atelier still owns editable canvases. Earlier
dated records below retain their original Canvas terminology. Automated evidence
covers preference persistence, linked/separate Vision routing and System-mode
changes; native visual verification remains with the user.

### Inline metadata and transparent artwork previews · 2026-11-02

Inspector names/descriptions display as text with a pencil action at the right
of each label. Each enters editing independently. Enter/check confirms that
field; Escape/close discards it. Shift+Enter inserts multiline text. Validation
errors retain the editor and draft; Tab/blur never submit or discard. Native IME
composition is respected, focus returns to the pencil, and confirmation cannot
silently overwrite a displayed value changed by another edit. Description drafts
retain normal Save/quit/conflict behavior; per-field confirmation leaves unrelated
drafts untouched. Components are documented in [the catalog](docs/ui-components.md).

Canvas, gallery, saved image history and Writer thumbnails use a light paper
preview background so dark strokes on transparent PNGs remain visible. The Canvas
Background selector offers Light, Dark and Checkerboard, shared for the Studio
session. This is a viewing preference: source pixels, alpha, revisions and
playback compositing are unchanged. Native visual acceptance is assigned to the
user; keyboard bridge and scoped metadata regression tests cover the behavior.

### Atelier Save updates Studio artwork · 2026-11-01

Ordinary Save and Save-and-close in a shared Atelier document also update its
linked images in Studio. No export, approval or publication action is required.
Save persists editable work first; render/catalog failures keep the project dirty
and show the existing save failure with recovery details. Retry preserves image
identity and avoids duplicate unchanged revisions. A concurrent outside image
change blocks overwrite; Assets can relink after reviewing that image, or unlink
an artboard. Undo/redo retains the latest publication baseline. Recovery and
navigation snapshots and Save As copies preserve editable work without changing
the shared image. Pending candidates and empty artboards stay unavailable until
accepted artwork exists. Automated save/persistence tests pass; native Metal
rendering and the visible cross-app refresh remain user acceptance.
User acceptance (2026-11-02): Atelier Save updates the image in Studio. Remaining
edge-case native checks are not implied by that reported round trip.

Save errors now distinguish failure to save the drawing, failure to publish
artwork after saving the drawing, and failure to record revision links after
publishing artwork. Save and Save-and-close carry committed revisions back to
the matching open artboard without replacing a newer relink. The project
stays open on failure and the saved drawing can be reopened and retried.
The offline chapter test injects the final write failure and verifies
recovery. Native error presentation still needs a live check.

### Open in Atelier · 2026-11-01

The shared Open in Atelier action appears in Canvas details, resolved Writer
references and the current scene toolbar. Its tooltip states that it saves
first; standalone projects expose their shared-folder prerequisite. Busy state
lasts through the receiving app's reply. Save/open failures retain work and
offer local Details. Opening performs no generation or image publication.
Atelier reuses the project tab and linked artboard, preserves existing edits
and unsent prompts, and shows the relevant source in a Story context panel.
New artboards embed saved artwork and prefill the description. A canvas toolbar
action reopens context. Mask-save failures block handoff with actionable feedback.

Automated identity, transport, worker and document tests cover the contract;
native launch/focus and control acceptance remain assigned to the user. See
[the handoff contract](docs/shared-project.md#open-in-atelier). Ordinary canvas
save publication is covered by SF-PLAN-06 above.

### Shared catalog synchronization · 2026-11-01

Studio and Atelier refresh saved assets approximately every two seconds while
preserving dirty source and form values. Independent fields merge; overlapping
edits show local/saved values and require an explicit choice. Studio's persistent
Review changes notice does not steal focus. Its shared dialog can keep either
side for conflicts; duplicate identities require an explicit saved-catalog
reload with the discarded catalog edits and project undo scope stated. Cancel
keeps local work. Existing items are renamed in Studio, so an Atelier form never
rewrites references in an unseen unsaved source buffer.

Inspector drafts participate in conflict detection. Undo/redo retains external
artwork and new items. Atelier's list refreshes separately from its form, and
an image attachment does not clear an unrelated description draft. A failed
refresh has persistent Retry/details. Interrupted source/catalog saves recover
from durable intent; unrelated file text requires a confirmed preserve-copy
action, and the copy location stays visible afterward. Registered artwork paths
serve shared-project images as well as generated output.

Automated acceptance covers merge/decision semantics, drafts, undo/redo, stale
decisions, duplicate identities, failed commits, interrupted saves, exact source
recovery and registered image paths. Workspace checks/tests, changed-package
strict Clippy and runnable builds are the delivery checks. Native two-app
interaction remains assigned to the user; linked-canvas Save publication is
SF-PLAN-06. See [the contract](docs/shared-project.md#synchronization-and-concurrent-editing).

### Workspace sidebar toggle · 2026-11-01

The workspace header starts with a sidebar toggle, followed by the current screen
label. It contains no project name or duplicate Projects shortcut. The toggle
uses the shared icon action with Show sidebar/Hide sidebar tooltips and accessible
expanded/control state. The Projects destination remains in the sidebar. Hiding
the sidebar removes its width and focus targets; the header toggle stays available
to restore it. Visibility is retained across screen changes during the session.
Projects home keeps its separate minimal presentation with no workspace chrome.
Native toggle and keyboard acceptance remains assigned to the user.

### Shared text and search inputs · 2026-11-01

Studio's reusable vocabulary now includes TextInput, TextInputWithAction and
SearchInput, composed around one native input surface and the existing text-value
bridge. Project search uses SearchInput with a visible trailing ×; × and Esc both
clear filtering, restore the heading/list and return focus to Search. There is
one cancel icon across desktop webviews. The New story name uses TextInput.
Inputs support accessible labels, optional stable IDs, placeholders, two sizes,
disabled/read-only states, mount focus and labelled trailing actions. Styling and
focus treatment are shared; action buttons do not submit enclosing forms.

See [the reusable component catalog](docs/ui-components.md) for APIs, defaults,
composition, examples and existing shared components. Native input/cancel/focus
acceptance remains assigned to the user.
Shared-input/sidebar validation passes: workspace tests/all-target checks,
strict Studio Clippy, formatting, regenerated Tailwind and the desktop build.
Logs: `.svs-e2e/logs/shared-inputs-*`.

### Minimal Projects home · 2026-11-01

The row directly above the project list places Your stories on the left and
search, refresh and add on the right, with tooltips and accessible names. There
is no separate home topbar. The workspace header (sidebar toggle, screen label
and save/undo controls), navigation rail and bottom
status bar are hidden on this page, including when a story remains open.
Search replaces that whole heading/action row with a focused input. Filtering
matches project names, locations, saved story content and asset names/descriptions,
case-insensitively; multiple words must all match. The cached index is refreshed
on returning to Projects or using Refresh, without file reads or copying story
indexes on each keystroke. Esc or the trailing × clears the filter, restores the heading/actions
and full list, and returns focus to Search. Empty results are explicit and result
counts are announced. The search action does not open the command palette;
the global command shortcut remains available. Project rows have a subtle hover
background/border highlight and a matching keyboard-focus treatment. Task owners
and open story state remain mounted across navigation.

The page shows the project list and its empty/error states. Add opens the shared
native dialog with the story-name form; creation, opening an existing file and
standalone-project adoption live there. Sample-project creation is removed from
the home UI; the driver fixture remains. Enter submits creation, Escape/Close
returns focus to Add, and closing retains the typed name. Failures retain the
form with local recovery feedback and direct technical Details. The permanent
creation sidebar and duplicate Recent toolbar are removed. Manual UI acceptance
is assigned to the user under SF-PLAN-11.
Workspace tests/all-target checks, strict Studio Clippy, formatting, regenerated
Tailwind styles and the Studio desktop build pass. A saved-project fixture tests
name/content/asset/location filtering, Unicode case, combined terms, no matches
and refresh after source edits. Native row, focus and Esc acceptance remains
assigned to the user.

### Shared asset definitions · 2026-11-01

Valid explicit DSL declarations create pending catalog identities without
rendering; unfinished/invalid source retains the previous catalog. New artifact
and variant forms default to definitions, with an explicit Generate an image now
checkbox. Names follow DSL identifier rules. Inspector Rename updates the exact
resolved references and catalog entry together in one undo, preserving prose.

Atelier's shared-project Artboards toolbar opens Assets. The panel creates and
describes items and variations, attaches existing image revisions and links
artboards, using the storage worker with retained forms on errors. An item used
by saved source is renamed in Studio. Illustrations have their own catalog
identity rather than masquerading as a scene/beat. Catalog edits never generate
images or rewrite Studio source from Atelier.

Checked GUI writes reject stale snapshots without overwriting newer work.
Atelier Refresh retains same-project form text for retry. Studio initially loads
external changes on reopening; live notification, merge and conflict recovery
remain SF-PLAN-04. Storage/catalog tests cover creation from both apps, missing
artwork, invalid source, rename/undo, stable variation/image links, migration,
stale saves, worker events and relocation. Native UI acceptance is assigned to
the user. See [catalog contract](docs/shared-project.md#shared-asset-catalog).
Workspace tests/checks, formatting, strict changed-package Clippy, the Studio
desktop build and Atelier's macOS development packaging pass.

### Shared project folders · 2026-11-01

New Studio stories create portable folders. Projects → Open accepts the shared
`storyfella.toml` manifest as well as existing `.sf`/`.db` files; Recent discovers
shared folders. An open standalone project offers Copy to project folder with a
native parent-folder picker. This copies source, registry identities and artwork,
opens the copy on success and retains the original on failure. Existing folders
are never overwritten. Atelier opens the same manifest and saves its working
document inside the folder. No model call or export/approval gate is involved.

A shared folder already open for editing in another Studio window shows an
open error with instructions to close it in that window and retry. Its saved
story remains searchable in Projects. Automated command tests cover blocked
opens while the first window has a dirty draft and successful open after
that owner switches away.
Studio uses the same conflict notice when a chat's MCP session owns the
project. The stdio process test verifies that closing the session releases
the lease and another owner can open the saved project.

Deterministic storage/adapter tests verify relocation with original paths removed,
exact source and image retention, saved Atelier documents, invalid versions/IDs,
missing source/images, immutable revision conflicts and escaping resource paths.
Manual UI acceptance of the create/open/copy flows is assigned to the user.
See [storage contract](docs/shared-project.md) and SF-PLAN-02.

### Project navigation availability · 2026-11-01

Story, Canvas and Preview appear in the left navigation only while a project is
open. Projects, Plans and Settings remain available. The command palette and
Story/Canvas shortcuts obey the same prerequisite. Visiting Projects retains the
open story; the Projects home now hides workspace navigation as described above.
Selecting an artifact is not required. Existing
mounted task owners and drafts retain their lifetimes across navigation.

Palette regression coverage checks the destinations before and after project
opening. Workspace checks/tests, formatting, strict Studio Clippy and the desktop
debug build pass. Manual UI acceptance is assigned to the user under SF-PLAN-11.

### Writing response boundaries · 2026-11-01

Recorded user attempts exposed a false repair cycle: model responses omitted
the captured block's final newline, joining a closing dialogue quote to the next
beat declaration. Another response trimmed only the first line's indentation.
Writing now restores captured block indentation, line endings and terminal blank
lines before compiler validation. Partial-line selections retain their text-edit
semantics; malformed DSL is still rejected. Repair requests contain a structured
original request, boundary previews and offending lines with line/column
diagnostics. Progress states show the repair attempt out of the fixed limit.

Offline replay of seven recent rejected outputs now compiles without another
model call. An older output containing unlabelled prose remains invalid, showing
that normalization does not bypass syntax checks. Synthetic regressions cover
first-line trimming, whole-block dedenting, CRLF, terminal blank lines and
partial selections. Workspace checks/tests, formatting and strict Studio/core
Clippy pass. Replay/logs live under `.svs-e2e/workflows/`; user source fixtures
remain ignored and the original project was not modified.

### Editor diff review · 2026-11-01 (current)

This supersedes the separate suggestion reading area below. A completed writing
request opens a continuous CodeMirror merge editor in the existing story area.
Inline review shows read-only deleted lines above editable proposed lines;
side-by-side review aligns the accepted draft and editable suggestion. Both use
the story's font metrics, indentation and reference-aware syntax colors, subtle
block backgrounds, line numbers, a reset arrow and a selection checkbox per change.
In side-by-side review, original and suggested line numbers flank the centre
divider. The reset arrow sits left of the original number column; the checkbox
sits right of the suggested number column. Inline review retains both controls
in its left-hand number gutter.
Word-level highlighting is disabled in both modes: changed text has one subtle
full-width block background, without additional backgrounds behind words or
wrapped text runs. Native text-selection highlighting remains available.
The existing workspace bars are retained. The editor toolbar switches views;
the request panel owns change navigation, Keep/Reset selected actions, Try again,
Keep all and Discard all. There is no floating action bar over the story.
Writing help uses the shared icon-action component: compact edit, retry, reset and
previous/next icons; labelled keep, discard and recovery actions; every
action has a tooltip. The panel's guidance is brief and acceptance buttons fit
their labels. Review shows the user's prompt once, without a separate Rewrite/
Expand label, repeated change-count heading or generic editing instructions.
The navigation row supplies the current change and total. Navigation and
shortcuts stop at the first/last difference.
Edit request and Try again are adjacent icon actions in the panel header;
review, interruption and failure states share that single retry entry point.
Keep is disabled when the chosen change/group would introduce source errors,
and Discard all is disabled when no pending differences remain.

Writing help scope includes Selection, Current beat, Current scene and Entire
file for both Rewrite and Expand. Entire file includes all scenes, comments and
file boundaries, independent of the caret; retry retains that whole-file scope.
The normal Writer uses one compact action row: editing tools on the left,
Problems with diagnostic counts and the current line/total on the right.
Expandable tools open their larger content below the action row, with named
regions and expanded/control associations. Problems and reference content are
hidden when closed, without separate permanent status rows. Future toolbar
toggles should follow this same pattern.

Mode switches preserve proposed text, selection and editor undo history. Manual
edits anywhere in the proposed version remain pending until kept. Manual
text is also protected across Try again: combine non-overlapping edits,
but retain the current review and explain a conflict when a new rewrite
overlaps them or makes their combination invalid. Do not insert merge markers
into story source. Authorship indicators should use a subtle gutter marker
and tooltip if introduced; avoid another background layer over the diff.
The accepted draft alone feeds normal saving and Preview. Rust owns source/project guards,
compiler validation, reconciliation and project undo. Writing workspace changes
autosave beside the project, including unsaved accepted text, pending manual
edits, checked changes, request controls and partial streamed responses. Reopening the project
restores review without another model call. Interrupted generation offers an
explicit retry; an external source conflict offers restore-as-unsaved-draft or
discard-saved-session before autosave can replace the checkpoint. Storage
failures are shown in the request panel. Keeping a change is one
undoable edit; Keep all is one edit for the remaining changes. A structural edit
which depends on another change cannot be kept alone: it must be corrected or
kept with the related edits. A changed draft or project pauses application and
retains the comparison. Retry uses the current accepted draft and preserves an
existing comparison on cancellation/failure.

Native macOS release checks with a disposable local fixture verified continuous
editing, Inline/Side by side switching, editor undo/redo across a view switch,
partial Keep followed by Discard, accepted-source-only autosave, invalid manual
edits blocked by both acceptance paths, retry and one-step Keep all/Undo. Shared
core tests cover ranges, insertion/deletion, ownership, CRLF/Unicode and retry.
Evidence: `.svs-e2e/workflows/`. Other-platform and full assistive-technology
acceptance remain open.
The long-document pass also verified aligned unchanged text after the inserted
passage on a shared outer scroll surface. Gutter actions are exposed to the
accessibility tree and were activated using Tab/Enter. The same pass corrected
side-by-side clipping caused by trying to scroll CodeMirror's inner views.

### Writing review and error recovery · 2026-11-01 (superseded panel layout)

This supersedes the Writer panel presentation below. Scope and action share a
compact row above a short instructions field and an explicit Allow new details
checkbox. Completed requests collapse to a brief with Edit request. The reading
area uses the available panel height, with Suggestion/Original comparison and
fixed Accept suggestion/Discard actions. Source text has one scroll container;
normal review contains no filesystem paths. Closing returns focus to the source.

Review groups new compiler notes and suppresses warnings already present in the
captured draft. Generation validates the complete proposed document and repairs
invalid output automatically, with at most two repair attempts. Only valid
results reach review; exhaustion becomes a contextual recovery notice. A draft
with existing source errors is rejected before model submission. Live registry
changes can still invalidate a retained suggestion, so acceptance rechecks the
current compiler state. Operational
failures appear once in the panel with retry, settings and Details, preserving
the request and any previous suggestion. Technical reports use a neutral dialog
with collapsed request context. Startup configuration failures use the same
recovery treatment, with a readable summary and Open configuration/Try again.

An isolated macOS release build with a local streaming fixture verified a long
suggestion, scrolling with actions pinned, Original comparison, acceptance and
one toolbar Undo, a single HTTP-failure notice and its details dialog. A malformed
scratch config verified the startup recovery layout. No real provider call was
made in these UI checks. Regression coverage verifies grouped new notes,
suppressed existing warnings and blocking source errors. Workspace checks/tests
and strict Studio/core Clippy pass. Other-platform and assistive-technology
acceptance remain open.

The validity gate was also exercised in the native release build: a deliberately
malformed first response was repaired into a valid, acceptable proposal. Three
invalid responses produced one recovery notice, no proposal and no Accept
button, with source unchanged. Offline tests verify repair lineage in storage,
bounded exhaustion, cancellation and existing-source preflight.
Pre-commit review also corrected escaped DSL examples in the system prompt,
removed unused workflow helpers and verified cancellation immediately before
publishing an otherwise valid suggestion. Required workspace validation and
strict Clippy for all four changed Rust packages pass; vendored Cocoa warnings
remain unchanged.

### Hosted configuration and Studio startup · 2026-11-01

Current direction (SF-PLAN-31): Studio Settings has one Models page for its
text and vision choices, saved in Studio's application configuration. Provider
connections, credentials, model availability and services are managed in
Clipforge; Studio offers a handoff and refresh. Studio does not expose image
model setup or a Services page. The implementation and acceptance are tracked
in the roadmap; the historical record below describes the earlier interface.

A valid shared hosted image default (including Replicate), or a text-only
registry, allows Studio to open. Writer, playback and text-provider Settings
remain available. Native image controls and model downloads are shown only
when the selected t2i provider resolves to a native deployment; otherwise Models
and Services explain the available authoring workflow and Atelier's hosted-image
role. Prewarm skips an unselected native engine. Explicit native-only requests
report SVS-CFG-110 before touching servers or downloads. Shared defaults,
credentials, models and malformed-configuration checks are preserved.

Regression tests cover hosted and text-only registries, retained text bindings,
unchanged shared settings, denied native operations/downloads and idle health
without a setup prompt. An isolated macOS dx build with a Replicate default and
prewarm enabled opened Writer/Writing help and both Settings pages; Services
showed the native-engine explanation without server/download controls. No
provider generation was submitted. Workspace tests/checks and strict changed
package Clippy pass; existing vendored Cocoa deprecation warnings remain.

### Writer assistance · 2026-10-31

Missing text setup is disclosed in the panel before submission, with an Open
Settings action and disabled Generate button. A readiness check also guards
dispatch, so this condition does not become a generic planning failure. Retained
request feedback uses the source filename, with its full path only in a tooltip.
Settings keeps the provider selector visible when exactly one chat provider is
configured, so a newly added first provider can be assigned to Text or Vision
without creating a second endpoint. Local LM Studio setup was verified with
`google/gemma-4-e4b`: Studio reported the selected text binding loaded/reachable
and a minimal local request completed successfully (2026-11-01).

Writing help replaces In scope in the existing right-hand slot. It keeps the
source editable while Rewrite/Expand runs, shows streamed feedback, and presents
the original with its proposed replacement, diagnostics and new requirements.
Accept is disabled for source/project mismatch or compiler errors. Reject and
request another version remain explicit; acceptance is one undoable source
edit. Request failures use the shared error report and retain instructions.
The native textarea keeps its selection and typing behavior. The panel stays
mounted through workspace navigation so delayed results retain their owner.

Offline regression coverage verifies snapshot guards, cancellation, error
preservation and acceptance/undo. An isolated macOS dx build with a local
streaming fixture verified the panel, beat rewrite/review, Accept and one Undo,
running Cancel with retained instructions, delayed reply after source edits
(Accept disabled), Reject and invalid-source diagnostics. The pass fixed a
desktop query-lifetime failure before acceptance. No real model request was
submitted. Other-platform, smaller-window and assistive-technology acceptance
remain open; see [workflow contracts](docs/workflows.md) and SF-PLAN-28.

### Atelier native reference capabilities · 2026-10-31

The existing reference picker and queue preflight read Agentkit's profile
capabilities. Qwen accepts a source plus two ordered references and retains PNG
alpha; Krea remains generation-only. Excess inputs and unsupported combinations
fail before startup. Settings describes reference editing without an obsolete
single-source claim. These changes use the existing controls and feedback.

Offline queue-contract tests cover accepted and rejected combinations. Real
Metal runs verified RGBA generation/editing and the shared client's three-input
path; all three outputs survived Atelier result attachment and project reopen
with encoded bytes and provenance intact. See the
[capability probes](../storyfella-canvas/docs/atelier.md#native-image-capability-probes). Live UI
interaction with multiple references and visual edit fidelity remain unverified.

Qwen's optional native mask now anchors protected source latents. The existing
Use mask tooltip explains that unmasked areas may change slightly. Native source,
mask and output dimensions must match; validation runs before server startup.
Controlled black/white/half-mask checks and a shared-client run with two RGBA
references verified the mechanism on Metal. All four PNGs survived Atelier result
attachment and project reopening. The far-protected region matched the all-black
reconstruction but differed from the source, confirming that native masking does
not promise exact protection. See [mask acceptance](../storyfella-canvas/docs/atelier.md#native-mask-acceptance).
Tooltip interaction and artistic edit quality have not been visually accepted.

### Atelier side panels · 2026-10-25 (current)

Activity, Settings, Brushes, Canvas & project, Practice, References and the
Layers/References inspector share one panel frame: 360px wide, docked at the
right edge, white background, left divider, 16px padding, a fixed title/close
header and a scrolling body. No floating offsets, rounded cards or drop shadows.
Settings alone adds its pinned Save footer. Reference-picker thumbnails fit two
columns within the shared width. Auxiliary panels replace the drawing inspector
and restore its visibility preference when closed. Close returns focus to the
workspace; Escape also closes a panel from its controls.

App-wide Settings is available from the Atelier application menu and ⌘,
(Ctrl+, elsewhere), including on Projects. Generation contains a numeric
**Request time limit (minutes)** field, default 30, bounded to 1–1440 minutes.
The limit is saved globally and captured by new queued requests; the panel
explains that queue waiting does not count. Existing requests keep their limit.

Isolated native checks verified Settings, Activity, References, Brushes, Practice
and the drawing inspector in the shared slot, including replacement and closing.
Workspace checks/tests/format, strict Atelier Clippy and packaging pass.

### Atelier Activity and failure feedback · 2026-10-25 (current)

Operational failures live in Activity, reached from a single header control on
every page. The workspace shows short contextual hints, not provider responses
or diagnostic paragraphs. Generation hints belong to their project and prompt
source/mode. A failed save retains a Not saved hint beside the project tabs.
Prompt validation remains beside the composer input. The footer stays quiet.

Activity reuses the image queue and its captured requests. A report shows one
short explanation, an applicable recovery action, Details and dismissal. Details
opens selectable, read-only diagnostics in a bounded field with verified Copy
report. No timestamps, filters, routine-save feed, stacked cards or intermediate
toasts. In drawing, Activity temporarily replaces Layers/References; closing it
restores that inspector preference. Escape closes Activity from its report field.

Report titles use a compact, left-aligned semibold heading with a small status
icon. Project ownership and attempt counts are secondary metadata. Recovery,
Details and Copy share one wrapping action row. Expanded diagnostics use a short,
neutral read-only surface without repeated report headings or project labels;
Copy report retains the complete report. Clipforge launch failures are app-scoped
with a specific title and Try again action, and resolve after a successful launch.
Native checks verified the missing-app report, Details, copy and repeated launch
attempts. Workspace checks/tests/format, strict Clippy and packaging pass (64
Atelier tests passed; 14 device/fixture tests ignored).

Reports retain operation/project ownership through navigation. Exact repeats and
retried attempts group; unrelated successes do not clear failures. Dismissing a
notification does not claim unsaved work is saved. Retry uses captured inputs,
and a successful attempt resolves its report. Do not invent retry countdowns or
automatically resubmit paid requests. Reports and queue history are session-only;
documents, prompt drafts and originals keep their existing persistence.

Each queued or running request exposes its own **Abort** action. A queued abort
stays in history as **Aborted**; a running abort stays **Aborting** with its
action disabled until the worker acknowledges cancellation. Pause remains a
queue-wide dispatch control. Aborting does not change other requests or pause
state. Native sampling may finish remotely after waiting stops; the result is
discarded. Isolated mock UI checks verified both actions without real inference
(2026-10-30), alongside queue regression tests and strict Atelier Clippy.

Native fixture checks verified long mocked 429 diagnostics, quiet footers,
details/copy/read-only/Escape, separate save failures, project isolation, grouped
retries and successful recovery using the originating request. The final layout
was checked with the drawing workspace and a narrower window. Workspace
checks/tests/format, strict Atelier Clippy and packaging pass (63 tests passed,
14 device/fixture tests ignored). Real-provider, VoiceOver and other-platform
acceptance remain open; the mock establishes no artistic quality.

### Atelier project tabs · 2026-10-25 (current)

Project cards on Home are single actions: clicking the preview, title or padding
opens the document. Each card is one keyboard-focusable button named for its
project, with hover feedback and a focus ring. There is no nested Open project
button.
Native verification confirmed card activation opens the document and the gallery
renders without the redundant button. Workspace checks/tests/format, strict
Atelier Clippy and packaging pass; keyboard behavior uses the shared Button.

The logo, project tabs and icon actions share one compact header row. Clicking
Atelier returns to the Projects home page, where Library is available; separate
Projects/Library header links and the repeated project title are removed. Each
tab gives its title, edited dot and close icon one shared surface; close hover
changes the icon color without a button background. Tabs scroll when space is
limited. Workspace checks/tests/format and strict Atelier Clippy pass. An isolated
native build rendered the combined header and home-page Library action; automated
interaction checks were limited by intermittent native-window access. Close-hover
and navigation acceptance remain pending.

Project tabs keep their chosen order when activated. A compact close icon saves
the project before removing its tab; save failure or a newer edit keeps it open.
Drag a tab to reorder it, or use Ctrl+Shift+Page Up/Down. Cmd/Ctrl+W saves and
closes the active tab. These actions also have named File menu entries. Keep
the strip quiet: no permanent reorder arrows or document-management panel.
Closing the last tab returns to Projects. Library remains independent, with
shared-image status, a compact Favorites filter beside search and no drawing
rail or project title/save/export controls. Save errors remain visible from
Projects and Library as well as the artboard workspace.

Recovery stores display order separately from active identity, upgrades earlier
sessions and preserves the empty session after all projects close. Native checks
in uniquely identified Project Tabs builds verified drag/keyboard reordering,
active/inactive close, failed-save retention, draft reopening, last-tab close,
ordered restart recovery and Library access without a project. Workspace
checks/tests/format, strict Atelier Clippy and macOS packaging checks pass
(Atelier: 60 passed, 13 device/fixture tests ignored). Real-provider, full
accessibility and other-platform acceptance remain open.

### Atelier composer sizing · 2026-10-25 (current)

The prompt and wine Create action lead the composer. Keep provider and aspect
choices readable in the main row; put resolution and sampling in Image options.
Expanded options align fixed-width pixel inputs, presets and sampling in one
wrapping row. Numeric fields must not stretch to consume
unused space. Secondary actions fit their content; creation briefs form a quiet
Start with row. The ellipsis opens a dropdown with Image options and Provider
settings. Image options toggles request controls; Provider settings opens the
settings panel. Clipforge management and Reload providers live only in Settings.
The composer has no setup action row, readiness footer or repeated setup-required
callout. Submission validation remains beside the prompt. Image options appears
only when the current mode/provider has request controls to show.

Isolated native checks verified the dropdown, its Settings destination and the
drawing composer without setup rows. Workspace checks/tests/format, strict Atelier
Clippy and packaging pass. No provider requests were submitted.

The uniquely identified Atelier Composer Final build was checked with hosted
and local provider selections, expanded options and editable dimensions/prompt.
No generation request was submitted. Workspace checks/tests/format, strict
Atelier Clippy and macOS packaging checks pass. Smaller-window and accessibility
acceptance remain open.

### Atelier visual refinement · 2026-10-24 (current)

Atelier's UI should be modern and minimal, with composition and hierarchy decided
before adding controls. Keep artwork primary, use the shared spacing and control
vocabulary, and disclose secondary tools at the point of use. A working feature
is not visually accepted merely because its controls fit somewhere.

The drawing inspector now uses one neutral surface with Layers first and a
separate References section. Layer selection exposes compact Move and opacity
controls. Layer properties discloses precise offsets, relative scale, opacity,
ordering, deletion and image/version actions. Candidate review retains a clear
Accept action and named reference, hide/show and discard alternatives. Reference
tools disclose beside their preview; hiding references collapses the preview.
An empty reference section offers one Add reference action without reserved
image space or disabled removal controls. Study controls live in Practice,
reached from the timer/help rail, with remaining time visible while running.

The shared prompt keeps source/destination and dimensions together, followed by
compact request controls and one primary submission action. Image options
discloses exact sizes, sampling and briefs. Provider setup lives in Settings,
reached from the ellipsis menu. Chosen references are ordered compact chips.
Missing provider setup appears as validation when a request is submitted.
The composer itself blocks canvas input; its surrounding stage
remains usable. Avoid parallel Settings/Options control stacks and repeated
explanatory paragraphs in the default workspace.

Native checks in the uniquely identified **Atelier Minimal UI** fixture build
verified the compact inspector, reference collapse, candidate hide/show,
accept/undo, precise opacity application/undo and prompt/options disclosure.
These checks used solid-color fixtures without a provider request. Workspace
check/test/format, strict Atelier Clippy and native packaging pass; Atelier has
57 passing tests and 12 ignored device/fixture tests. Smaller-window, keyboard-only,
VoiceOver and first-time-user acceptance remain open.

### Atelier creation workflow · 2026-10-24 (current)

Projects contain real artboards with artwork layers and separate reference
associations. New compositions create image-backed artboards. Full-artboard edits
and selection blends arrive as pending layers with Accept, Hide/Show, Save as
reference and Discard controls. Pending candidates and references are excluded
from artwork exports. Layer movement, scaling, opacity and visibility remain
undoable; Move layer reveals an outline and corner handle, with precise numeric
values in the layer dock. B/E/H and bounds/scale canvas resizing are preserved.

The existing floating prompt is now one implementation for new compositions,
full-artboard edits, selection blends, separate images and inspected-image
variations. The mode states the destination before submission. Selection blend
sends the full accepted artwork as context and crops only the chosen rectangle
into a candidate; separate-image mode sends the crop and exposes output size.
This does not establish seam quality or identity fidelity. Unsent drafts retain
their project, artboard/source, coordinates and mode through saves and recovery.
Options disclose precise output size and creation briefs; routine controls stay
compact. Missing provider setup remains actionable; the Clipforge handoff lives
in Settings.

Library is app-wide. A separate picker combines project references with shared
originals and supports import-and-select. Reference visibility and associations
belong to their artboard. Project tabs retain independent documents, history,
drafts and dirty state; session recovery restores open documents. Clipforge owns
provider connections and secrets; Atelier offers Open Clipforge and Refresh
readiness, request model choices and ComfyUI mappings. Direct setup navigation
awaits a Clipforge deep-link contract. Tab closing/reordering now follows the
project-tab behavior above.

The [Atelier reference](../storyfella-canvas/docs/atelier.md) documents the
version-2 upgrade, reused behavior, captured queue destinations and remaining
native/provider coverage. Source migration, geometry, candidate export exclusion,
history, multi-project recovery and preflight regressions pass. The workspace
check/test/format gates and strict Atelier Clippy pass (Atelier: 57 passed,
12 device/fixture tests ignored). Native macOS packaging and its packaging checks
also pass. Existing vendored GPUI
deprecation warnings remain unchanged.

Live macOS checks in a uniquely identified isolated bundle verified project
creation, draft restoration across tabs and quit/restart, drawing composer access,
a local stroke and gizmo display. This is partial acceptance: real-provider
selection requests, candidate review and reference import through the native UI,
failure injection, first-time-user, VoiceOver and Linux/Windows passes remain.
Older Atelier entries below describe implementation history and are superseded
where the workflow changed.

### Atelier project-first handoff · 2026-10-16

Atelier opens to Projects. New projects open to Artboards with a centered blank
canvas on a quiet tinted stage and a compact image prompt visible by default.
The blank canvas is an empty-state entry point: the first generated artboard
replaces it in the sequence. A canvas with actual drawing remains an artboard.
Sparse artboard sequences are centered, with artwork large enough to lead the
screen and contextual activity beside the selected artboard.
Artboards use one light neutral workspace surface from title through composer.
The artboard retains its native aspect ratio and takes the available visual
attention; the composer is a compact floating control with settings tags below
the input. Routine save state belongs in the header, without a second footer.
When artwork exists, a faint plus artboard follows the real artboards. It adds
a blank drawing canvas to Artboards, preserving existing canvas artboards. It is
excluded from the artboard count and from centered artboard layout, so it never
shifts the artwork.
The prompt keeps its dropdown controls together below the input for provider,
aspect ratio, resolution, and local sampling steps. Draw is an editing state
reached by opening the canvas artboard. Drawing controls belong in its left tool rail;
the canvas has no separate bottom brush bar. Keep the drawing rail narrow:
show a centered vertical stack of brush presets, size step actions around a
compact size tag, opacity, and smoothing. Omit the brush-name caption. Keep the
rail and the optional side dock slim, and place tool popovers beside their rail
triggers. Draw reuses the Artboards prompt composer at the bottom of the canvas:
the same input, provider, aspect, resolution, optional steps, and submit layout.
Only the submission action changes to use the drawing as its source. Keep its
top edge free of extra icon and close rows. The drawing composer adds a compact
Full/Area control alongside the shared settings, with a visible marquee after
a canvas drag. ⌘P toggles the prompt in each workspace; Artboards shows it by
default.
The image editor keeps artwork primary, opens its variation composer only on
request, and returns submitted work to the project artboard sequence. Artboard
activity is a short contextual event list with a direct
full-details action. This is source-level acceptance; live visual and keyboard
checks remain pending. See [Atelier](../storyfella-canvas/docs/atelier.md) for the workflow and
verification status.

Verification: `cargo test -p storyfella-atelier` (48 passed, 11 ignored),
`cargo clippy -p storyfella-atelier --all-targets -- -D warnings`, and
`cargo fmt --package storyfella-atelier -- --check` pass.

### Atelier game-art creation briefs · 2026-10-16

The new-image composer offers editable starting briefs for **Character**,
**Object**, **Background**, and **Illustration** work. Each brief chooses a
portrait, square, or landscape canvas that fits the asset and seeds the existing
per-project prompt draft, so users can move from a game-art intent to the normal
generation queue without introducing a second generation path. Choosing a brief
while editing returns the composer to new-image mode; the user can then refine
the prompt before queueing it. The Settings credential field remains masked and
uses Clipforge's shared provider config; environment-only keys are resolved for
requests without being copied into the stored-key field.

Verification: `cargo test -p storyfella-atelier` (48 passed, 11 ignored),
`cargo clippy -p storyfella-atelier --all-targets -- -D warnings`,
`cargo fmt --all -- --check`, and `git diff --check` pass. The four brief
buttons have not yet been exercised in a live UI session.

Atelier acceptance · 2026-10-10: the image workspace uses named icon actions for
export, canvas placement, edit mode, comparison and masking. Details and provenance
are disclosed on demand; the prompt has more space and the version strip is compact.
Isolated macOS checks exercised those controls and resizing to about 1058 × 732
logical pixels. A live Replicate forest-bookshop generation and evening edit were
saved as a portable project, original PNGs and approved artwork.
Hosted provider keys save in the shared `clipforge.toml` through Clipforge's
atomic, owner-only writer; Studio and Atelier read that shared provider config.
Linux/Windows native interaction and assistive-technology checks remain unverified.

Follow-up iteration acceptance completed two further real Replicate edits, including a branch from the original,
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

[ROADMAP §0](docs/ROADMAP.md#0-current-direction-and-actionable-backlog) owns planned delivery
status. This document owns interaction and presentation rules.
[Development](docs/development.md) documents the executable authoring scenarios,
and [Atelier](../storyfella-canvas/docs/atelier.md) documents canvas behavior. Use this standard for
interaction requirements and keep planned changes in the roadmap.

## 1. Product principles

Studio helps a person write a story, create its artwork, and test its playback.
The current project, current selection, pending work, and saved state must be
understandable without knowing Rust, the CLI, or the backend process model.

- Keep feedback beside the action and object it explains.
- A control's label promises its immediate result. One activation reaches it.
- Protect drafts and completed work across failures, navigation, and shutdown.
- Make available actions discoverable with both a pointer and a keyboard.
- Share terminology, state, and behavior across equivalent surfaces.
- Favor minimal interfaces with a clear hierarchy. Group related actions where
  the user makes the decision; show each action once in the current task.
  Remove repeated headings, counts and boilerplate. Keep the user's work and
  relevant state visible, with detail available on demand. Minimalism must
  preserve discoverability, accessible names and useful feedback.
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

### Icon actions and labels

Prefer compact icon actions throughout the workspace, using the shared icon
and button components. Every icon action must have an explicit tooltip naming
its immediate result and an accessible name, whether or not a label is visible.
Use icon-only buttons for familiar local actions such as previous/next, edit,
retry, close and reset. Keep icon-plus-label buttons for primary submissions, group
acceptance, destructive or ambiguous choices, and recovery actions. Do not
replace meaningful labels with unfamiliar symbols or rely on tooltips as the
only explanation of a consequential decision. Keep icons, focus treatment and
terminology consistent across equivalent actions; retain keyboard operation.
Disable an action whenever its prerequisites are unmet or it has no target.
Navigation stops at its boundaries: Previous is disabled on the first item,
Next on the last, and both when there is only one item. Keep unavailable
controls visually subdued and semantically disabled; handlers and shortcuts
must obey the same availability rules.

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

### Studio visual-production cutover · 2026-11-02

SF-PLAN-07 leaves Studio's Canvas focused on catalog definitions, saved previews,
read-only image history and story references. Names/descriptions and explicit
variation definitions remain authoring actions. The shared Open in Atelier
action owns visual creation/editing; ordinary linked Save refreshes Studio.
No Studio control generates, edits, promotes, activates or reruns image revisions.
The former image composer/import form and request size/steps/LoRA settings are
removed, including their keyboard and guide entry points. The palette supports
navigation and authoring actions; `/` opens it without changing screens.
New definitions use the shared native input vocabulary and never invoke inference.
Saved active-image selection is consistent across gallery, details and Writer.

Existing images/history/provenance stay in projects. Machine service/model-file
administration and Writer's writing assistant remain. The implementation records
below describe their dated audit/remediation state; references to the removed
Studio composer and production controls are historical. The user verified the
Atelier Save → Studio refresh round trip before this cutover. Native verification
of this simplified UI is pending; automated evidence belongs to SF-PLAN-07 in
[the roadmap](docs/ROADMAP.md).

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
Linux/Windows native UX were not exercised in this pass. See `../storyfella-canvas/docs/atelier.md`
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

### Atelier generation queue · 2026-10-12

The Atelier generation rail now queues requests instead of running one at a time.
**Add to queue** validates and captures the request — provider, model, prompt,
width, height, sampling steps and the applied LoRA adapters with their weights —
and runs it after those already waiting, so several images can be prepared before
any finishes. Exactly one request runs at a time on the shared server. **Pause**
holds dispatch without interrupting the running image, **Stop** cancels it, a
queued request can be removed, and **Clear finished** drops completed history.
Each entry keeps the values it was created with, so later control edits never
change work already queued. The list labels every state in text (Queued, Running
with its progress, Done, Failed, Cancelled) and pairs each with a colour, never
colour alone.

Size, steps and LoRA adapters are per-request controls in a floating **Image
settings** window (a gear in the rail);
Settings keeps the provider, model, endpoint and credentials. This is a
state-only change: `queue.rs` owns the request snapshots,
ordering, pause and states, and the app dispatches one item at a time through the
unchanged `worker::generate`, so no second execution path was added and the
single-GPU constraint holds.

The queue now lives in its own panel — opened from a header badge that counts
outstanding requests (queued + running) and from the rail's summary — with
Pause/Resume, Stop and Clear finished, and one card per request showing its state,
prompt and settings. The generation rail keeps only the composer and the summary.
**Generate from drawing** now uses the bottom floating composer shared with
Artboards. Its Full/Area control switches between the full canvas and area
selection; a drag shows the selected region. Submission rasterizes the area
(cropped to the region), opens it as the edit source and queues it. Rails and the header were
narrowed, and explanatory copy was removed to keep the workspace out of the way.

Verification: `cargo test -p storyfella-atelier` (49 tests; seven new — six
covering queue ordering/pause/history/removal/masked-edit size, one covering
region clamping) and `cargo clippy -p storyfella-atelier --all-targets -- -D
warnings` pass. The queue panel, header badge and drawing composer
have not been exercised in a live UI run.

### Atelier shared reference library scoping · 2026-10-12

The Atelier reference library is now the **shared collection**, not a record of
every image ever made. Generated images, edits and project imports stay in their
project; only an import into the **All projects** tab or an explicit **Add to
reference library** action puts an image in the collection. Renaming or favoriting
a project-only image is a project-local change and a no-op in the library, and the
image workspace now persists a rename/re-tag to the project's own record and its
cached preview label, so a project-only image keeps its details across reopen.
The image workspace's version strip, before/after comparison and draft restore now
resolve images from the project and the collection together (deduplicated by
content), so project-only variants appear in their family and remain comparable.
Existing images already in the collection are left untouched — no destructive
migration.

Verification: `cargo test -p storyfella-atelier` (48 tests; 2 new covering that a
project-only image is not shared until promoted, and that the library loads
without collecting project images) and `cargo clippy -p storyfella-atelier
--all-targets -- -D warnings` pass. The promote action's live UI behaviour is not
yet exercised. The test helper now resets its pid-keyed temporary directory so a
run cannot inherit a fixture left by an earlier one (the pid is reused and TMPDIR
persists).

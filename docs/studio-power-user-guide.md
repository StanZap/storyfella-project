# Studio power-user guide and workflow assessment

Reviewed **2026-10-10**, against implementation baseline **3952830**.

This guide describes what the current app can do, how to combine those features,
and where the experience still breaks down. It distinguishes Studio from Atelier
and implemented behavior from proposed improvements. It is a code-backed workflow
review, not a claim that every AI example below was executed successfully. Recent
isolated macOS acceptance covers forms, command entry, import, Writer navigation,
rename/undo, diagnostics and save/quit. Fresh generation quality, planner behavior
for the example requests, other platforms and screen readers require separate
acceptance. No paid requests were submitted for this guide.

## 1. What you are looking at

In the supplied screenshot you have opened the **environment artifact
`harbor_dock`** in the project's Canvas. The center is its image preview. The
right inspector contains its canonical description and generation settings.
The bottom strip holds its base and visual variants. The bottom entry searches
actions and artifacts, accepts explicit commands, and can send requests to the
text planner. The selected artifact supplies the default target for requests.

The `Appears in harbor · line 6` link returns to the statement in Writer that
uses this environment. `open harbor` and `open harbor.arrival` open the associated
scene and beat artwork entries; they do not enter playback. The artifact count
counts registry entries, not generated images or completed scenes.

**No image yet means this entry has no displayable image.** You can describe and
generate it, or import approved Atelier artwork as a completed artifact. There
is nothing here yet for an instruction edit to modify.

The immediate practical workflow is:

1. Check the description. If it is empty, describe the dock, architecture,
   composition and intended illustration style.
2. Apply the description, then open the refresh action or type `render`.
3. Submit the generation form. Opening the form alone does not render anything.
4. Once an image exists, edit it, create alternatives, and inspect revisions.
5. Use `Appears in` to return to the story, then test the ready story in Preview.

**UX problem:** the screen calls the first-image action *Regenerate*, including
the empty-state instruction. The form says *Generate image*, but the entry
points are inconsistent. The empty preview should offer a direct **Generate
image** action, with **Edit image** appearing once an image exists. The large
empty surface currently makes the app feel less capable than its actual actions.

## 2. Know the different things you are editing

| Thing | Purpose | What changing it does |
| --- | --- | --- |
| Project | The working collection of story source, artifacts and revisions | Establishes the context for the session |
| Writer source | The authored `.sf` story: scenes, beats, dialogue and branches | Changes story structure and execution |
| Character / environment / object | A reusable visual subject | Supplies artwork used by the story |
| Canonical description | The subject's persistent identity and look | Changes the anchor used by later generations and edits |
| Variant | A separately named alternative of a visual subject | Preserves another look without replacing the base artifact |
| Revision | An image attempt within one artifact or variant | Lets you choose which completed image is active |
| Scene / beat registry entry | Artwork metadata associated with a Writer declaration, or a manually created library entry | Does not independently rewrite the authored story |
| Generated text draft | An AI-produced text note attached to an artifact | Does not automatically become Writer source |

For example, `harbor_dock` is a place. A rainy version is a variant. Three image
attempts at that rainy version are revisions. `scene harbor` describes where
that place is used in the authored story. These identities are related, but
they are not interchangeable.

**UX problem:** *New story* has different consequences in different places.
Projects → Start a new story creates a project. Typing `new story` in the bottom
entry opens a form for a Story registry entry. Neither means “write a complete
playable story.” Project creation should be named **New project**, and library
text entries should have a distinct name and purpose.

## 3. Choose the fastest interaction for the task

### Direct controls

Use the toolbar and inspector for a task on the artifact already open. The
detail toolbar offers New variant, Regenerate, Edit image when an image exists,
and toggles for Variants, History and Inspector. Hover reveals action names and
available shortcuts. The inspector is the reliable place to change the
canonical description, sampling steps, size and adapter bindings.

### Bottom action entry

Focus the entry and type short action words:

| Type | Expected route |
| --- | --- |
| `new environment` | Creation form with Environment selected |
| `new character` | Creation form with Character selected |
| `variant` | Variant form for the selected visual subject's base |
| `render` or `regenerate` | Generation form for the selected visual artifact |
| `edit` | Edit form, or prerequisite feedback if the image/selection is missing |
| `harbor_dock` | Matching artifact-opening results |
| A full instruction such as `Add warm lantern light to harbor_dock; keep the buildings and viewpoint` | Assistant request route, subject to the selected suggestion |

Use Up/Down to choose a suggestion and Enter to activate it. A task suggestion
opens its form; an artifact suggestion opens that artifact. An assistant request
first opens a proposed-operation review after the planner responds. On macOS,
**Cmd+Enter** submits through the command/request path explicitly. Non-slash
input goes to the planner; slash input goes to the command parser.

Escape dismisses suggestions without clearing the text. In raw command mode,
Tab accepts a visible completion; Shift+Tab remains focus navigation. Outside
raw command mode, Tab moves focus normally. Shift+Enter can insert a newline,
but multiline text is still one submitted request, not a command script.

### Global palette

Use **Cmd+K** to find screens, Save, Undo/Redo, artifacts, contextual actions,
and advanced slash operations. Common operations open forms; some advanced
ones prefill the bottom entry so you can supply their arguments.

**UX problem:** the bottom entry and global palette overlap substantially.
The blank bottom popup mixes global navigation, current-artifact actions and
artifact search. The meaning of Enter depends on the selected row. A search
icon does not explain an AI request route, and a chevron does not explain
whether it will open a form or start work.

The recommended division is **Cmd+K for navigation/find/global actions** and
**the bottom entry for requests and actions on the current selection**. Its
active action and target should be obvious. This division is a recommendation;
it has not been implemented as a fully separate pair of catalogs.

## 4. Generate and refine a useful piece of art

Suppose you need a harbor background for a lantern story. Give the canonical
description durable traits:

> A wooden harbor dock beside weathered coastal houses, a lighthouse in the
> distance, eye-level view from the pier, hand-painted storybook illustration.

Generate the first image. If it is unsuccessful, correct the description or
generation setup and generate again. Failed visual creation can leave an
artifact with a failed revision; it does not guarantee that no entry was made.
Check the gallery before creating a duplicate.

For a local adjustment, open Edit image and give a bounded instruction:

> Add warm light from the windows. Keep the dock layout, lighthouse, camera
> position and illustration style.

The current active image becomes the edit reference. Studio grounds the edit
against the description, using the text model when available and a deterministic
fallback otherwise. A direct edit form does not require the planner to translate
the whole request into an operation stack, although grounding may still use it.

**Limit:** this is instruction editing, not a pixel editor. Studio has no brush,
mask, selection rectangle, crop handles, movable image layers or protected-pixel
compositing. A request to change only one window is an instruction to the model;
it is not a strict pixel boundary. Inspect the result for unwanted changes.

**Important:** a changed image does not automatically change the canonical
description. If warm windows should persist in future edits, update the
description to include them. Otherwise later grounding can reintroduce the old
look. Image activation and canonical-description editing are separate decisions.

**UX improvement:** after a successful edit, offer **Keep as experiment** and
**Update the subject's description**, with a reviewed description change. The
current “canon” badge and “Promote” terminology demand too much internal knowledge.

## 5. Preserve alternatives with variants

Use variants for alternatives you expect to revisit: daytime/rainy/night harbor,
formal/work clothes for a character, or intact/damaged versions of a prop.

Open New variant, give it a meaningful name, and describe the difference:

> Night harbor: same dock and houses, blue evening sky, warm lanterns reflected
> in the water, retain the hand-painted style.

Creating a visual variant also renders its first image. The filmstrip keeps the
base and its variants available; `[` and `]` step through that family. Each
variant can have its own revisions and settings.

The detail action deliberately targets the **base** even when a variant is open.
Variants form a single-level family, not nested branches. To combine changes,
describe the combined look in one variant. A day → rain → night chain does not
become a nested variant hierarchy.

**Limit:** a variant made before the base has an image cannot use that missing
image as a reference. The operation can fall back to generating from its grounded
description; visual identity has a weaker anchor. Generate/approve the base first
when consistency matters.

**UX improvement:** show the source image and base name in the variant form.
Explain missing references before submission, and make naming easy. For an
environment, the positive action should read **Create alternate version** or
an equally clear label; users should not need to infer a registry relationship.

## 6. Review, restore and make changes permanent

Open History (`e`) to inspect revisions and the operation log. Click a completed
revision to make its image active. Revisions without an image cannot be shown.
The `/revert` command activates the previous completed image without deleting
the later one. Project Undo/Redo restore state rather than rerunning generation.

Use revision restoration when yesterday's image is better. Use a variant when
both images should remain named alternatives. Update the canonical description
when the subject's durable identity has changed.

Experiment revisions expose Promote. **Its current implementation copies the
revision prompt into the canonical description.** A prompt can be an edit
instruction rather than a clean description of the finished subject. Review the
resulting description, or write a complete description in the inspector instead.
Promote does not understand the image and reconstruct its visual identity.

History also shows artifact-scoped generated text under Story text and allows
rerunning applicable operations. Rerun executes again; it is not an image restore.
Recent submitted requests can be restored from the bottom entry's Recent picker,
but that list is short, session-local and reset when the project changes.

**Limit:** Studio's picture is fitted to the available space. It has no dedicated
100% image inspector, zoom/pan workspace or side-by-side revision comparison.
Selecting revisions is the available comparison technique. Undo cannot refund
inference work or erase an external provider charge.

## 7. Accomplish several things in one request

Studio can ask the LLM for a list of typed operations, show them for review,
and execute the chosen operations in order. It is a constrained planner, not a
general agent with unrestricted control over the app.

For example, with `harbor_dock` and a completed `mara` portrait already present:

> Create a rainy alternative of harbor_dock named harbor_rain. Add round glasses
> to mara while keeping her face, clothing and style. Create an object named
> brass_lantern: an antique brass lantern with warm light.

Submit as a request, then inspect every proposed operation. Confirm each target,
whether it creates a new artifact or edits an existing one, and whether it starts
generation. Uncheck unwanted operations and run the remainder. The exact plan
depends on the configured text model; this example is not a guaranteed macro.

For independent new assets, a request such as this is reasonable:

> Create an environment named lantern_lane, a cobbled coastal street at dusk.
> Create an object named brass_lantern, an antique brass lantern. Create a
> character named keeper, an older lighthouse keeper in a navy coat. Use the
> same hand-painted illustration style in each description.

This asks for multiple independent creates. It does not establish a persistent
shared style system or guarantee consistent art direction; explicitly inspect
each result.

### What makes multi-step work unreliable today

1. **Dependencies are not evaluated progressively in the review.** Each operation
   is validated against the starting registry. “Create keeper, then create an
   outfit variant of keeper” can mark the second operation invalid because keeper
   does not exist yet. Create first, wait, then request dependent work.
2. **The batch is not atomic.** Operations run sequentially and stop at the first
   failure. Earlier successful changes remain, and failed work can retain its
   artifact/revision. Inspect the outcome before retrying a whole request.
3. **Review supports check/uncheck, not an editable task plan.** There is no proper
   inline correction, drag reorder, dependency editor or preview of every result.
   Cancelling keeps your request for revision; ask again or use direct forms.
4. **The operation catalogs are not identical.** The strict planner vocabulary
   currently omits `set`, `describe` and `revert`, although direct controls and
   slash commands support them. Do not rely on a request such as “pin this seed
   and make its appearance canonical” to perform both correctly.
5. **There is no Studio generation queue/cancel experience.** Artwork mutations
   are gated while a stack runs. The current Canvas has no cancel-running-job
   control, no per-item queue dashboard, and no batch time/cost estimate.
6. **Review copy overpromises later checkpoints.** The modal says checkpoints
   still ask you, but Studio's execution worker uses automatic approval. The
   pre-execution operation review is real; a second generated-text review should
   not be assumed.

A dependable session uses small batches of independent operations, then reviews
their results before dependent edits. This workaround is a current limitation,
not the experience a polished power-user tool should require.

## 8. Write a real interactive story

Writer is the structure editor. Here is a small valid story using already-created
environment and character names. Every referenced image must be completed and
available before visual playback can start.

```sf
story "The Lanterns"

scene harbor
  location: harbor_dock
  cast: mara

  beat arrival
    mara: "The lighthouse has gone dark."
    choice "Where should we go?"
      option "Follow the coast": lighthouse
      option "Stay on the pier": pier

scene lighthouse
  location: lighthouse_exterior
  narrate: A lantern flickers behind the glass.
  end

scene pier
  location: harbor_dock
  narrate: You wait until the tide turns.
  end
```

Use two spaces for indentation. Scene/beat identifiers start with a letter or
underscore and continue with letters, digits or underscores. Choice/goto targets
are scene names. Add `prop: brass_lantern` for a prop, `mara think: ...` for thought,
or `narrate: ...` for narration.

Writer offers Ctrl+Space completion, definition/reference navigation, formatting,
scene/beat Rename, scope-based artwork links and the expandable Problems panel.
Click a diagnostic to move the caret to its source. Missing assets can be created
as catalog entries separately from rendering. No syntax error forces you to throw
away an unfinished draft.

Use the Rename action for a resolved scene/beat symbol if it must retain its
associated artwork identity. Directly replacing the name in source can detach the
old entry and create a new association. Asset renaming is not the same supported
operation as scene/beat symbol renaming.

### Things that sound supported but do something else

Creating a Scene or Beat through Canvas creates a registry/library entry; it
does not insert a declaration in Writer. A planner `compose` operation stores a
beat composition with references, but does not build a WYSIWYG layered image or
insert/reorder authored story structure. Source order drives Studio's sequence.

Asking for story text produces an artifact-attached draft visible in History.
It does not populate Writer. There is no complete “review generated story,
insert it at the caret, reconcile artwork, and preview” workflow yet.

`shot:`, `style:` and `mood:` can be authored, but their execution is deferred.
They produce readiness diagnostics and block starting a ready visual plan. They
are not currently operational camera, rendering-style or mood controls.

**UX improvement:** provide explicit **Insert scene**, **Insert beat** and
**Review story changes** actions on Writer, with an actual source diff. Keep
library metadata creation distinct. A structured scene navigator would make the
story editable without requiring every user to learn `.sf` syntax.

## 9. Preview and test the story

Open Preview and use Apply draft. It compiles the current source and checks its
exact active artwork revisions. Syntax/flow problems, missing or pending images,
unsupported directions and unreadable files prevent starting. Zero syntax errors
does not mean the story is ready to play; warnings can describe blocking readiness
problems.

Once ready, advance through dialogue and choices. Right arrow advances; Space
pauses/resumes. Playback options provide restart, end, scene jump, bookmarks,
state files, artwork inspection and the source snapshot. A bookmark is an
in-memory checkpoint; Save state/Load state use verified runtime state files.

Playback retains a fixed snapshot. Changing Writer or an active image does not
silently replace the running story. Restart story repeats that snapshot; Apply
draft starts a checked session from the changed draft. Edit source returns to
the executing line when the source still matches the retained snapshot.

The current player presents still imagery, dialogue/thought/narration and
choices. It is not a video timeline, animated scene compositor or narrated
production export. Voice, expressive media, timing/transitions and portable
packaging remain later work.

**UX improvement:** provide a compact **Ready to preview / Needs 3 images**
indicator with direct remediation actions. Keep the detailed diagnostics in
Problems; avoid turning readiness into a wall of permanent workspace copy.

## 10. Use Atelier for detailed editing and reusable images

Atelier is a separate GPUI app in the same repository. Studio is Dioxus. Sharing
Clipforge inference code does not mean they share every feature or project model.

Use Atelier when you need full-size inspection, fit/100%/zoom/pan, version
comparison, image layers, drawing/erasing, mask drafts or reference-image editing.
Its Library → All projects supports reusable images with names, tags and
favorites; Use copies a library item into a project. Original bytes are retained
and project references are embedded rather than depending on a global library
file remaining in place.

Mask and multiple-reference support depend on the selected route. The documented
workflow permits up to two additional references beside the edited source and
rejects unsupported combinations before submitting a job. It is not arbitrary
multi-image conditioning on every provider.

For Studio, inspect the version and Export for Studio, or export the painted
canvas for Studio. Import the resulting `.sfart` package in Studio, review its
preview/name/Character–Environment–Object role, and confirm. This creates a
completed revision with provenance, supports undo and needs no image generation.

**Important:** import creates a new canonical artifact. It is not a live link or
an explicit “replace the currently selected harbor_dock” operation. Inspect the
imported name and use the intended artifact in Writer; do not assume merely
importing something binds it to the story's existing pending reference.

The import currently also fills the canonical description from the package's
**provenance**, rather than reconstructing a description of the visual subject.
Review and rewrite that description before relying on future identity-preserving
edits. Provider/job metadata should remain separate from the subject's look.

Studio currently generates through its native `sd` deployment. Atelier owns
the hosted image routes, including the configured external providers. A text
planner setting in Studio does not switch Studio's image generation to Replicate.
Studio's configured vision capability supports evaluation paths; the Canvas
planner/grounding calls currently use text descriptions, not visual inspection
of the displayed image. No current provider prices are asserted in this guide.

**UX improvement:** add an explicit **Edit in Atelier → Return to this artifact**
handoff, preserving target identity, with controlled Replace/Add as variant/Add as
new artifact choices. Bring hosted image routing and the reusable library into
Studio through shared services rather than duplicating their implementations.

## 11. Tune settings without accidentally rendering

The Generation disclosure in Inspector contains size, sampling steps, seed
pinning and adapters. Steps and seed changes affect future renders. **Resize and
render** both changes the stored canvas size and starts a new image revision.
Resetting size inheritance uses the project/default size on a later generation.

Valid sizes are multiples of 32 from 256 to 2048 per dimension; steps are 1–50.
The presets are conveniences, not the entire accepted range. A pinned seed
helps controlled experiments, but it is not a guarantee of identical results
after changing model, input image, prompt, adapters or other parameters.

Attach a real adapter from the configured files/registry, then adjust its weight
or detach it in Inspector. A base binding can be inherited by variants. Removing
an inherited adapter can affect the base and its other variants; it is not merely
a local change to the image being viewed.

Optional raw commands expose precise operations. These are individual entries,
not a block to paste and execute as a script:

```text
/set c:harbor_dock --steps 24
/set c:harbor_dock --seed 42
/set c:harbor_dock --seed random
/modify c:harbor_dock Add warm light to the windows; keep the dock layout
/variant c:harbor_dock Rainy evening, same layout --name harbor_rain --axis weather
/describe c:harbor_dock A wooden harbor dock with warm windows, hand-painted illustration
/revert c:harbor_dock
/resize c:harbor_dock 1024x576
```

`set`, `describe` and `revert` do not render. `modify`, visual `create`/`variant`,
`regenerate` and `resize` can render. Slash commands are parsed/validated and
execute without the AI operation-proposal review. With a command followed by
whitespace, the bottom entry enters its argument/completion mode. `c:` completion
offers registered targets; names with spaces require quoting.

**UX improvement:** keep raw syntax optional, surface the actual target and
whether an action will render, and show inherited adapter consequences clearly.
The edit-grounding system currently explicitly requests photographic wording;
that instruction is a poor fit for an illustration-first tool and should preserve
the project's chosen style instead of imposing photography.

## 12. Save, recover and move projects

Cmd+S saves the project and applies pending description drafts. Autosave saves
committed project/source changes and deliberately leaves inspector drafts
editable. Description drafts survive selection/screen changes, but do not assume
autosave alone has committed a typed description. Apply it or explicitly Save.

An unsent request is not a saved project field. Quit protects it and disables
Save and quit until you return and submit or clear it. A recognized action search
does not block clean quitting. Pending artwork work also blocks Save and quit.
Cancel keeps work intact; Discard and quit is deliberate abandonment, not save.

A project has a paired `.sf` source and `.db` database. Move both together.
Artwork revision paths are currently absolute, so copying only those two files
does not make a portable project package. There is no finished distributable
story export with all media bundled.

External edits to `.sf` are supported with conflict protection, but there is no
background file watcher. Save/open refresh a clean source; simultaneous external
and local edits are reported rather than overwritten. Save copy preserves a
local draft elsewhere; Reload uses disk text with confirmation for local changes.

When generation fails, inspect its revision/history and Details report. Reports
can retain operation, prompt, seed, size, steps and job context. Restore an old
completed revision to keep working, or correct the issue before regeneration.
Recent/rerun paths can execute work again; they are not resume-from-last-step
features. There is no universally safe one-click batch retry today.

## 13. Useful keyboard habits

The following use macOS notation. Cmd becomes Ctrl where the app defines a
platform primary modifier; single-letter Canvas shortcuts apply outside fields.

| Keys | Result / scope |
| --- | --- |
| Cmd+K | Global palette |
| Cmd+S | Explicit project save, including description drafts |
| Cmd+Z / Cmd+Shift+Z | Native text undo when editing a field; project undo/redo elsewhere |
| `g` then `s`; `g` then `c` | Story / Canvas navigation outside fields |
| `/` outside a field | Focus the bottom entry; use Cmd+K → Enter a command if needed |
| Arrows or `hjkl` in the gallery | Move the artifact cursor |
| Enter in the gallery | Open the keyboard-selected artifact; a focused button still owns native Enter |
| `n` in the gallery | New artifact form |
| `v`, `r`, `m` in visual detail | Variant, generation, instruction-edit form |
| `[` / `]` in detail | Previous/next family member |
| `e` / `f` in detail | History / Variants disclosure |
| Escape | Dismiss the active popup/dialog, or return from detail when the workspace owns the key |
| Up/Down, Enter in the action entry | Select/activate a suggestion |
| Cmd+Enter in the action entry | Explicit command/request submission |
| Ctrl+Space in Writer | Shared language completion |
| Right / Space in Preview | Advance / pause-resume |

Typing in a field takes precedence over letter shortcuts. Leaving the bottom
entry focused means `r` types a letter instead of regenerating. The advertised
`m` shortcut can open the edit form without an image even though the toolbar
hides Edit and typed `edit` explains the prerequisite; submission then rejects
the missing image. That inconsistency needs fixing, not memorization.

## 14. Repeat workflows with plans and the CLI

Plans is the app's existing guided/test workflow library. Guide runs visibly with
narration; Check runs assertions. Plans can create/open projects, submit commands,
inspect expected artifacts/images and exercise playback through app-level intents.
They are developer-authored YAML, not a finished user macro recorder. Some generate
images; inspect their steps and use an isolated project before running them.
Stopping a guide is not cancellation of an already-running image operation.

For repeatable operations, the CLI supports typed operations and JSON stacks:

```sh
svs --project /absolute/path/project.db stack run /absolute/path/stack.json --approve auto
```

Use interactive approval when the stack includes decisions you want to inspect.
CLI stacks execute progressively, but still stop on failure and preserve prior
applied changes. Close Studio or coordinate file ownership before mutating the
same database externally; there is no live registry synchronization experience.

For source checks, export its asset context and use the same language/compiler:

```sh
svs story catalog /absolute/path/project.db > /absolute/path/catalog.json
svs story check /absolute/path/project.sf --assets /absolute/path/catalog.json --require-ready
```

These are terminal commands, not bottom-entry slash commands. An external editor
can use the Storyfella LSP for diagnostics/completion/rename with an explicitly
exported catalog. The catalog is a snapshot requiring refresh. CLI `/compose`
terminology must not be mistaken for a supported `/compose` bottom command:
compose and draft exist as typed operations/CLI commands but are absent from
Studio's current slash parser.

**UX improvement:** saved recipes, parameter forms, target pickers, dependency
previews and a resumable task queue should make repeated work possible without
learning YAML, JSON or shell syntax.

## 15. The most important improvements

These are new findings/recommendations, not claims that the earlier UX remediation
made the product complete. “Code” means inspected implementation; “screen” means
visible in the supplied image. Priority describes the practical impact on users.

| Priority | Finding and evidence | Proposed change | Acceptance example |
| --- | --- | --- | --- |
| High | First generation is named Regenerate; no primary button in the empty preview. Screen/code. | Generate image empty-state action and consistent labels | Open an empty environment and generate without searching or interpreting icons |
| High | Palette and request entry mix navigation, actions and AI. Screen/code. | Separate global find/navigation from contextual request/action entry | Typing an instruction has a clear target and action, with predictable Enter |
| High | Proposed batches validate every step against the initial registry. Code. | Validate dependencies in order with stable created-result references | Create keeper, then its rainy variant in one reviewed request |
| High | A failed batch retains earlier work but has no resume dashboard. Code. | Per-step task results, resume and deliberate retry | Retry the failed third task without regenerating the first two |
| High | New story/scene/beat and draft operations do not write authored structure. Code. | Distinct project/library labels and reviewed Writer edits | Ask to insert a scene, approve a source diff and see it in Preview |
| High | No detailed image inspection or precise editing in Studio. Code. | Zoom/100%, comparison, and explicit Atelier handoff | Inspect a face at full size, mask-edit it, return to the same artifact |
| High | Edited appearance and canonical description diverge; Promote copies a prompt. Code. | Reviewed identity update after an edit | Keep new glasses through the next edit without manually rewriting identity |
| High | Imported artwork uses provenance as its canonical description. Code. | Separate provenance from a reviewed visual description at import | An imported illustrated character retains its visual traits in later edits |
| High | Planner omits operations supported by direct UI/slash commands. Code. | Shared capability catalog with truthful availability | Settings-only and canonical-description requests never accidentally render |
| High | Selected-picture planner/grounding is text-based and imposes photographic wording. Code. | Image-aware, style-preserving grounding where supported | Request a local illustration edit without invented traits or style change |
| High | Studio hosted image routing/library are separate from Atelier. Code/docs. | Shared provider execution and asset library, explicit UI integration | Use the chosen hosted image provider and reuse approved art across projects |
| Medium | Icon-only detail actions and large generic popup need interpretation. Screen/code. | Named tooltips, context ordering and compact shortcuts; no routine subtext | A new user discovers all actions while a power user needs few keystrokes |
| Medium | Keyboard `m`, toolbar and typed `edit` disagree about missing images. Code. | One prerequisite rule across every entry point | Every Edit path offers generation first when the image is absent |
| Medium | Text checkpoint copy conflicts with automatic Studio execution. Code. | Truthful review copy or actual generated-text checkpoint | The user knows exactly what approval does and which later decisions remain |
| Medium | No-image inspection/import does not offer replace-current-target semantics. Code. | Replace / Add variant / Add artifact handoff | Import final harbor art into the intended story reference without duplicate keys |
| Medium | Readiness warnings can block Preview despite zero errors. Code/docs. | Dedicated actionable readiness summary | “Needs 2 images” links directly to the missing art |
| Medium | Project pair + absolute media paths is not portable delivery. Code/docs. | Export package with checked relative resources | Open an exported story on a second machine without missing art |

The highest-value path is to complete one **select → generate → inspect → edit →
approve identity → use in story → preview** loop, then extend it to dependable
multi-step requests. Adding more actions before making that loop coherent would
increase the amount a power user must remember.

## Implementation references

- [Action catalog and matching](../apps/studio/src/ui/palette.rs),
  [entry behavior](../apps/studio/src/ui/cmdbar.rs),
  [shortcuts](../apps/studio/src/ui/shortcuts.rs).
- [Canvas, operation review, revisions and history](../apps/studio/src/ui/canvas.rs),
  [forms](../apps/studio/src/ui/art_actions.rs),
  [inspector](../apps/studio/src/ui/inspector.rs).
- [Execution and partial failure](../apps/studio/src/commands.rs),
  [typed operations](../crates/studio-core/src/registry/ops.rs),
  [slash vocabulary](../crates/studio-core/src/registry/slash.rs),
  [planner and grounding](../crates/studio-core/src/registry/backend.rs).
- [Source and identity](story-files.md), [language](language.md),
  [playback](playback.md), [Atelier/library/exchange](atelier.md).
- [Design standard](../DESIGN.md), [delivery status](ROADMAP.md#12-suggested-implementation-order).

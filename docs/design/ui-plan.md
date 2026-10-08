# UI/UX plan: getting the prototypes into the app

Historical prototype/implementation plan. Current interaction, visual and
accessibility requirements and open audit findings live in
[`DESIGN.md`](../../DESIGN.md); use [ROADMAP §12](../ROADMAP.md#12-suggested-implementation-order)
for current delivery status. The phase labels below describe this earlier plan.

Status: phases 0–4 complete; phase 5 blocked on the DSL. Supersedes nothing in [`../ROADMAP.md`](../ROADMAP.md); it
sequences the surface work that roadmap items 4 and 9 assume.

Reference prototypes (throwaway, not shipped):

- [`writer-prototype.html`](writer-prototype.html) — story editor, cursor-driven
  artifact panel, pending placeholders, completion.
- [`canvas-prototype.html`](canvas-prototype.html) — artifact grid, image-dominant
  detail with an inspector rail, tokenized command bar, command palette.

## 1. The problem being solved

Three separate complaints, one of which is structural:

1. **No subject.** A session produces images, not a *thing*. Nothing accumulates,
   so nothing feels productive. Only the DSL fixes this (`.sf` is the artifact
   that grows). Out of scope here, tracked as roadmap §9.
2. **A keyboard vocabulary behind a mouse-only shell.** The app has exactly one
   shortcut — `CmdOrCtrl+S` at [`ui/mod.rs:231`](../../apps/studio/src/ui/mod.rs) — and one
   `onkeydown` (Enter to submit). The CLI has a fast path; the GUI does not.
3. **A discovery cliff.** Nothing on screen reveals that `/resize`, `--steps`,
   or `describe` exist. The composer placeholder at
   [`ui/canvas.rs:1287`](../../apps/studio/src/ui/canvas.rs) is the entire vocabulary
   crammed into one string, and the narrated Guide plans exist partly to
   compensate for it.

This plan addresses 2 and 3, and prepares the seams for 1.

## 2. The dependency that shapes everything

Several of the best ideas in the canvas prototype are **derived from the story**
and cannot land before the DSL:

| Feature | Needs the DSL? |
| --- | --- |
| Artifact grid, variant stacks, in-flight tiles | No |
| Image-dominant detail + inspector rail | No |
| Tokenized command bar | No |
| Focus/mode model, command palette | No |
| **Pending placeholders** (artifacts the story names but that do not exist) | **Yes** |
| **Origin line** (`from harbor.handoff` under each tile) | **Yes** |
| **`Appears in` backlinks** in the inspector | **Yes** |
| Writer screen | **Yes** |
| Run mode | **Yes** (needs beats in order) |

So: **build the components now with those slots present but empty**, and let
them light up when the language arrives. Do not redesign later to accommodate
them. Concretely — the tile component takes an `origin: Option<StoryRef>`, the
grid takes a `pending: Vec<PendingRef>` that is empty until the compiler feeds
it, and the inspector has an `Appears in` section that renders nothing when the
list is empty.

## 3. Decisions the prototypes settled

- **Unresolved reference is *pending*, not an error.** Amber, with a Create
  affordance, counted in the status bar as "3 pending". This contradicts
  [`ROADMAP.md` §9](../ROADMAP.md) ("a compile error"); the roadmap should be
  updated. Same information, opposite emotional valence — the story becomes a
  work queue instead of a list of mistakes.
- **The canvas is not an editor.** An earlier pass modelled the artifact
  description as a text buffer with line numbers; it was rejected as forcing the
  writer's architecture onto a different job. Description is a plain field;
  generation parameters are real controls (segmented sizes, sliders, a seed
  lock), because a slider you can scrub while watching a cost estimate beats
  `steps: 8` as text.
- **Canon vs experiment is made visible.** `/modify` results are tagged
  `experiment`; the description is tagged `canon`; `/describe` is surfaced as a
  **Promote** action on a revision. This makes the existing "edits do not
  accumulate" behaviour legible instead of surprising.
- **One vocabulary, four surfaces**: CLI, command bar, palette, and later the
  DSL. Every inspector control must also be a command, and running the command
  moves the control.
- **Enter runs; Tab completes.** An exact-match completion popup must never
  swallow the run.

## 4. Sequence

Each phase is independently shippable and leaves the app working.

### Phase 0 — Spans and a completion engine *(done)*

The piece everything else waits on.

- Add a byte span to `Token` in [`registry/slash.rs:450`](../../crates/studio-core/src/registry/slash.rs)
  and make `tokenize` public. **One tokenizer** — the highlighter must not get
  its own, or the two will drift.
- Add `complete(input: &str, cursor: usize) -> Completions` next to the parser:
  operation names, artifact refs, flags valid for the current op, and flag
  values (`--size` → the size list, `--seed` → `locked`/`random`).
- Add `signature(op) -> Signature` for the parameter hint line.
- Unit tests for both, including the smart-punctuation cases already handled in
  `tokenize` (`—` → `--`, curly → straight).

This is a miniature language server over the op vocabulary — the same shape as
the LSP work in roadmap §10, at a tenth the scale. Good practice on purpose.

**Spike first (≈20 min):** does Dioxus 0.7 expose `selectionStart` on form
events? If not, the options are a `document::eval` round-trip or assuming
cursor-at-end for v1 (correct while typing, wrong when editing mid-line). This
shapes the `complete()` signature, so resolve it before writing the API.

### Phase 1 — The tokenized command bar *(done)*

Replaces the textarea at [`ui/canvas.rs:1287`](../../apps/studio/src/ui/canvas.rs) and
deletes its placeholder.

- Transparent `<input>` layered over a highlight `div` with identical font
  metrics. **Chips take their padding from `box-shadow` spread, never
  `padding`** — otherwise text width diverges and the caret desyncs.
- Token classes: op (invalid ops underlined), `c:` ref tinted by status
  (resolved / pending / unknown), flag, number, size, prose.
- Completion popup opening upward; signature help above the bar.
- Carry over `autocorrect: "off"` and `spellcheck: "false"` and test the
  WebKit `--` → em-dash rewrite early — a tokenized bar leans on literal `--`
  much harder than a plain textarea does.

### Phase 2 — Canvas home *(done)*

Replaces the kind-grouped sidebar with the grid.

- Tiles with variant stacks behind the thumbnail, in-flight tiles dimmed with a
  `rendering` badge (generation state belongs in the grid, not in a toast).
- `origin` and `pending` slots present but unfed (see §2).
- `hjkl` navigation, `Enter` to open.

### Phase 3 — Detail and the inspector rail *(done)*

- Image-dominant centre column: render, variant filmstrip, revision list,
  command bar.
- Inspector: description field (`canon`), canvas size segmented control, steps
  slider with cost estimate, seed value + lock, LoRA + weight, `Appears in`.
- Revisions tagged canon/experiment with **Promote** wired to `/describe`.

**Resolved:** seed lock did not exist and was added as a domain feature —
`Artifact::default_steps`/`default_seed` (schema v5), reachable through a new
`/set` operation, and consumed by the pipeline. Revisions also gained
`RevisionOrigin` (schema v6) so canon and experiment are distinguishable, which
is what Promote acts on.

### Phase 4 — Keyboard navigation and the palette *(done)*

- One key listener for the app (`ui/keys.rs`), **not** scattered `onkeydown`
  handlers. The filter for "is a text field focused?" lives in JavaScript
  because Dioxus events do not expose the focused element.
- `hjkl` moves the grid cursor, `Enter` opens, `Esc` returns, `n` creates,
  `/` focuses the command bar, `Ctrl+K` opens the palette.
- **No mode enum.** The canvas has two states and the selection already
  records which one it is in; a second source of truth could disagree.
- The palette is a view over the Phase 0 completion engine, scoped to context
  (artifact commands when one is open, create/open/navigate otherwise). It is
  nearly free once Phase 0 exists — which is why it comes last, not first.

### Phase 5 — Blocked on the DSL

Writer screen, story-fed placeholders and origins, `Appears in`, run mode. Not
started here; the slots from §2 are what they plug into.

## 5. Structure

[`ui/canvas.rs`](../../apps/studio/src/ui/canvas.rs) is 1426 lines and this is a rewrite of
it, not an edit. Make `apps/studio/src/ui/canvas/` a module directory — `home.rs`,
`detail.rs`, `inspector.rs`, `cmdbar.rs` — rather than growing the file.

Styling: Tailwind v4 scans `apps/studio/src/ui/**/*.rs`. Do not convert the prototype CSS to
utilities wholesale — the chip and layered-input rules are awkward as utilities.
Put those in a component layer in [`tailwind.css`](../../apps/studio/tailwind.css) and use
Tailwind for ordinary layout, matching current practice.

## 6. Acceptance

Every phase adds or updates a plan in `config/plans/`. Those are already
executable UX specifications: Guide mode narrates them, Check mode turns a
failure into a UX bug. A phase is done when its plan passes in Check mode.

- Phase 1 → extend `edit-vocabulary.yaml` to drive the bar by keyboard only.
- Phase 2/3 → new `canvas-navigation.yaml`: reach an artifact, change its size,
  promote an experiment, without the mouse.
- Phase 4 → new `keyboard-only.yaml`: a full session with no pointer events.

If a session cannot be expressed as a plan, that is the finding.

## 7. Doc changes owed

Done. `ROADMAP.md` §9 now calls an unresolved reference *pending* rather than
a compile error and records that the Writer surface landed ahead of the
language crate; §10 lists the applied schema versions; §12 item 4 describes
the grid-and-detail canvas. `AGENTS.md` describes the current `apps/studio/src/ui/`
layout. `architecture.md` records that the CLI, the command bar, the palette
and the inspector are four views of one vocabulary.

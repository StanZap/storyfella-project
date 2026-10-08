# Handoff: connected authoring and playable stories

Historical planning snapshot from baseline `3668233`. The four milestones below
have since been implemented; read `ROADMAP.md` §12, `playback.md`, and the current
`../AGENTS.md` for present behavior and remaining work. The baseline descriptions
and suggested opening prompt below record the original plan, not new instructions.

Prepared October 7, 2026. This is the handoff for the next implementation
session. The owner authorized work on these four milestones, in order:

1. Connect Writer and Canvas.
2. Finish authoring tools and extend the initial LSP.
3. Build the shared story execution runtime.
4. Add Studio preview and standalone Player.

Start implementation with milestone 1. Continue through the remaining milestones
in focused, reviewable increments; this document is not a claim they are done.
`ROADMAP.md` §12 remains the product status tracker. Update it and the relevant
contracts as work lands.

## Repository and starting point

- Work in `/Users/stanley/code/projects/storyfella/storyfella`.
  The previous session's environment cwd was the sibling `finewinetts` repo;
  change directory explicitly before running commands or editing files.
- Write implementation and documentation inside this Storyfella checkout.
  Scratch fixtures, probes and logs belong in `.svs-e2e/`.
- Keep the current `.cargo/config.toml`: build artifacts belong in checkout-local
  `target/`, and Cargo temporary files belong in `.cargo/tmp/`.
- The current branch at handoff preparation is `main`, with implementation
  baseline `3668233` (`Tighten walkthrough validation and clean compliance
  documentation`). Inspect `git status` and HEAD when resuming; preserve any
  subsequent user changes. This handoff is added after that baseline.
- The workspace needs the sibling `../clipforge/crates/inference` path dependency.
  Clipforge and FineWineTTS remain separate repositories.
- If sandbox permissions cover only a sibling directory, request the necessary
  tool escalation for this checkout; do not move the work into `/private/tmp`.

Read `../AGENTS.md`, `ROADMAP.md` §12, `language-plan.md`, `language.md`,
`compiler.md`, `story-files.md`, `data-model.md`, and `ux-walkthrough.md` before
changing contracts. The owner's explicit priorities above advance the Studio
integration work that the older roadmap labels deferred.

## What already works

| Area | Implemented baseline |
| --- | --- |
| Language | Shared lossless lexer, recovering parser, typed AST, source spans, Unicode/UTF-16 conversion, scene/beat symbols, asset and branch references, diagnostics, completion and navigation services. |
| Compiler | Deterministic version-1 JSON `StoryPlan`, source maps, resolved flow, immutable asset/revision bindings, and explicit readiness requirements. `svs story check` and `compile` work without models. Compilation produces a plan, not a native executable. |
| LSP | Stdio, full-document sync, versioned diagnostics, completion, definitions, references and scene/beat outline. It currently uses front-end analysis without a project asset catalog or compiler diagnostics. |
| Writer | Shared highlighting, scene/beat scope, resolved/pending references and asset affordances. Source changes currently mark the document dirty; they do not synchronize registry scenes/beats. |
| Persistence | Authoritative `.sf` beside project SQLite DB, exact invalid drafts, atomic source replacement, external-edit conflicts, Save copy/Reload, and schema-v9 draft export. Source is no longer duplicated in SQLite. |
| Canvas | Registry artifacts, scene/beat kinds, composition data, variants, revisions, typed operations, undo/redo, image generation, canon editing and settings. |
| Walkthrough | A narrated, configurable typing tour through the actual Writer and Canvas. Images/history are fixtures; rendering is explicitly simulated. |

The story runtime does not exist yet. `apps/player/src/main.rs` only prints a
placeholder. `shot`, `style`, and `mood` are retained but block readiness because
their execution is unsupported. Generated image paths still have an absolute-path
limitation. Do not advertise portable export, voice or expressive presentation
as part of these four milestones.

At baseline `3668233`, all 274 workspace tests passed, plus workspace/all-target
check, formatting, strict Studio Clippy, and diff checks. Those results are
historical evidence, not validation of the next changes. Earlier desktop
walkthrough validation passed all 24 steps; visible typing was also observed.

## 1. Connect Writer and Canvas

Implement reconciliation in the GUI-free domain layer, with a thin `AppState`
integration. Consume the shared analysis; do not parse `.sf` again in Studio.

- Make valid scene/beat declarations produce an ordered registry association.
  Compiler scene IDs are authored names; beat IDs are scene name plus beat name.
  Registry artifacts use UUIDs. Define their association explicitly and persist
  what is needed to keep UUIDs, composition and revisions through reopening.
- Define rename, move, removal and manual-artifact collision behavior before
  applying destructive updates. Preserve identity for dialogue edits and
  declaration reorder. A name change is not automatically identity-preserving
  under the current compiler contract; support an explicit rename operation or
  document its semantics rather than guessing from source proximity.
- Invalid or unfinished drafts must remain editable and saveable. Do not remove
  or recreate artwork based on a recovering partial parse. Define when a valid
  snapshot replaces the active association and how detached artwork remains
  reachable after a declaration is intentionally removed.
- Keep `.sf` authoritative for story structure. Identify which existing Canvas
  scene/beat actions must edit source and which are artwork-only; prevent a second
  independently editable story order in the registry.
- Use one asset catalog adapter for Studio analysis and compiler binding context.
  Preserve kind rules: character → Character, environment → Location,
  object → Prop. Missing assets are pending work; wrong kinds and ambiguity are
  errors. Inspect and extend existing pending-asset creation instead of replacing
  working affordances.
- Link the current beat to its referenced assets, allow creation of a missing
  asset of the expected kind, and refresh resolution after registry changes.
  Add source origins and `Appears in` backlinks from the shared reference data.
- Integrate source, registry and undo/redo deliberately. Current undo snapshots
  contain only the registry; a structural source edit must not leave the two
  representations inconsistent after undo, reload or project switching.

Acceptance: author a harbor scene with Mara and a lantern, create a missing
asset, revise dialogue, reorder beats, save and reopen. The order, associations,
UUIDs and artwork remain correct. Malformed drafts, duplicate names, external
edits, removals, undo/redo and failed saves do not erase source or revisions.
Use fixture images so this acceptance does not require inference services.

## 2. Finish authoring tools and extend LSP

- Wire shared completion into Writer at the actual caret offset, including
  replacement ranges and keyboard selection. The existing caret bridge reports
  a line only; add precise position handling with correct UTF-16/UTF-8 conversion.
  Preserve comments, indentation, CRLF and prose when applying edits.
- Add definition/reference navigation and positioned, clickable diagnostics.
  Integrate compiler flow errors and readiness reporting without treating missing
  artwork as a syntax error. Keep analysis/compiler diagnostic ordering and
  deduplication consistent across Writer, CLI and LSP.
- Supply external editors with a documented project asset context. Keep its
  adapter outside the language/compiler crates and do not couple LSP to the GUI
  or start inference. Define how open unsaved documents and catalog refreshes
  interact before publishing project diagnostics.
- Extend LSP with resolved-symbol rename, semantic tokens and conservative
  formatting. Share domain logic with Writer rather than implementing a second
  set of symbol rules in the transport. Rename must touch only resolved targets
  and reject collisions; asset rename requires an explicit host persistence
  contract, not a source-only rewrite that breaks the registry binding.
- Formatting must preserve prose, comments and meaning. Establish malformed-draft
  behavior and test it before advertising the capability. Add code actions only
  for concrete diagnostics with valid fixes.

Acceptance: Writer and an LSP client agree on completion, navigation and errors
for the same source/catalog. Unicode positions are correct; rename never changes
dialogue or unrelated names; formatting is idempotent and preserves prose and
comments. Protocol/process tests exercise advertised capabilities and failures.

Multi-file imports, project graphs, variables, conditions, expressions and
localization are later language milestones. Do not make this tooling increment
depend on inventing their grammar.

## 3. Build the shared story execution runtime

Add a real `crates/runtime` package (`storyfella-runtime`) when implementing it,
with workspace wiring and real callers/tests in the same increment. It consumes
the compiler plan and owns story state, execution and timing. It is distinct from
`crates/studio-core/src/runtime`, which manages creative inference.

- Establish a typed event/state contract for scene and beat entry, asset uses,
  dialogue, thoughts, narration, choice presentation and completion. Preserve
  instruction source spans for host navigation.
- Implement explicit advance, choice selection, jump, end, pause/resume,
  restart, and save/restore. Preserve current compiler semantics: source-ordered
  beats; implicit fallthrough to the next declared scene; final scene ends;
  choice/goto/end terminate the whole scene; cycles are allowed.
- Make timing deterministic and host-driven. Start with manual text advancement;
  use explicit time input where needed instead of embedding wall-clock sleeps in
  the engine. Full voice/media duration semantics remain later work.
- Bound automatic transitions through empty scenes or jump-only cycles so a
  valid cyclic story cannot hang a UI thread. Define a recoverable failure/event
  for an exhausted transition budget and test intentional cycles.
- Validate imported/deserialized plans at the execution boundary: supported
  version, identities, destinations, source ranges and instruction requirements.
  Do not assume arbitrary JSON is valid merely because it deserializes.
- Validate saved state against the exact plan identity and execution position.
  Reject malformed or mismatched state with actionable errors. Do not add
  migrations or aliases for hypothetical older save formats.
- Hosts resolve the plan's exact asset/revision binding and check resources.
  Never silently use the latest image when a pinned revision is missing. Separate
  source validity, compiler readiness and host resource availability.
- Keep unsupported directions as explicit blockers. The runtime must not skip
  `shot`/`style`/`mood` and claim it performed them.

Acceptance: deterministic headless traces cover dialogue/thought/narration,
empty scenes, fallthrough, both choice routes, invalid choices, cycles, pause,
restart, completion, restored state and rejected mismatches. Replaying the same
commands and explicit time inputs yields the same events.

## 4. Add Studio preview and standalone Player

- Studio compiles a snapshot of the current source with exact registry bindings,
  checks readiness/resources, and starts the shared runtime. Show text, choices,
  current scene/beat, basic resolved visuals and playback controls.
- Source-linked events let the user return to the executing line. Keep preview
  read-only with respect to authored source and artwork. Define source changes
  during preview explicitly; recommended first behavior is a fixed snapshot with
  a clear restart-to-apply-changes action.
- Replace the Player placeholder with a usable host for the same compiled plan,
  resource resolution and runtime controls. Define the input/open contract and
  document the launch command. A text-only console trace is useful validation,
  but it does not complete the visual Player milestone.
- Player's UI framework is currently undecided. Prefer reusing the existing
  desktop Dioxus dependency and small presentation components when practical;
  avoid introducing a second framework without a concrete reason. Share playback
  behavior through the runtime, not by making Player depend on Studio state.
- Support basic fixture-backed presentation for this slice. Do not implement
  packaging/export or imply the current absolute image paths are portable.

Acceptance: open a short, ready two-scene fixture in Studio preview and Player;
choose one route, restart and choose the other. Both hosts produce the same
events, source positions and pinned imagery. Pause/resume and restore work;
missing resources or unsupported directions are explained before playback.
Verify the actual desktop UI, including keyboard interactions and source return.

## Implementation entry points

| Concern | Existing files |
| --- | --- |
| Source analysis/services | `crates/language/src/{analysis,ast,source}.rs` |
| Compilation and plan contract | `crates/compiler/src/{lib,lower,plan}.rs` |
| LSP adapter/protocol | `crates/lsp/src/lib.rs` and `crates/lsp/tests/` |
| Source document and saves | `crates/studio-core/src/story.rs`, `crates/studio-core/src/persistence/mod.rs`, `docs/story-files.md` |
| Registry identity, hierarchy, revisions, operations | `crates/studio-core/src/registry/{mod,ops}.rs` |
| Studio state and project/operation seam | `apps/studio/src/state/mod.rs`, `apps/studio/src/commands.rs` |
| Writer scope, asset catalog, caret and highlighting | `apps/studio/src/ui/writer.rs` |
| Canvas and artifact details | `apps/studio/src/ui/{canvas,gallery,inspector}.rs` |
| Shell and shared presentation controls | `apps/studio/src/ui/{mod,components,icons,keys,shortcuts}.rs` |
| Player starting point | `apps/player/src/main.rs`, `apps/player/Cargo.toml` |
| Model-free UX validation | `apps/studio/src/driver/mod.rs`, `config/plans/ui-tour.yaml`, `docs/ux-walkthrough.md` |

Move genuinely shared host adapters out of Writer UI when their new callers need
them. Keep language/compiler/runtime independent of Studio and inference.

## Quality and validation for every increment

Follow `AGENTS.md`: clean code, no unused future APIs, no deprecated aliases,
duplicate parsers, compatibility shims, dead handlers or lint suppression to hide
unused code. Replace changed models and all consumers together. Actual SQLite
data migrations remain legitimate upgrade code and must preserve existing data.
Use `AppState` methods for state mutations and never hold signal guards across
`await`. Keep Rust workspace dependencies centralized; do not add Python or ML
frameworks.

Run from the Storyfella root after Rust changes:

```sh
cargo check --workspace --all-targets
cargo test --workspace
cargo fmt --all -- --check
cargo clippy -p <changed-package> --all-targets -- -D warnings
git diff --check
```

Run strict Clippy for each changed Rust package. Test meaningful semantics and
failure boundaries with isolated fixtures. Build runnable Studio binaries through
`cargo xtask build`/`run` or `dx` from `apps/studio`; plain Cargo-built Studio
binaries do not wire the stylesheet pipeline. Regenerate CSS with `npm run
css:build` from `apps/studio` when styling changes require it. Put live logs in
`.svs-e2e/logs/`.

The current walkthrough can be checked or viewed from the root:

```sh
cargo xtask plan config/plans/ui-tour.yaml --mode check --log
cargo xtask plan config/plans/ui-tour.yaml --mode guide --log
```

Extend real driver assertions for new behavior; do not treat simulated rendering
or a passing build as evidence of playable preview. Review the complete diff and
AGENTS compliance before committing each completed increment; stage only reviewed
scope, update live documentation and report actual validation. The owner has
authorized clean, validated implementation commits; no remote push is requested.

## Suggested opening prompt for the new session

> Work in `/Users/stanley/code/projects/storyfella/storyfella`. Read `AGENTS.md`
> and `docs/handoff-authoring-playback.md`. Implement the four authorized
> milestones in order: Writer/Canvas synchronization, authoring/LSP tooling,
> shared story runtime, then Studio preview and Player. Start with stable
> scene/beat reconciliation and its persistence/failure tests. Preserve drafts,
> artwork and identities, remove superseded code, validate each increment, and
> commit clean changes. Do not push. Continue the implementation, not just planning.

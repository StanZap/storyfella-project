# Storyfella language execution plan

Started October 5, 2026. This is the implementation sequence for roadmap items
9–10. The existing Writer's scene/beat syntax is the starting point. The old
prototype is design reference material; its parser and formats are not imported.

## 1. Shared front end and editor services — implemented first slice

- `storyfella-language`: source text, lossless lexical tokens, typed ordered AST,
  half-open byte spans, UTF-16 position conversion, recovering parser, scene/beat
  symbols, scene and asset references, diagnostics, navigation and completion.
- Two-space indentation; scenes, beats, references, directions, dialogue,
  thoughts, narration, choices/options, explicit scene jumps and endings.
- Host-supplied asset catalog with stable IDs, kind validation and collision
  checks. Missing assets are informational pending work; unknown branch targets
  and ambiguous names are errors.
- Writer highlighting, outline and in-scope references use the shared front end;
  source errors are visible without making missing assets into syntax failures.
- `storyfella-lsp`: stdio transport, initialization/shutdown, full-document sync,
  versioned diagnostics, completion, definitions, references and scene/beat outline.
- Fixture and regression tests for source preservation, Unicode/CRLF, malformed
  drafts, scope recovery, collisions, branch navigation and protocol lifecycle.

Acceptance: the bundled `examples/harbor.sf` and Studio starter/sample parse
without errors; tokens reconstruct source exactly; both clients consume one
analysis implementation. AST acceptance does not imply executable playback.
See [the initial language contract](language.md) for syntax and current limits.

## 2. Compiler and explicit executable representation — implemented first slice

`storyfella-compiler` emits version-1 serializable plans with stable scene/beat
identities, ordered instructions, resolved branch destinations and source maps.
The owner selected sequential scene fallthrough: a scene without an explicit
terminal continues to the next declaration; the final scene ends. Beats advance
in source order. Choice/goto/end terminate their scene; following statements
are compile errors. Unreachable scenes produce warnings; intentional cycles are
allowed. [compiler.md](compiler.md) records the contract and CLI examples.

Draft validity, asset readiness and direction readiness are separate. Immutable
host asset/revision bindings are snapshotted in each use. Compilation does not
generate artwork, mutate the registry or load inference models. Shot/style/mood
remain retained directions that block readiness until execution is implemented.

Acceptance implemented: deterministic lowering, source mapping, all targets
resolved, explicit flow origins, JSON round-trip and actionable readiness reports.
`svs story check`/`compile` expose the compiler; `--require-ready` gates binding and
direction readiness. Writer/LSP expose shared compiler diagnostics and catalog
readiness; `svs story catalog` exports project context for external editors.

## 3. Story files and Studio integration — file persistence implemented

Studio now opens/saves authoritative `.sf` files through the project commands.
Schema v9 exports actual `projects.story_source` drafts before dropping the
column. Document snapshots detect external-edit conflicts; copy/reload preserve
edits, save failures remain dirty, and project/source pairs can move together.
There was no `story_documents` table to migrate. See [story-files.md](story-files.md).

Scene/beat reconciliation now follows error-free shared analysis, with ordered
authored-name → UUID associations persisted by schema v10. Invalid drafts keep
the last valid association; removals detach artwork without removing revisions.
Text renames and inter-scene moves create new identities; dialogue/reorder retain
existing ones. Source and registry undo together. See [story-files.md](story-files.md).

Pending catalog creation of the expected kind, artifact origins and `Appears in`
backlinks, precise Writer completion/navigation and source-linked compiler
diagnostics are connected. Explicit rename retains artwork identity. Shared
rename, conservative formatting and semantic tokens are implemented with LSP
protocol/process tests. Remaining: project-relative generated asset paths.

Acceptance: disk source, Writer and canvas reconcile after save/reopen without
losing draft text or asset identities. Replace transitional behavior in the same
change; leave no parallel legacy parser or document store.

## 4. Shared story runtime and preview

Add `storyfella-runtime` with a deterministic state machine consuming the
compiled representation. The host supplies visuals/audio and receives playback
events; runtime owns scene/beat state, choices and timing. Implement explicit
advance, choice selection, jump, end, pause/resume and save/restore. Studio preview
and Player consume the same runtime; neither implements separate story semantics.

Acceptance: headless execution traces cover branching, invalid choices, restored
state and deterministic replay. Then validate the Studio preview flow and Player.

## 5. Expressive performance and media

Design speech annotations with the owner using real authored scenes. Keep speaker
identity, voice version, delivery scopes, timing, events and pronunciation typed
and source-mapped. Lower selected performance to FineWineTTS through an adapter;
do not embed its bracket/XML formats into Storyfella or assume parsed expression
is supported synthesis. Unsupported directions remain explicit readiness blockers.

Define visual variants, composition, transitions and media durations against the
runtime plan. Clipforge and FineWineTTS stay separate backend repositories.

Acceptance: a scene retains requested intent and exact bindings through compile,
execution and evidence; playback never quietly substitutes neutral performance.

## 6. Multi-file language and deeper tooling

Specify modules/imports, cross-file identity, cycles and project boundaries before
implementing a project graph. Single-document semantic tokens, rename and
formatting are implemented; workspace navigation and concrete code actions remain.
Variables,
conditions, expressions and localization need their own grammar/semantics and
fixtures; they are not implied by this first slice.

Acceptance: Studio and external editors agree on analysis; rename touches only
resolved references; formatting preserves prose and comments; cyclic imports and
invalid edits remain bounded and diagnosable.

## Validation

Run the focused language/LSP tests for each front-end change, then the repository's
required `cargo check --workspace --all-targets` and `cargo test --workspace`.
The inference services and model weights are not needed for language analysis.
GUI behavior needs a later interactive check; compilation alone does not verify
caret placement, highlighting layout or navigation affordances.

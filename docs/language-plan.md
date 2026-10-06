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

## 2. Compiler and explicit executable representation — next

Add `storyfella-compiler` only with its first implementation. Compile analyzed
stories into a versioned, serializable plan with stable scene/beat identities,
ordered instructions, resolved branch destinations and source maps. Establish
whether scene fallthrough is allowed, how beats advance, how choices terminate a
beat, and how `goto`/`end` interact with following instructions before lowering.
Reject unreachable or conflicting terminal instructions; do not guess semantics.

Separate draft validity, asset readiness and runtime capability readiness. Link
the host's immutable asset/revision bindings explicitly; compilation must not
generate artwork, mutate the registry or load inference models. Preserve shot,
style and mood as authored directions until execution semantics are specified.

Acceptance: deterministic lowering, exact source mapping, all targets resolved,
and actionable readiness reports with no silent defaults or dropped directions.

## 3. Story files become authoritative

Implement `.sf` open/save and project-relative asset linking through Studio's
existing project path/state methods. Migrate actual `story_documents` drafts to
files before dropping that SQLite table. Handle unsaved drafts, external edits,
save failures and project moves. Registry scenes/beats follow valid analysis;
partial edits must not delete artwork or recreate stable IDs.

Add Writer completion/navigation UI, pending assets of the correct kind,
source-linked diagnostics, artifact origins and `Appears in` backlinks.

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
implementing a project graph. Then add workspace navigation, semantic tokens,
rename, code actions and formatting with lossless comments/source maps. Variables,
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

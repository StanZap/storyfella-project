# Storyfella compiler: first execution-plan contract

`storyfella-compiler` validates and lowers stories without Studio, a project database,
GUI or inference backend. It emits a typed, serializable version-1 `StoryPlan`.
The [runtime](runtime.md) is the execution layer; compilation performs no playback.

## Flow semantics

The first declared scene is entry. Statements and beats execute in source order.
Beats are named organizational/event boundaries, not branches. A scene without
an explicit terminal continues to the next declared scene, as selected by the
owner. The last scene's implicit boundary ends the story. Empty scenes pass
through; an empty document/title-only draft cannot produce a plan.

`choice`, `goto` and `end` are terminal for the entire scene. Choice selects one
of its resolved scene targets; goto immediately targets a scene; end ends the
story. There is no resume-after-choice continuation. Statements after a terminal,
including another beat, are errors. Choice prompts/labels must be nonempty, and
option labels distinct. Intentional cycles are allowed. A graph walk warns about
scenes unreachable from entry; warnings do not prevent compilation.

The compiler records implicit flow as an instruction with `origin: SceneBoundary`
and a zero-width source span at the boundary. Explicit terminals have
`origin: Authored` and their original source span. Thus declaration order controls
fallthrough without silently discarding authored flow.

## Plan and linking

`compile(source, bindings)` returns `Compilation { plan, diagnostics }`;
`compile_analysis(analysis, bindings)` reuses shared analysis. Source or
binding-table errors yield no plan. Missing assets and unimplemented directions
yield a valid plan with blocked readiness; they never result in dropped instructions.

- Scene IDs are authored scene names. Beat IDs contain scene ID and beat name;
  text edits and declaration reordering do not replace those identities.
- Every instruction has a byte span, origin and optional beat scope. Names and
  text retain their own spans; branch options retain their label/target spans.
- The plan retains the exact original UTF-8 source, title, entry, ordered scenes
  and instructions. Decoded text is distinct from the original source snapshot.
- The host supplies `{id, name, kind, revision}` bindings with nonempty immutable
  IDs/revisions and unique IDs/names. Kind is `Character`, `Location` or `Prop`.
  Lookup is case-insensitive, as in shared language analysis. Wrong kinds and
  collisions are errors; unknown JSON binding fields are rejected.
- Each asset use snapshots its exact binding. An unbound use retains its authored
  name/kind and becomes a source-mapped pending requirement. Catalog order never
  selects an arbitrary duplicate.
- Shot/style/mood instructions are retained and listed as unsupported direction
  requirements. Their execution semantics have not been implemented. A complete
  asset catalog cannot make them ready.

`Compilation::ready()` checks source validity plus all represented linking and
direction requirements. It does not certify voice synthesis, load assets, check
files/hashes or start a runtime; hosts must validate resources when executing.
There are no format migrations, version aliases or older-plan import paths.

## CLI

Both commands print JSON to stdout and read only the requested source/binding
files. They do not load application configuration, open/create a project database,
start generation servers or modify source.

```sh
# Source/compiler validity; missing art remains informational pending work.
cargo run -p svs-cli --bin svs -- story check examples/harbor.sf

# A plan retains all pending assets and unsupported directions.
cargo run -p svs-cli --bin svs -- story compile examples/harbor.sf

# Require all represented binding/direction requirements to be satisfied.
cargo run -p svs-cli --bin svs -- story check story.sf --assets bindings.json --require-ready
```

The catalog file is a JSON array (revision is optional for pending artwork):

```json
[
  {"id":"character-mara","name":"mara","kind":"Character","revision":"immutable-revision-id"}
]
```

`check` emits validity, readiness, requirements and diagnostics with both byte
spans and zero-based UTF-16 positions. `compile` emits the plan when valid; on
source errors or failed `--require-ready`, it emits the check report and exits
unsuccessfully. Missing assets alone do not fail either command unless
`--require-ready` is selected. File/JSON errors fail before compilation.

Writer, CLI and LSP expose the same compiler diagnostics and readiness. Export
Studio metadata and exact completed active revisions with
`svs story catalog project.db`. This opens an existing database (including schema
upgrade if needed), without loading model configuration or starting inference.
Missing paths are rejected. See [editor context](language.md#external-editors).

## Validation and remaining work

Tests cover implicit and explicit flow, ordered beat scope, forward branches,
cycles/reachability, unreachable statements, Unicode source maps, duplicate option
labels, exact immutable bindings, blocked directions, determinism and JSON
round-trip. CLI process tests check JSON/exit behavior and prove that model config
and project databases are unnecessary.

Studio [`.sf` persistence](story-files.md), registry synchronization and authoring
tools, shared headless runtime and basic visual preview/Player are implemented.
Hosts validate exact image resources; see [playback.md](playback.md). Further
media semantics, timing, save-state compatibility, expressions,
imports and speech performance annotations require their own contracts.

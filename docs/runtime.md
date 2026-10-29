# Story execution

`storyfella-runtime` consumes a version-1 compiler `StoryPlan`, independently of
Studio, GUI and inference. `Runtime::start` validates before executing and
requires completed artwork. `Runtime::start_with_policy(...,
AssetPolicy::AllowMissing)` permits pending or unavailable artwork while
preserving the exact source and flow checks.

Both modes check the supported version, gather consistent pinned bindings,
recompile the retained source and require exact plan equality. This uses compiler rules for imported
identities, flow, ranges and requirements. Pending artwork blocks strict execution; unsupported shot/style/mood blocks both modes. Hosts additionally resolve exact resources in
strict mode; allow-missing hosts omit unavailable visuals.

Execution is manual and host-driven: no clocks, sleeps, audio or automatic text
duration. `Advance` passes the current text, `SelectChoice { index }` uses a
zero-based option, `Jump { scene }` targets a declared scene, and `End`, `Pause`,
`Resume`, `Restart` are explicit. Paused execution rejects advancement/choices/
jumps. Restart begins at entry, unpauses and clears command history. Invalid
commands leave state unchanged. Choices, jumps and ends terminate the scene;
implicit fallthrough and source-ordered beats follow the compiler.

Events report scene/beat entry, exact asset bindings, dialogue/thought/narration,
choice presentation, completion, pause/resume and exhausted transitions. Authored
events retain instruction source spans. Asset declarations replace the active
visuals of their kind; dialogue also introduces its speaker. Scene entry clears
visuals and beat scope. `current()` and `state()` expose read-only presentation.

Each start/flow command permits 128 automatic instruction transitions. Empty
scenes and jump-only cycles cannot hang a host. Exhaustion emits
`TransitionBudgetExceeded`, retaining a resumable position; Advance attempts the
next bounded batch, Jump can escape, and End/Restart remain available.

Save version 1 stores a SHA-256 identity of canonical full-plan JSON, a typed
command history and checkpoint. Restore validates the plan, version and exact
identity, replays the history, then requires checkpoint equality. Invented indices,
assets, phases or impossible commands fail with an actionable error. Restore
accepts at most 100,000 commands; execution requests Restart when history is full,
so every emitted save remains restorable. Restart begins a new history. There
are no format aliases/migrations. Resources are a separate host responsibility.

The real headless caller is:

```sh
svs story compile ready.sf --assets catalog.json --require-ready > plan.json
svs story trace plan.json commands.json
```

For example, `commands.json` can contain
`["Advance",{"SelectChoice":{"index":0}},"Pause","Resume","Advance"]`.
Trace emits a deterministic JSON event array and fails on invalid plans/commands.
It checks compiler readiness, without opening artwork files; it is not a visual
Player. Tests cover all text kinds, entry/beat/fallthrough, both choice routes,
invalid commands, cycles, pause/restart, completion, restore and corrupt imports.

## Studio preview and standalone Player

Studio's Preview tab and the standalone Dioxus desktop Player share
`storyfella-playback-ui` controls, the GUI-free `studio-core::playback` adapter,
and the execution runtime described above. Player has no dependency on Studio state.
No model configuration, generation or inference process is needed for playback.

Preview → **Apply draft** compiles the current unsaved Writer
source with exact completed active registry revisions. Syntax/flow errors and
unsupported shot/style/mood block starting. Studio Preview permits pending
artwork and shows black placeholders; standalone Player remains strict unless
started with `--allow-missing-assets`. Every plan binding is resolved by asset
UUID and exact revision UUID, with matching name/kind and a completed revision.
An absent revision, missing/unreadable file or undecodable image is explained
before strict visual execution. With `--allow-missing-assets`, those visuals
are omitted and the story continues with black placeholders; newer artwork is
never a fallback.
All used bindings, including artwork in alternate routes, are checked first.

The session retains its compiled source and decoded PNG visuals. Editing source,
changing artwork or replacing a file does not alter it. Source changes display
the quiet **Draft changed** indicator; **Restart story** replays the fixed snapshot,
while **Apply draft** checks and starts a new one. Switching projects
clears preview. Playback does not mutate authored source or registry artwork.

Both hosts use the same minimal widescreen stage: location artwork fills the
scene, with softly shaded dialogue/thought/narration near the bottom, a small
speaker portrait, and text choices. Without location artwork, the available
portrait or prop is contained instead of cropped to fill the stage. A quiet
header and current scene/beat leave the artwork prominent. Long passages and
short windows scroll; narrower windows reduce margins.

Continue and pause/resume stay beside the story. **Playback options** (the
ellipsis) holds open, restart, end, scene jump, bookmarks, state files, artwork
and source inspection. **Artwork** shows all active pinned images with name/kind
captions. One panel opens at a time; Escape or clicking outside dismisses it and
returns focus to the options button. Opening a panel focuses its Close button.
Right arrow advances; Space pauses/resumes. Tab and Enter operate native controls
and choices. Transition-budget exhaustion shows the recoverable runtime message.

**Source snapshot** displays read-only source with the executing line highlighted.
Studio's **Edit source** returns to the precise Writer caret. It is
available when current source matches the snapshot; after editing, apply the
new draft or inspect the retained snapshot. Source spans are UTF-8 bytes with
shared UTF-16 conversion for the editor.

**Bookmark / Restore bookmark** keep a checkpoint in memory. **Save state… /
Load state…** write/read runtime save-version-1 JSON, verified against this exact
plan and its replayed history. Failed load leaves the ongoing session intact.
Restore retains checked snapshot resources and clears the last host event batch.
Hosts retain only the latest bounded batch, so cycles do not grow a resident
event log. The CLI trace explicitly collects its requested event sequence.

### Player input and launch

Open a paired `.sf`/`.db`, either from **Open story…** or at launch:

```sh
cargo run -p storyfella-player -- /absolute/project.sf
# Equivalent paired database input:
cargo run -p storyfella-player -- /absolute/project.db
# A compiled plan uses its pinned revisions from the supplied project:
cargo run -p storyfella-player -- /absolute/plan.json --project /absolute/project.db
```

The pair has the same basename (`project.sf` and `project.db`). Missing databases
are rejected before opening, so Player never creates a project by mistake.
Existing databases use the normal persisted schema upgrade path. Text-only JSON
plans need no resource DB. For a visual JSON plan opened in the UI, Player asks
for its resource project after validating the plan. Cancel leaves the current
session alone. Invalid plans and unavailable resources display a blocking error.

Player uses embedded component styles, so Cargo launch is styled without an
asset pipeline. A dx desktop artifact can also be built from `apps/player` with
`dx build --platform desktop`. Studio continues to require its dx/CSS pipeline.

### Model-free verification

```sh
cargo run -p storyfella-studio-core --example playback_fixture
cargo xtask plan config/plans/playback.yaml --mode check --log
cargo xtask plan config/plans/playback.yaml --mode guide --log
cargo run -p storyfella-player -- .svs-e2e/inputs/playback/lantern.sf
```

The fixture generator writes three procedural PNGs, a two-scene `.sf`/`.db` pair
and compiled JSON beneath `.svs-e2e/inputs/playback`. It deliberately overwrites
that disposable fixture with fresh identities. The 22-step desktop plan checks
text, choices, both routes, pause/resume, bookmark restore, restart/completion and
pinned decoded imagery through the same model/actions as the visible controls.
Artwork creation is shared with the resource tests through the GUI-free
`studio-core::playback::fixture::create`. It reports unusable output directories
as errors instead of panicking or replacing a blocking file.

Host adapter tests compare event sequences, source positions and pinned image
bytes for source compilation versus imported-plan playback, including a newer
active revision and missing/corrupt files. Runtime tests cover rejected save
mismatches and malformed execution positions.

This slice presents basic still artwork. Voice, media durations, transitions,
composition performance, portable image paths and packaging/export remain later
work. The absolute paths in current project revisions are not portable exports.

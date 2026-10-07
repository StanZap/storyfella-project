# Studio preview and standalone Player

Studio's Preview tab and the standalone Dioxus desktop Player share
`storyfella-playback-ui` controls, the GUI-free `studio-core::playback` adapter,
and [the execution runtime](runtime.md). Player has no dependency on Studio state.
No model configuration, generation or inference process is needed for playback.

Preview → **Start / apply current draft** compiles the current unsaved Writer
source with exact completed active registry revisions. Syntax/flow errors,
pending artwork and unsupported shot/style/mood block starting. Every plan
binding is resolved by asset UUID and exact revision UUID, with matching name/
kind and a completed revision. An absent revision, missing/unreadable file or
undecodable image is explained before execution. Newer artwork is never a fallback.
All used bindings, including artwork in alternate routes, are checked first.

The session retains its compiled source and decoded PNG visuals. Editing source,
changing artwork or replacing a file does not alter it. Source changes display
an apply-current-draft banner; **Restart** replays the fixed snapshot, while
**Start / apply current draft** checks and starts a new one. Switching projects
clears preview. Playback does not mutate authored source or registry artwork.

Both hosts show current scene/beat, dialogue/thought/narration, basic asset images,
choices and completion. Asset captions show pinned artwork; the tooltip identifies
the revision. Controls provide advance, pause/resume, restart, end and scene jump.
Right arrow advances; Space pauses/resumes. Tab and Enter operate native controls
and choices. Transition-budget exhaustion shows the recoverable runtime message.

**Source snapshot** displays read-only source with the executing line highlighted.
Studio's **Edit executing line** returns to the precise Writer caret. It is
available when current source matches the snapshot; after editing, apply the
new draft or inspect the retained snapshot. Source spans are UTF-8 bytes with
shared UTF-16 conversion for the editor.

**Bookmark / Restore bookmark** keep a checkpoint in memory. **Save state… /
Load state…** write/read runtime save-version-1 JSON, verified against this exact
plan and its replayed history. Failed load leaves the ongoing session intact.
Restore retains checked snapshot resources and clears the last host event batch.
Hosts retain only the latest bounded batch, so cycles do not grow a resident
event log. The CLI trace explicitly collects its requested event sequence.

## Player input and launch

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

## Model-free verification

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
Host adapter tests compare event sequences, source positions and pinned image
bytes for source compilation versus imported-plan playback, including a newer
active revision and missing/corrupt files. Runtime tests cover rejected save
mismatches and malformed execution positions.

This slice presents basic still artwork. Voice, media durations, transitions,
composition performance, portable image paths and packaging/export remain later
work. The absolute paths in current project revisions are not portable exports.

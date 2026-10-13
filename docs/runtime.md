# Story execution

`storyfella-runtime` consumes a version-1 compiler `StoryPlan`, independently of
Studio, GUI and inference. `Runtime::start` validates before executing and
requires completed artwork. `Runtime::start_with_policy(...,
AssetPolicy::AllowMissing)` permits pending or unavailable artwork while
preserving the exact source and flow checks.

Both modes check
the supported version, gathers consistent pinned bindings, recompiles the retained
source and requires exact plan equality. This uses compiler rules for imported
identities, flow, ranges and requirements. Pending artwork and unsupported
shot/style/mood block execution. Hosts additionally resolve exact resources in
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

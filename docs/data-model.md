# Data model and persistence

The application state is a single `AppState` (`src/state/mod.rs`) holding a
`Project`, an unsaved-changes flag, and the currently selected frame. The
domain types live in `src/models/mod.rs` and `src/timeline/mod.rs`; persistence
lives in `src/models/persistence.rs`.

## Types

```text
Project
├── id: Uuid
├── name: String
├── timeline: Timeline
│   └── clips: Vec<Clip>
│       └── Clip { id: Uuid, label: String,
│                  start_seconds: f64, duration_seconds: f64 }
└── storyboard: Vec<StoryboardFrame>
    └── StoryboardFrame
        ├── id: Uuid
        ├── prompt: String
        ├── asset_path: Option<String>
        ├── revisions: Vec<ImageRevision>
        │   └── ImageRevision { id, prompt, asset_path,
        │                       status: RevisionStatus, error }
        └── active_revision_id: Option<Uuid>
```

`RevisionStatus` serializes as `snake_case`:

| Variant | Wire value |
| --- | --- |
| `Queued` | `queued` |
| `Generating` | `generating` |
| `Completed` | `completed` |
| `Failed` | `failed` |
| `Cancelled` | `cancelled` |

## Invariants maintained by `AppState`

The state API is the only place the model mutates, which keeps these rules
centralized:

- **Blank beats are ignored.** `add_storyboard_beat` trims the prompt and
  returns without touching state when empty.
- **Beat ↔ clip are created together.** Each new beat appends a five-second
  clip (`duration_seconds = 5.0`) that starts where the previous clip ended,
  so the sequence is contiguous. Beats are labeled `Beat 1`, `Beat 2`, … by
  their 1-based index.
- **A new beat becomes the selection.** `selected_frame_id` points at the new
  frame, and `begin_new_beat` clears the selection so the composer starts a
  fresh scene.
- **Selection is validated.** `select_frame` ignores ids that do not exist.
- **Revisions own the active asset.** `start_revision` appends a `Queued`
  revision and marks it active. `update_revision` promotes the frame's
  `asset_path` when a revision completes. `activate_revision` restores a
  completed revision by rewiring both `active_revision_id` and `asset_path`;
  revisions without an asset cannot be activated.
- **Every mutation sets `has_unsaved_changes`** — except the no-op paths
  (blank beat, invalid selection), which must not dirty the project.

The sequence is the core invariant: the storyboard is the creative source of
truth, and the timeline is derived from it. There is currently no delete,
insert, or reorder operation, so the two lists can only be out of sync if the
state API is bypassed.

## Persistence format

### SQLite (the project format)

`ProjectDb` (`src/persistence/mod.rs`) is SQLite storage for the project:
one `projects` row, then `artifacts`, `revisions`, `masks`, `layers`,
`lora_registry` (schema-only until the LoRA feature lands), and
`operation_log`. The `Project` model (storyboard + timeline) is carried as
`project_json` on the `projects` row (schema v2) until the canvas replaces
it. The registry stays the in-memory source of truth; a save replaces the
snapshot rows in one transaction. Schema versioning lives in `schema_meta`;
migrations are appended scripts applied on open — the way the current
format evolves (no legacy formats are parsed; the TOML `ProjectStore` was
removed with the pre-SQLite formats). The `svs` CLI persists to
`.svs-project.db`.

### The GUI save flow

`AppState.project_path` names the open database file. The Projects screen
scans `paths.project_dir` (config) for `.svs-project.db` files, creates new
projects as `slug.svs-project.db`, and opens any `.db` file in place via
the Open… picker. The workspace autosaves per the General settings select
— after every change, every minute, or off — and `Cmd/Ctrl+S` saves
manually; saving clears `has_unsaved_changes`.

## Known gaps

These are documented deliberately because persistence is the current boundary
between "in-memory prototype" and "real product":

1. ~~**Not wired into the UI.**~~ The Projects screen opens/creates
   `.svs-project.db` files and the workspace autosaves or saves manually
   (Cmd/Ctrl+S); saving clears the dirty flag.
2. ~~**`has_unsaved_changes` is never cleared.**~~ Cleared on every successful
   save (autosave, manual, or open).
3. **Absolute asset paths.** `CreativeRuntime::import_asset` stores the
   canonical absolute path of each generated file. A project moved between
   machines (or even directories) loses its image references. A stable,
   relative-to-project asset reference is needed before shipping.
4. ~~**No schema version.**~~ SQLite carries versioned migrations
   (`schema_meta`, `src/persistence/`); the current format evolves in
   place via migrations.
5. **No generated-asset index.** `src/assets/mod.rs` (`AssetCatalog`) is a stub;
   there is no central registry of imported/generated files, thumbnails, or
   usage counts.

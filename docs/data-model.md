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

`ProjectStore` (`src/models/persistence.rs`) serializes `Project` to pretty
TOML and reads it back with strict parsing errors. Optional fields are omitted
when empty. An illustrative project file:

```toml
id = "550e8400-e29b-41d4-a716-446655440000"
name = "The Lighthouse"

[[timeline.clips]]
id = "17e29460-a0f4-47ab-a1d6-c22c60e2f078"
label = "Beat 1"
start_seconds = 0.0
duration_seconds = 5.0

[[storyboard]]
id = "0e5d0c53-4ce8-42a4-a11f-1d0e5f5ac001"
prompt = "A quiet lighthouse above a silver sea at dusk"
asset_path = "/Users/me/Library/Application Support/Smart Visual Sequencer/assets/generated/9f2c…png"
active_revision_id = "c11…"

[[storyboard.revisions]]
id = "c11…"
prompt = "A quiet lighthouse above a silver sea at dusk"
asset_path = "/Users/me/…/generated/9f2c…png"
status = "completed"

[[storyboard.revisions]]
id = "d4e…"
prompt = "make the light warmer"
status = "queued"
```

`PersistenceError` distinguishes read, write, parse, and serialize failures
with the offending path attached, so the UI can present actionable messages.

## Known gaps

These are documented deliberately because persistence is the current boundary
between "in-memory prototype" and "real product":

1. **Not wired into the UI.** `ProjectStore` is implemented but nothing calls
   it. The Projects screen is a mock list and the Autosave setting is not
   functional. Saving/loading a project is the natural next slice.
2. **`has_unsaved_changes` is never cleared.** There is no save flow, so the
   dirty indicator in the header stays on for the whole session.
3. **Absolute asset paths.** `CreativeRuntime::import_asset` stores the
   canonical absolute path of each generated file. A project moved between
   machines (or even directories) loses its image references. A stable,
   relative-to-project asset reference is needed before shipping.
4. **No schema version.** `models/mod.rs` carries an explicit TODO for versioned
   schemas and migration support. Any format change before that lands will
   silently break older files.
5. **No generated-asset index.** `src/assets/mod.rs` (`AssetCatalog`) is a stub;
   there is no central registry of imported/generated files, thumbnails, or
   usage counts.

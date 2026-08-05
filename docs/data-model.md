# Data model and persistence

The application state is a single `AppState` (`src/state/mod.rs`) holding the
artifact registry, an unsaved-changes flag, and the currently selected
artifact. The domain types live in `src/registry/`; persistence lives in
`src/persistence/mod.rs`. The legacy `Project`/`StoryboardFrame`/`Timeline`
model was removed when the canvas replaced it (roadmap §12 item 4).

## Types

The registry (`src/registry/mod.rs`) is one id space for every artifact:

```text
ArtifactRegistry
├── artifacts: Vec<Artifact>
│   └── Artifact
│       ├── id: Uuid                     — one id space, c:<ref> resolution
│       ├── kind: ArtifactKind           — story | scene | beat | character |
│       │                                  environment | object
│       ├── name: String                 — the memorable c: key (unique per
│       │                                  project; empty → short id refs)
│       ├── description: String          — defining text / default prompt
│       ├── default_size: Option<(u32, u32)> — explicit `/create … --size`;
│       │                                  None = the kind default applies
│       ├── variant_of: Option<Uuid>     — variant artifacts point at a base
│       │                                  (variants inherit the base's size)
│       ├── variant_axis: Option<VariantAxis> — outfit|age|body|hair|expression|
│       │                                  time-of-day|weather|season|mood
│       ├── parent_id: Option<Uuid>      — scene→story, beat→scene,
│       │                                  room (environment)→environment
│       ├── composition: Option<BeatComposition> — beat-only: mode + layers
│       ├── active_revision_id: Option<Uuid>
│       ├── revisions: Vec<ArtifactRevision>    — generation history
│       │   └── ArtifactRevision { id, prompt, asset_path, status:
│       │       RevisionStatus, seed, model, error, masks: Vec<StoredMask> }
│       └── drafts: Vec<StoryDraft>      — pre-.sf story text proposals
└── log: Vec<OperationLogEntry>          — provenance, the undo basis
```

`RevisionStatus` serializes as `snake_case`: `queued | generating | completed |
failed | cancelled`.

## Invariants maintained by the registry and `AppState`

- **Kind/parent compatibility is enforced at creation** (`create_artifact`):
  stories/characters/objects take no parent; scenes attach to stories; beats
  to scenes; environments may parent environments (rooms). Variants require a
  base of the same visual kind and never a variant of a variant.
- **Names stay unique for system-created keys** (`ops::unique_name`):
  `c:<ref>` resolution is a case-insensitive exact match; duplicates are
  rejected as ambiguous, so auto-derived names are suffixed (`-2`, `-3`, …).
- **Sizes are per artifact, defaulted per kind.** `create_artifact` accepts
  an optional `default_size` (validated: multiples of 32 within 256..=2048,
  the pipeline contract bounds); `None` means the kind default applies
  (character 512×768, object 768×768, environment/scene/beat 1024×576,
  story text-only). Variants inherit the base's size. `regenerate`/`modify`
  compile their pipeline with the resolved size; per-run overrides (CLI
  `--size`) still win.
- **Revisions own the active image.** `start_revision` appends a `queued`
  revision and marks it active; `finish_revision` promotes it (completed +
  asset). `latest_image` is the last completed revision with an asset — a
  failed revision never becomes the active image. `AppState::activate_revision`
  restores an older revision (only revisions with an asset), and
  `AppState::display_image` shows the active revision's asset or falls back to
  the latest completed one.
- **Selection is validated.** `select_artifact` ignores ids that do not exist;
  undo/redo prune a selection whose artifact the restored snapshot lacks.
- **Undo is state restore, never pipeline re-execution.**
  `snapshot_for_undo` saves the full registry before an operation applies
  (capped at 100 snapshots); `undo`/`redo` swap snapshots and dirty the
  project. A new operation clears the redo stack (history forks).
- **Every mutation sets `has_unsaved_changes`** — except the no-op paths
  (invalid selection), which must not dirty the project.

## Persistence format

`ProjectDb` (`src/persistence/mod.rs`) is SQLite storage for the project: one
`projects` row, then `artifacts`, `revisions`, `masks`, `layers`,
`lora_registry` (schema-only until the LoRA feature lands), and
`operation_log`. The registry stays the in-memory source of truth; a save
replaces the snapshot rows in one transaction. Schema versioning lives in
`schema_meta`; migrations are appended scripts applied on open — the way the
current format evolves (no legacy formats are parsed; the TOML `ProjectStore`
and the `project_json` storyboard column (schema v3) were removed with the
models they carried). The `svs` CLI persists to `.svs-project.db`.

### The GUI save flow

`AppState.project_path` names the open database file. The Projects screen
scans `paths.project_dir` (config) for `.svs-project.db` files, creates new
projects as `slug.svs-project.db`, and opens any `.db` file in place via the
Open… picker. The workspace autosaves per the General settings select — after
every change, every minute, or off — and `Cmd/Ctrl+S` saves manually; saving
clears `has_unsaved_changes`.

## Known gaps

1. **Absolute asset paths.** `CreativeRuntime::import_asset` stores the
   canonical absolute path of each generated file. A project moved between
   machines (or even directories) loses its image references. A stable,
   relative-to-project asset reference is needed before shipping.
2. **No generated-asset index.** `src/assets/mod.rs` (`AssetCatalog`) is a
   stub; there is no central registry of imported/generated files,
   thumbnails, or usage counts.
3. **No delete/rename ops.** The operation vocabulary has no `delete`
   (artifacts can only be undone) and no description-edit op; both are
   roadmap work.
4. **No per-run size control in the GUI.** `--size`/`--steps` overrides
   exist on the CLI (`svs op regenerate`); the canvas uses the artifact's
   default size for every run.

# Data model and persistence

The application state is a single `AppState` (`apps/studio/src/state/mod.rs`) holding the
artifact registry, an unsaved-changes flag, and the currently selected
artifact. The domain types live in `crates/studio-core/src/registry/`; persistence lives in
`crates/studio-core/src/persistence/mod.rs`.

## Types

The registry (`crates/studio-core/src/registry/mod.rs`) is one id space for every artifact:

```text
ArtifactRegistry
├── story_scenes: Vec<SceneAssociation> — source order, authored keys → UUIDs
├── artifacts: Vec<Artifact>
│   └── Artifact
│       ├── id: Uuid                     — one id space, c:<ref> resolution
│       ├── kind: ArtifactKind           — story | scene | beat | character |
│       │                                  environment | object | illustration
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
│       │       RevisionStatus, seed, model, error, origin: RevisionOrigin }
│       └── drafts: Vec<StoryDraft>      — artifact-scoped generated text proposals
└── log: Vec<OperationLogEntry>          — provenance, the undo basis
```

`RevisionStatus` serializes as `snake_case`: `queued | generating | completed |
failed | cancelled`. `RevisionOrigin` is `canon | experiment` — canon is a
promoted revision, experiment an edit that has not been made canonical.

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
  story text-only). Variants inherit the base's size. Visual `create`/
  `variant` compile their first-image pipeline with the resolved size (the
  variant also references the base's active image); `regenerate`/`modify`
  use the same rule, and per-run overrides (CLI `--size`) still win.
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
  `snapshot_for_undo` saves the registry and exact source text before an operation applies
  (capped at 100 snapshots); `undo`/`redo` swap snapshots and dirty the
  project. A new operation clears the redo stack (history forks).
- **Every mutation sets `has_unsaved_changes`** — except the no-op paths
  (invalid selection), which must not dirty the project.

## Persistence format

### Portable project folders

New Studio projects use the [shared project folder](shared-project.md):
`storyfella.toml`, `assets/story.sf`, `story.db`, immutable `artwork/` revisions and
`atelier/` working documents. The existing registry/schema stays authoritative;
shared databases store image paths relative to their root and resolve them on
load. Existing standalone source/database pairs remain readable and can be
explicitly copied into a project folder with identities and artwork preserved.
The standalone pair behavior described below still applies to CLI and standalone
projects. Shared manifests require layout version 2.

`ProjectDb` (`crates/studio-core/src/persistence/mod.rs`) is SQLite storage for the project: one
`projects` row, then `artifacts`, `revisions`, `layers`,
`lora_registry` (project LoRA entries — round-tripped since the
LoRA registry landed), and `operation_log`. The registry stays the in-memory source of truth; a save
replaces the snapshot rows in one transaction. Schema versioning lives in
`schema_meta`; migrations are appended scripts applied on open — the way the
current format evolves (no legacy formats are parsed; the TOML `ProjectStore`
and the `project_json` storyboard column (schema v3) were removed with the
models they carried, and the `masks` table (schema v8) was dropped with the
mask edit path). Schema v9 exports `projects.story_source` to `.sf` before
dropping the column. Schema v10 adds ordered scene/beat name-to-UUID associations
in `projects.story_scenes_json`; `.sf` is the authoritative source.
Schema v11 adds the independent illustration kind and a registry version for
checked GUI writes. The artifact-table rebuild retains existing graph rows and
checks foreign keys before commit. A loaded `StoredProject` carries that version;
GUI writers commit against it and preserve newer work on a stale-write error.
Schema v12 adds transient `source_write` intent: Studio commits its catalog and
exact before/after source together, then publishes `.sf` and clears the intent.
Load/retry recovers an interruption only against a matching file; unrelated
source text requires explicit preserve-copy recovery. Schema v13 adds a
project-relative filename to that intent for an MCP edit of one included
source file. An intent with no filename targets the entry, including intents
carried forward from v12. GUI catalog refresh uses
UUID/field three-way merging, retaining drafts and rebasing undo. See the
[concurrency contract](shared-project.md#synchronization-and-concurrent-editing).
Catalog entries reuse artifact UUIDs; source requirements, saved image revisions
and Atelier's asset/artboard links remain separate. See the
[shared catalog contract](shared-project.md#shared-asset-catalog).

### The GUI save flow

`AppState.project_path` names the open database file. The Projects screen
scans `paths.project_dir` (config) for project folders and `.db` files, creates new
projects as `slug/`, and opens any `.db` file in place via the
Open… picker. The picker also opens `.sf` with a companion `.db` beside it.
`AppState.story` holds the authoritative source document, bound file path and
last disk snapshot. Source-file migration and external-edit conflict handling are described below. The workspace autosaves per the General settings select — after
every change, every minute, or off — and `Cmd/Ctrl+S` saves manually; saving
clears `has_unsaved_changes`.

## Story source files in Studio

The Writer reads and writes UTF-8 `.sf` files. SQLite stores project metadata,
artwork, revisions, operation history and artifact-scoped generated text proposals.
It no longer stores the authored Writer source.

### Project files

Standalone projects are a pair in the same directory:

```text
my-story.svs-project.db     artwork and project metadata
my-story.svs-project.sf     authored story
```

The source path is the database path with its extension replaced by `.sf`.
Projects → Open accepts either file. Opening a standalone `harbor.sf` creates
`harbor.db` beside it for artwork, preserving the source exactly. New projects
choose a filename unused by either member of the pair.

Move both files together to preserve source and artifact identities. Generated
image paths still have the existing absolute-path limitation; this change does
not relocate images or implement portable asset packaging.

### Editing and saving

Writer edits retain comments, CRLF/LF, Unicode and unfinished or invalid syntax.
Parsing diagnoses a draft; it does not rewrite source, reject saving or delete
artwork. An unbound Writer draft is retained when creating its first project.

Autosave and Cmd/Ctrl+S use the same project command. Source saves stage a complete
file beside the original, flush it, and replace it with a rename. A save first
compares disk text with the document's last saved snapshot. If both disk and
Writer have changed, it reports `SVS-STO-110`, keeps the local draft, and leaves
the external file untouched. A clean Writer refreshes external edits when saving.
Opening a project also loads current disk text. Background polling refreshes
clean source and shared catalog state approximately every two seconds; a dirty
Writer buffer retains its disk conflict guard.

Writer → Save copy saves local text to a new file without changing the open
project or overwriting an existing file. Writer → Reload .sf reads disk text,
asking for confirmation before discarding local edits. Failed reads leave the
editor intact. A missing source in an upgraded project is an error rather than
an empty draft that could erase the story.

Studio validates source before committing the catalog/source intent. A failed
catalog commit publishes no source. A failed source publish leaves a durable
intent and the editor's work intact; load/retry completes publication or exposes
preserve-copy recovery. Switching projects first saves pending changes; a
failure leaves the current project open.

### Migration from SQLite drafts

Schema v9 exports the actual v8 `projects.story_source` column before dropping
it. An existing identical `.sf` file permits retrying a partially completed
migration. An empty stored draft permits an existing source file to remain
authoritative. Different nonempty copies block migration: the original column
and schema version remain intact, and the `.sf` file is not replaced. Reconcile
the two copies before retrying. Export failures also preserve the database draft.

There is no parallel SQLite source writer, old-format parser or compatibility
alias. The export is an active schema upgrade for existing projects.

### Validation and next work

Filesystem tests cover exact text, malformed drafts, external edits, copy/reload,
missing files, migration/retry and moved project pairs with stable artifact IDs.
Studio state tests verify joint source/registry undo and draft preservation.
Studio's command tests exercise the actual project commands,
including conflict-blocked navigation and retrying a partial project save.

### Scene and beat reconciliation

Every error-free shared analysis replaces the active ordered association.
Scene keys are authored names; beat keys are `(scene, beat name)`. Schema v10
stores those keys and their registry UUIDs in `projects.story_scenes_json`.
It stores no second source or editable story order. Dialogue edits and reorder
retain UUIDs, compositions, descriptions and revisions. Invalid/unfinished drafts
and duplicate declarations retain the last valid association and remain saveable.
A valid empty source detaches every declaration.

Text renames and moves between scenes change authored identity: they detach the
old artwork and create a new association. Reintroducing a removed name also
creates new artwork. No proximity matching or automatic adoption of manual
artifacts occurs. Reloading saved source after an unsaved rename/removal reattaches
its saved UUID when its artwork is still present, retaining current revisions.
Name collisions get a unique Assets display key; the persisted
authored key remains exact. Detached artwork is still listed in the gallery and
keeps its revisions and composition. Assets labels detached scene/beat artwork.

Writer is the structure editor. Core/CLI `/create scene`, `/create beat` and
`/compose` operations never insert or reorder source declarations. Studio's
Assets derives its Story sequence from authoring associations and retains catalog
definitions, names/descriptions, previews and read-only image history. Atelier
owns image production and saved revision selection; Studio's guide host rejects
visual operations before changes.
Writer's scene/beat buttons open their associated artwork. Resolved references
provide source origins and `Appears in` links. A missing asset can be created as a
catalog entry immediately, with Open in Atelier for artwork. Wrong kinds and
ambiguity remain errors. One GUI-free catalog maps character/environment/object
to Character/Location/Prop and exposes exact completed active revision bindings.

Undo/redo snapshots source text together with the registry, preserving the current
disk snapshot for conflict detection. External refreshes reconcile before saving
the registry. Background authoring operations merge their artifact changes with newer
Writer associations; project switching and undo wait for the worker to finish.
Save failures retain dirty state and revisions. Persisted association corruption
is reported before opening rather than silently rebuilding UUIDs.

Writer uses precise UTF-16 caret positions for keyboard completion, definition
and reference navigation and clickable compiler diagnostics. Shared edits retain
comments, indentation, line endings and prose. Explicit Rename retains associated
artwork UUIDs; direct text renames detach/create as above. During live typing, a
new declaration requires its terminating newline before reconciliation; explicit
save/open snapshots also accept complete declarations at EOF. This prevents
typed name prefixes from creating placeholder artwork.

Basic snapshot playback is implemented in [preview and Player](ROADMAP.md#story-execution-and-playback-reference).
Project-relative image paths are tracked by SF-PLAN-02 in the
[central backlog](ROADMAP.md#0-current-direction-and-actionable-backlog).

## Current storage limits

Image revisions use absolute file paths, so moving the source/database pair does
not produce a portable asset package. The shared project folder and portable asset
references are planned in SF-PLAN-02; see [the roadmap](ROADMAP.md). The future
shared catalog and Atelier-save synchronization are not part of current storage.

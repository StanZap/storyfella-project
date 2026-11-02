# Shared project storage and catalog

SF-PLAN-02 storage contract · 2026-11-01. [ROADMAP](ROADMAP.md) owns task status;
[app responsibilities](app-responsibilities.md) defines the editor boundaries.

## Folder contract

```text
harbor/
  storyfella.toml       version and stable project UUID
  story.sf             authoritative UTF-8 source, preserved byte for byte
  story.db             project metadata, registry, revisions, associations
  artwork/             immutable saved images named by revision UUID
  atelier/
    main.atelier       editable visual document with embedded references/layers
```

Manifest version 1 fixes the entry source/database filenames. This is a
single-source project; SF-PLAN-08 will define modules and entry-point selection
before introducing a source graph. Database schema migrations remain the existing
`ProjectDb` migrations. The manifest UUID must match the database project UUID;
unsupported versions, missing source/database or mismatched identity block opening.

Studio writes source and its authoring associations. Atelier writes editable
`.atelier` documents. GUI-free `studio-core` owns the shared registry and resource
path contract. Saved image revisions and editable documents have independent
identities and lifetimes. Both editors open `storyfella.toml`; core callers may
also open the folder. Atelier's ordinary Save writes `atelier/main.atelier`.
Standalone `.atelier` documents retain their established format and workflow.

The database stores saved revision image paths relative to the project root.
Loads resolve them to absolute host paths for existing compiler/playback/UI
callers. Saving imports newly attached images into `artwork/` before committing
references. A revision filename cannot be replaced with different bytes.
Absolute paths, traversal and resource symlinks escaping the project are rejected
in shared storage. Images preserve their original encoded bytes, including alpha;
all retained revisions travel, not just the active image. A failed registry save
may leave an unreferenced image; it never deletes an existing revision.

Project folders contain no provider credentials, model weights or global
application settings. LoRA/request provenance is retained, but model availability
and provider bindings remain machine configuration. They are not prerequisites
for opening, writing or ordinary playback. Historical request logs may retain
original paths as provenance; current artwork references are portable.

## Creation and adoption

Studio creates new projects as uniquely named folders and discovers manifests
under the configured project directory. Open accepts the manifest, `.sf` or `.db`.
Existing standalone source/database pairs still open in place. From Projects,
Add (+) opens the story form, where **Copy current story to project folder…**
copies the currently open standalone story to a new
folder with its original project/artifact/revision identities and associations.
The source pair and original artwork remain in place. Existing destinations are
never merged or overwritten. The destination manifest is published only after
source, registry and image copying succeed. A failed copy retains its unpublished
partial folder for inspection; retry chooses a fresh destination.

Adoption copies saved authored source, registry/history, adapter metadata and
artwork. It does not transfer an in-flight inference request or a pending writing
review session to a new path; the original project retains that session. Do not
delete the original pair as part of adoption. Existing standalone Save As and
CLI paths remain available; adoption is explicit rather than a destructive
format upgrade on opening.

## Milestone boundaries and verification

SF-PLAN-03 adds shared catalog operations against the same persisted identities.
Cross-app refresh and conflict merging are implemented in SF-PLAN-04 below.
Contextual launch is implemented below; canvas-save publication remains
SF-PLAN-06. Studio's
production controls remain through SF-PLAN-07.

Storage acceptance moves an adopted populated folder, removes the original
source/images and reopens it: exact CRLF/Unicode source, project and artifact IDs,
scene/beat associations, variation grouping and every encoded image revision
survive. Atelier acceptance opens the same manifest, saves an editable document
and embedded image, relocates the folder and reopens that document while the
story remains intact. Failure checks cover existing destinations, missing
artwork/source, version mismatch and project-identity mismatch. Native UI
acceptance remains assigned to the user.

Offline evidence: workspace tests/all-target checks, strict Studio/core/Atelier
Clippy, formatting and the Studio desktop build pass. Logs are under
`.svs-e2e/logs/shared-project-*`; executable relocation tests live in
`crates/studio-core/tests/project_folder.rs` and `apps/atelier/src/tests.rs`.

## Shared asset catalog

SF-PLAN-03 uses the existing registry UUIDs and SQLite tables, with no second
identity or catalog file. Characters, places (`environment`), props (`object`)
and independent `illustration` entries can exist without images. Explicit
variations retain their own UUID and same-kind base link. Illustrations are
independent of source scene/beat identities; this milestone adds no illustration
DSL keyword or new runtime staging instruction.

Studio materializes `cast`, `character`, `location` and `prop` declarations on
valid source snapshots, with no model call. Live typing waits for the terminating
newline; opening/saving an explicit snapshot also accepts a final line without
one. Speech alone can remain an unresolved reference. Invalid drafts retain the
previous catalog; removing usage never deletes an entry, its revisions or visual
documents. Source analysis/compiler remain pure; application reconciliation owns
the creation. Accepted writing proposals use that same path; pending/rejected
proposals publish nothing.

Studio's New artifact/New variant forms default to creating a catalog entry;
image generation is an explicit checkbox. Descriptions may remain empty for
definitions. Reference names use the language's identifier grammar and are
case-insensitively unique. The inspector's Reference name/Rename form updates
only source references resolved to the selected UUID, with one source/registry
undo; prose and unrelated names remain unchanged.

In a shared project, Atelier's Artboards → Assets opens the persisted catalog.
It creates/updates entries and explicit variations, attaches existing image files
and links the current artboard to an asset. Saving the document retains those
asset-to-artboard links. An attached image creates a new immutable revision for
the same asset; it does not create a second named item or replace older artwork.
Catalog edits run on the storage worker, without inference or source writes.
Existing items are renamed in Studio so source ownership and identity remain
coordinated, including references in an unsaved buffer that Atelier cannot see.

Both apps refresh saved catalog changes automatically. Atelier's explicit Refresh
also preserves the current catalog form in the same project. Source remains
untouched when an independent item is created in Atelier. The explicit shared
reference library remains separate.

Schema v11 adds an optimistic registry version and extends the artifact kind
constraint through a transactional table rebuild. It retains existing graph
rows, history and source; foreign-key validation runs before commit and
enforcement is restored. Both GUI writers compare their displayed snapshot's
version during an immediate SQLite write transaction. Stale writes fail without
replacing another editor's work; the refresh/merge workflow below supplies normal
recovery. CLI snapshot commands retain their unconditional behavior and must not
be used as concurrent GUI writers.

Catalog acceptance covers declaration creation, invalid/unfinished drafts,
case-insensitive duplicate/wrong-kind rejection, later image attachment to the
same identity, variations, rename/undo, source protection, real storage-worker
events, artboard links and relocation. Native UI acceptance is assigned to the
user. Logs are under `.svs-e2e/logs/catalog-*`.
Workspace tests/all-target checks, formatting, strict Clippy for all changed
packages, the Studio desktop build and Atelier's signed macOS development bundle
pass. These automated checks do not replace the assigned native UI acceptance.

## Synchronization and concurrent editing

SF-PLAN-04 · 2026-11-01. Studio owns existing catalog names, authored `.sf`
source and scene/beat associations. Atelier owns `.atelier` documents and their
artboard links. Both apps create shared definitions and update descriptions and
saved image revisions through the same catalog; requirements are descriptions,
and work/availability follows the retained revision state. No second status or
identity model is introduced. Atelier never asks for a source rewrite. Studio's
source-aware rename acts on its current buffer and remains one undo operation.

Both apps poll saved state approximately every two seconds on background workers.
This includes revision paths/status and descriptions, not just new names. Studio
keeps the owner mounted on Projects and other screens. Atelier refreshes after
Assets has been opened for the current project, even when the panel is hidden.
Its list and form have separate snapshots: refresh never replaces typed form
values. Epoch/path guards reject results from another active document. Reads
and saves use an existing database; a moved/deleted database is not recreated.
Studio defers refresh and saving during generation, then merges the finished
operation with the latest saved state. Read failures retain work and show a
persistent retry surface rather than repeated notifications.

The GUI-free merge uses the last displayed/saved baseline, local state and latest
saved state. UUIDs identify artifacts, image revisions, proposals, adapters and
operation history. Separate fields on one artifact merge independently; image
histories from both editors remain. Conflicting active-image choices require a
decision while retaining both image revisions. Overlapping fields leave both
snapshots untouched until the person chooses. Studio's Review changes dialog
shows the conflicting local/saved values and offers either side for those fields.
Unapplied inspector descriptions participate in this check. Choosing a side also
resolves conflicting drafts; unrelated drafts stay editable.

Studio rebases undo/redo snapshots with the external delta, so undo cannot erase
another app's new item or artwork. Structurally incompatible history steps are
removed rather than restored over shared work. Two newly created identities
with the same name are never silently combined. A structural/name collision
offers an explicit Reload saved catalog decision: source text and drafts for
retained items stay, while unsaved catalog edits and joint project undo history
are discarded. The saved item's original UUID remains authoritative. Cancel
retains the local state without publishing the conflicting catalog.

Atelier's failed details form remains populated. It shows the conflicting values
and can keep its draft or discard it and load saved details. Keeping a draft is
bound to the saved version shown in the conflict; another intervening save
requires reviewing again. Normal submissions automatically merge independent
changes, then use SQLite's optimistic commit check to catch a race after the
read. Attaching an image refreshes availability without clearing an unrelated
details draft. Normal document saves still only save the editable document;
automatic linked-canvas image publication remains SF-PLAN-06.

### Paired source/catalog recovery

Schema v12 adds a transient `source_write` record. Studio's authoring save imports
immutable image resources, validates the source's disk baseline, then commits
the registry plus the exact old/new source intent in one SQLite transaction.
Only after that succeeds is `.sf` replaced. A failed catalog commit publishes
no source and leaves the local buffer dirty. A failed source publish leaves its
intent durable; load/retry finishes it if the file still equals either side.
Reads serialize briefly with cooperating publishers so they cannot expose a
catalog paired with an intermediate source. The record is removed after success;
the standalone `.sf` remains the authoritative source during ordinary use.

An unrelated source edit blocks automatic recovery. Studio exposes Recover
interrupted save: an explicit confirmation first preserves the current file as
`story.conflict-<UUID>.sf` beside the original, then finishes the committed save.
The copy uses exclusive creation and preserves exact UTF-8 bytes; another file
change still blocks replacement. Its location remains visible after success.
Dirty editor text remains protected by its usual source conflict check and the
Writer's Save copy/Reload workflow. There is no force overwrite or silent source
merge. Standalone database/source pairs use the same protection.

Manual acceptance with both apps open: create a definition in each app; wait
for it in the other; rename it in Studio with unsaved writing; edit a description
in each app; resolve the conflict both ways; attach saved artwork in Atelier;
undo a Studio edit and verify that the external image remains; retry a failed
save; close/reopen and confirm IDs, source and artwork. Also try simultaneous
same-name creation and cancel/recover the explicit collision decision. Native
interaction is assigned to the user. Automated evidence is in
`crates/studio-core/tests/catalog_sync.rs`, persistence/state tests and Atelier's
real storage-worker test. Validation logs use `.svs-e2e/logs/catalog-sync-*`.

## Open in Atelier

SF-PLAN-05 · 2026-11-01. Canvas details and resolved Writer references expose
**Open in Atelier**; the Writer toolbar also opens the current scene. The action
saves source/catalog changes and description drafts through the normal guarded
command before delivery. Save errors retain work and prevent launch. Standalone
projects first use Projects → Copy current story to project folder; the disabled
action's tooltip states this prerequisite. No handoff inserts DSL, invokes
inference or publishes a new image revision.

The bounded, versioned request binds manifest/project/item UUIDs, description
and relevant scene text. Atelier revalidates those identities on its storage
worker. A private `handoff/` inbox under Atelier's data directory supports cold
starts and a running receiver. An OS file lock permits one receiver per data
directory and releases on exit/crash. Timestamped atomic files preserve order;
completion replies report successful opening or a local failure. Studio keeps
busy state until the reply, exposes failure Details and times out after 30
seconds. Retry reuses the item identity. Context is plain text, never executable
instructions. The transport uses no network or shell interpolation.

macOS launch discovery checks a sibling bundle, the checkout's
`dist/atelier/Atelier.app`, `/Applications/Atelier.app`, then Launch Services by
bundle ID. Installation in `/Applications` is optional. Other hosts use a sibling
executable or `atelier` on PATH. `STORYFELLA_ATELIER` selects an explicit
executable/bundle. Set `STORYFELLA_ATELIER_DATA_DIR` in both apps for isolated
data/inbox testing; Atelier's existing `--data-dir` remains supported. Restart
older builds before testing the protocol.

Atelier reuses the open `atelier/main.atelier` tab and its unsaved document.
`asset_links` selects the same artboard on repeated opening. A first attachment
creates a linked artboard, embeds the current saved image when available and
prefills its prompt with the description. Existing layers and unsent prompts
are preserved. Read-only `story_context` is stored by artboard and displayed in
a named panel, reopened from its canvas toolbar action. Normal document save
and recovery retain links, context and prompts. A pending/failed mask save blocks
handoff and returns an error to Studio. Reassigning the asset clears old context.

Automated evidence: `crates/studio-core/tests/atelier_handoff.rs` and Atelier's
`studio_handoff` storage/document test cover ownership guards, relevant context,
source preservation, instance leases, FIFO/cancel/replies, invalid input, artwork
embedding, retained edits/prompts, repeated opening, atomic validation failure
and reopening. Logs use `.svs-e2e/logs/atelier-open-*`. Native cold launch,
running-instance focus, project switching and visible controls remain manual
acceptance assigned to the user. Linked-canvas Save publication is SF-PLAN-06.

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
Cross-app live change notification, conflict merging, contextual launch and
canvas-save publication belong to SF-PLAN-04–06. The initial shared folder does
not imply those integrations or authorize both apps to replace stale registry
snapshots concurrently. Studio's production controls remain through SF-PLAN-07.

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
Atelier can rename an unused item. An item referenced by saved source must be
renamed in Studio so source ownership and identity remain coordinated.

Opening/reopening Studio reads Atelier-created items from the same database;
Atelier's Refresh reads Studio's saved definitions. Refresh preserves the current
catalog form in the same project. Source remains untouched when an independent
item is created in Atelier. The explicit shared reference library remains separate.

Schema v11 adds an optimistic registry version and extends the artifact kind
constraint through a transactional table rebuild. It retains existing graph
rows, history and source; foreign-key validation runs before commit and
enforcement is restored. Both GUI writers compare their displayed snapshot's
version during an immediate SQLite write transaction. Stale writes fail without
replacing another editor's work. This is an initial protection, not the live
merge/notification workflow in SF-PLAN-04. Atelier can refresh/retry its retained
form. A stale Studio session retains its local work; preserve any desired draft
with Save copy, close without saving that stale catalog and reopen the project.
Automatic reconciliation of concurrent edits remains SF-PLAN-04. CLI snapshot
commands keep their existing unconditional behavior and should not run as a
concurrent GUI writer in this first slice.

Catalog acceptance covers declaration creation, invalid/unfinished drafts,
case-insensitive duplicate/wrong-kind rejection, later image attachment to the
same identity, variations, rename/undo, source protection, real storage-worker
events, artboard links and relocation. Native UI acceptance is assigned to the
user. Logs are under `.svs-e2e/logs/catalog-*`.
Workspace tests/all-target checks, formatting, strict Clippy for all changed
packages, the Studio desktop build and Atelier's signed macOS development bundle
pass. These automated checks do not replace the assigned native UI acceptance.

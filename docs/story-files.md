# Story source files in Studio

The Writer reads and writes UTF-8 `.sf` files. SQLite stores project metadata,
artwork, revisions, operation history and artifact-scoped generated text proposals.
It no longer stores the authored Writer source.

## Project files

Each project is a pair in the same directory:

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

## Editing and saving

Writer edits retain comments, CRLF/LF, Unicode and unfinished or invalid syntax.
Parsing diagnoses a draft; it does not rewrite source, reject saving or delete
artwork. An unbound Writer draft is retained when creating its first project.

Autosave and Cmd/Ctrl+S use the same project command. Source saves stage a complete
file beside the original, flush it, and replace it with a rename. A save first
compares disk text with the document's last saved snapshot. If both disk and
Writer have changed, it reports `SVS-STO-110`, keeps the local draft, and leaves
the external file untouched. A clean Writer refreshes external edits when saving.
Opening a project also loads current disk text; there is no background file watcher.

Writer → Save copy saves local text to a new file without changing the open
project or overwriting an existing file. Writer → Reload .sf reads disk text,
asking for confirmation before discarding local edits. Failed reads leave the
editor intact. A missing source in an upgraded project is an error rather than
an empty draft that could erase the story.

The source file and SQLite registry snapshot are separate writes. If source
saving succeeds but registry saving fails, Studio keeps the new source snapshot
and the project's dirty flag, so retrying does not mistake its own save for an
external edit. A failed source save prevents the registry save. Switching projects
first saves pending changes; a failure leaves the current project open.

## Migration from SQLite drafts

Schema v9 exports the actual v8 `projects.story_source` column before dropping
it. An existing identical `.sf` file permits retrying a partially completed
migration. An empty stored draft permits an existing source file to remain
authoritative. Different nonempty copies block migration: the original column
and schema version remain intact, and the `.sf` file is not replaced. Reconcile
the two copies before retrying. Export failures also preserve the database draft.

There is no parallel SQLite source writer, old-format parser or compatibility
alias. The export is an active schema upgrade for existing projects.

## Validation and next work

Filesystem tests cover exact text, malformed drafts, external edits, copy/reload,
missing files, migration/retry and moved project pairs with stable artifact IDs.
Studio state tests verify joint source/registry undo and draft preservation.
A headless Dioxus harness exercises the actual project commands,
including conflict-blocked navigation and retrying a partial project save.

## Scene and beat reconciliation

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
Name collisions get a unique Canvas display key; the persisted
authored key remains exact. Detached artwork is still listed in the gallery and
keeps its revisions and composition. Canvas labels detached scene/beat artwork.

Writer is the structure editor. Existing `/create scene`, `/create beat` and
`/compose` commands create library artwork only; they never insert or reorder
declarations. Canvas's Story sequence is derived from the association. Generation,
composition, descriptions, variants and revision controls are artwork operations.
Writer's scene/beat buttons open their associated artwork. Resolved references
provide source origins and `Appears in` links. A missing asset can be created as a
catalog entry immediately, with rendering as a separate action. Wrong kinds and
ambiguity remain errors. One GUI-free catalog maps character/environment/object
to Character/Location/Prop and exposes exact completed active revision bindings.

Undo/redo snapshots source text together with the registry, preserving the current
disk snapshot for conflict detection. External refreshes reconcile before saving
the registry. Background artwork results merge their artifact changes with newer
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

Project-relative image paths and playback remain work in
[the language plan](language-plan.md).

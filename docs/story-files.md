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
Studio state tests verify that editing source marks it dirty without changing
the registry. A headless Dioxus harness exercises the actual project commands,
including conflict-blocked navigation and retrying a partial project save.

Scene/beat synchronization, project-relative image paths, Writer completion and
navigation, artifact origins/backlinks and compiler diagnostics in Writer/LSP
remain separate work in [the language plan](language-plan.md).

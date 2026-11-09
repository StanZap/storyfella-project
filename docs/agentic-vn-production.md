# Agentic visual novel production architecture

The first delivery uses Studio for writing and direction, Atelier for image
work, and the shared Studio Preview and Player presentation for playtesting.
The [research](agentic-vn-production-research.md) describes the larger product.
The [roadmap](ROADMAP.md#agentic-visual-novel-production) owns priorities and
completion status.

## Start a text-first VN from a chat

Build `svs` with `cargo build -p svs-cli --bin svs`. In an MCP-capable chat
client, add a local stdio server. Set the executable to the absolute path to
`target/debug/svs` and use these arguments, replacing the project folder.
Choose an existing parent directory.

```json
["mcp", "--folder", "/absolute/path/to/the-last-signal", "--create-if-missing", "The Last Signal"]
```

`--create-if-missing` initializes a new shared folder and reuses it on later
connections. Omit that option to require an existing folder. Close the
project in Studio before connecting. The server is a headless authoring
owner and holds Studio's writer lease for that one folder until it exits.
Disconnect the server before opening the project in Studio. The command
uses standard input and output for MCP and needs no image or text provider
configuration.

Ask the chat to write a short branching chapter, preview and save its edit,
and playtest both routes. The chat uses these tools.

| Tool | What it does |
| --- | --- |
| `storyfella_project_inspect` | Returns the project identity, session, source filenames and current version. |
| `storyfella_story_read` | Reads the saved entry or one of its included files in UTF-8 byte pages. |
| `storyfella_story_edit` | Validates one byte-range replacement in the entry against the full include workspace and current catalog. Repeat the exact request with the returned proposal to save. Syntax errors return diagnostics. |
| `storyfella_story_playtest` | Plays the saved version through the shared story runtime, taking zero-based choice indexes and returning dialogue, source positions, flags and pending artwork counts. |

For a whole-entry edit within the 64 KiB replacement limit, use `start: 0`
and the entry's reported `bytes` as `end`. An edit needs the session and
the current version from inspection or reading. A saved source, include or
catalog change requires a fresh preview. Playtests are text-only and stop at
an unanswered choice, completion or the step limit. Included files are
readable. The entry is the only editable file in this first interface.
If a save is interrupted, restart the MCP server to recover it under the
project lease. Inspection and edit preview leave a pending source save intact.
Projects with broken include paths need repair in Studio before MCP edits.
Art generation, visual frame capture and editing while Studio is open
remain later parts of the workflow.

## Project ownership and checked edits

The first local stdio server owns a closed project and uses the same compiler,
catalog rules and checked authoring store as Studio. Live desktop access will
use a local stdio MCP bridge with app-owned commands. Studio owns open Writer
buffers, source and authoring associations. Atelier owns open visual documents,
candidate review and publication. Each app serializes agent requests with its
normal editor commands on the app's state thread, then runs storage and
inference off that thread through its existing workers. Keep Studio core
GUI-free. Reuse the handoff's local instance discovery and lifecycle patterns.
The existing Atelier inbox serves an open-and-focus request and its lease
covers a data directory. Studio now holds a project writer lease while editing
a shared folder, including through background refresh and recovery. Atelier
document ownership and an authenticated command channel are separate work.

Grant a local client access to an explicitly selected project folder and to
specific scopes for inspection, editing, publishing and provider submission.
An agent with edit access can save an editable Atelier snapshot. Updating
the shared artwork additionally requires the publish scope.
Bind a session handle to the manifest UUID, canonical root and current app
owner epoch. Use an OS-backed, per-project lease for each owner domain before
enabling agent writes. All app entry points that write that domain must
participate, including alternate app data directories. Keep the lease through
completion of queued writes. Check the project identity, scope, owner epoch
and expected revision in the owner immediately before mutation. Never redirect
an old handle to whichever project is open. If the owner is unavailable,
report how to reopen and rebind, with writes disabled until the new owner
validates the project and restores recoverable work. Use a private local
channel with endpoint permissions and per-session credentials. Treat project
content, prompts and images as data. Validate project-relative paths and
resolved targets. Keep raw filesystem paths, arbitrary uploads and credentials
out of tool arguments and results. The lease coordinates participating apps;
version checks still protect against external disk edits.

Opening a folder through the current `ProjectFolder::open` path can migrate
storage and recover a pending source write. Studio's authoring open acquires
the lease before loading. The Studio Projects browser uses read-only SQLite
for shared folders and indexes the saved text on disk. A pending publication
can leave that text behind the catalog until authoring recovery completes.
Serve agent inspection from the owning app's snapshot, with a separate grant
for any recovery command.

Return versions for each domain separately. Source edits address one file
using UTF-8 byte ranges and a content digest, with a separate saved baseline
and editor revision. Propose and compile a change against the full include
workspace, then apply it to the matching draft through Studio's normal undo
path. An edit based on a saved version conflicts with a newer dirty buffer.
Expose a bounded diff and the current revision so the client can reread and
propose a fresh edit. Initially use one source file per command. Studio's
normal Save writes included files separately before the checked entry/catalog
commit. Report the files saved if that process stops partway. Entry/source
intent recovery and catalog version checks stay in `ProjectDb`. Review sessions
and pending suggestions remain separate from accepted Writer text.

Use registry UUIDs for assets and revision UUIDs for images. Scope numeric
artboard IDs to the document. Gate operations on the document revision, and
add stable persisted layer IDs before exposing independent layer references.
The first mask operation can address a specific artboard revision and a
rectangle in unrotated artboard pixels, with origin at the top left. Use a
bounded crop from that same coordinate space for feedback. Build candidate and
pixel operations around the existing document and workflow code. Extract GUI
dependencies as each real command needs them.

## Recovery boundaries

Atelier persists the editable document first, then renders and publishes
accepted art with per-artboard expected revisions and a checked registry
commit. It finally saves the new links in the editable document. Save now
classifies failures by these three stages. If the artwork committed and the
last write failed, its revision IDs travel back to the open tab, where they
apply only to links that still match the saved baseline. Reopening the saved
document and retrying the same render reuses the already published revisions.
Report an artwork conflict with its current revision and a way to inspect it
before relinking. The registry commit is atomic. Failed publication can leave
unreferenced image files on disk. Recovery snapshots save editable work
without publishing.

Atelier's queue and provider result attachment are separate from tab recovery.
Persist the job and request fingerprint under a project-scoped idempotency
key before external submission. The same key and same request return the
existing job. A different request with that key is a conflict. Capture the
provider/model, immutable input references, mask, destination and destination
revision. Record dispatch before the provider call and store the provider ID
when available. On restart, resume polling a known provider job where supported.
A submission with unknown disposition requires reconciliation or an explicit
new attempt. Persist a successful result before attachment. If the destination
has changed, retain the result for review and checked reattachment without
another provider call. Distinguish requested cancellation, confirmed remote
cancellation and suppressed local attachment. Show those distinctions in
Atelier's existing Activity and Queue surfaces.

## Application features and MCP surface for the first chapter

| Area | App work | MCP operations |
| --- | --- | --- |
| Coordination | Show project/client scope, owner availability, conflicts and recovery. Route shared commands through the same app state and undo paths. | Open/inspect explicit project handle, capabilities, bounded changes since a cursor. |
| Writing and direction | Reuse Writer review, diagnostics and catalog links. Add a chapter dependency view for scenes, choices, required variations and missing images. Start with asset descriptions and authored references. | Inspect sources, graph, dependencies and diagnostics. Propose checked source edits and apply a matching proposal. Create or update catalog items with expected versions. |
| Artwork | Reuse linked artboards, crop and group editing, rectangle/paint masks, ordered references and candidate layers. Add project-scoped job recovery and a bounded before/after review with mask overlay. | Inspect artboard and image region. Set a checked rectangle mask, query provider capabilities, submit/poll/cancel, inspect candidate pixels, accept/reject, attach retained results and publish. |
| Playback | Add isolated playtest sessions and capture through the shared desktop playback presentation. Keep the user's interactive Preview and Player state independent. | Start, step/choose, inspect, checkpoint/restore and capture. Return plan identity, source location, flag state, exact image bindings and the actual stage PNG. Report both route traces. |

Version the MCP schemas independently of project, document and runtime formats.
Use bounded structured results for metadata and native MCP image blocks for
thumbnails, crops and captures. The current Agentkit adapter converts
`metadata.structured_content` to structured results. Its synchronous registry
and string content omit native image blocks from outgoing responses. A generic
Agentkit transport enhancement should preserve structured metadata alongside
image blocks and dispatch bounded async commands. Keep Storyfella operations
in Storyfella. Test with two independent clients before claiming
interoperability. Use durable polling for jobs and avoid making client task
extensions a prerequisite.

For playback capture, render the same `storyfella-playback-ui` stage and styles
at a specified viewport and device scale. Wait for the requested playback step,
fonts and image decoding before capturing. Return viewport and crop geometry
with the PNG. A pinned image loaded by the runtime establishes the correct
asset revision. The rendered capture is the evidence for layout, layering,
dialogue readability and clipping. Start with two viewport aspect ratios.

## Acceptance and current evidence

The offline chapter, *The Last Signal*, has three characters, two locations and
two expressions for the lead. The choice sets a persistent trust flag on one
route. A later conditional selects the line and the expression. The Atelier
test `branching_chapter_survives_partial_publication_and_stale_authoring`
creates the shared folder, saves the chapter and catalog through the checked
authoring store, rejects an invalid candidate destination, composites a masked
edit, and verifies pending candidates stay out of playback. It injects a
post-publication write failure, reopens the drawing, retries without creating
a duplicate revision, and rejects a stale authoring commit while preserving its
dirty draft. Both routes restore their flag and source position, and the test
checks the changed and protected pixels in the exact image bytes loaded by
playback. An older play session keeps its pinned pixels. The fixture uses
procedural images and the CPU rasterizer.

Related code and tests establish the underlying guarantees.

- Studio's `commands.rs` and `state/mod.rs` tests cover multi-file save
  conflicts, dirty source preservation and undo after a catalog refresh.
  Studio core's persistence tests cover interrupted source-intent recovery.
  The Studio project command probe opens the same folder through multiple
  paths, verifies a second owner is refused while a draft is dirty, then
  opens it after the first owner switches projects. The persistence test
  verifies passive discovery leaves a pending source intent untouched.
- The `svs` stdio tests create and playtest a branching chapter using a raw
  JSON-RPC client, then reopen the same project with Agentkit's MCP client.
  They check competing owners, invalid edits, changed includes, stale
  proposals and a replaced lease file.
- Atelier's `tests.rs`, `queue.rs` and `shared_save.rs` cover captured
  destinations, protected pixels, abort and per-artboard publication
  baselines. The current in-memory queue cannot restore an interrupted job.
- The runtime's `execution.rs` tests cover flags, conditional routing and
  verified checkpoint replay. Studio core's `playback.rs` tests cover pinned
  resources and multi-file source maps. Shared playback UI tests cover markup
  and navigation.

Complete the slice with both apps open, including a deliberately dirty source
and artboard, and drive it through independent MCP clients. Use a deterministic
provider fixture for queue restart, ambiguous submission, cancellation and
attachment recovery. Capture both rendered routes at two aspect ratios.
Separately run configured-provider and native-renderer acceptance with
reference-led expression and outfit variants. Record model settings, cost and
human review of visual consistency. Structured bibles, larger batches, richer
narrative state, segmentation, layered sprites, and release features follow
under the roadmap.

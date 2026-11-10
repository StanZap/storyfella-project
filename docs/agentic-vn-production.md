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
| `storyfella_story_example` | Returns playable reference source with a choice, flag and conditional endings. Call this before drafting a branching chapter. |
| `storyfella_story_read` | Reads the saved entry or one of its included files in UTF-8 byte pages. |
| `storyfella_story_edit` | Validates one byte-range replacement in the entry or a named included file against the full include workspace and current catalog. Repeat the exact request with the returned proposal to save. Diagnostics identify the source file. |
| `storyfella_story_playtest` | Plays the saved version through the shared story runtime, taking zero-based choice indexes and returning dialogue, source positions, flags and pending artwork counts. |

Edit diagnostics report the file, the half-open UTF-8 byte range
(`start` and `end`), and the corresponding `start_position` and
`end_position`. Each position has a zero-based `line` and a zero-based
`character` column in UTF-16 code units.

Omit `file` to edit the entry or pass an included filename from project
inspection. For a whole-file edit within the 64 KiB replacement limit, use
`start: 0` and that file's reported `bytes` as `end`. An edit needs the session
and the current version from inspection or reading. A saved source, include
or catalog change requires a fresh preview. Each request edits one existing
file at its canonical path. Use Studio to create new chapter files and add
their includes. The current workspace limit is 256 files and 1 MiB of source
text. Playtests are text-only and stop at an unanswered choice, completion or
the step limit.
If a save is interrupted, restart the MCP server to recover it under the
project lease. Inspection and edit preview leave a pending source save intact.
Projects with broken include paths need repair in Studio before MCP edits.
For live Writer edits, see [Connect to the running apps](#connect-to-the-running-apps).

## Produce linked artwork through Atelier MCP

Build the server with `cargo build -p storyfella-atelier --bin atelier`.
Prepare a shared folder with pending visual assets in Studio. Close the
project in Studio and use **Save and close project** if the shared drawing
is open in Atelier. Configure a vision-capable chat client to launch the
absolute path to `target/debug/atelier` with these arguments.

```json
["mcp", "--folder", "/absolute/path/to/chapter", "--allow-generate", "--max-submissions", "6", "--allow-publish"]
```

Atelier reads the provider, generation model and edit model from its existing
Settings. Pass `--data-dir DIRECTORY` to select a separate Atelier workspace.
Provider submission and publication are separate launch grants. Set
`--max-submissions` to the agreed number of attempts for a paid run and keep
that flag on reconnect. It counts the project's recorded submissions across
sessions, including attempts without a completed result.

| Tool | What it does |
| --- | --- |
| `atelier_project_inspect` | Returns the session, provider settings, visual asset IDs, variant relationships, published revisions, submission count and recent job IDs. |
| `atelier_asset_inspect` | Returns a target asset's artboard and the saved drawing version. |
| `atelier_candidate_generate` | Queues a prompt for one pending asset. Use its base asset's current `reference_revision` to generate a variant, or `revise: true` to edit a pending candidate in place. A new artboard can specify `size: [width, height]`. |
| `atelier_job_poll` | Returns compact status by project job ID and attaches the finished result if the captured destination and drawing version still match. |
| `atelier_job_cancel` | Requests cancellation of a running job and persists a block on automatic attachment. A late result can still be inspected and explicitly recovered. |
| `atelier_job_thumbnail` | Returns a native PNG of a retained result, up to 512 pixels on the long edge. Leaves the drawing and reviews untouched. |
| `atelier_job_recover` | Reattaches a completed result to its original pending artboard at the captured size, using a freshly inspected drawing version. This explicitly replaces any pending candidate there. |
| `atelier_candidate_preview` | Returns a native artboard PNG and a review ID for the exact rendered candidate. |
| `atelier_asset_publish` | Saves and publishes only the asset named by the approved review. Returns its immutable revision UUID. |

The user chooses the review workflow in chat. If they explicitly delegate
artwork approval, inspect each PNG and pass `agent_approval: true` with the
exact review ID to `atelier_asset_publish` for each image you accept. Skip
images you reject. Otherwise, use human review.

Keep the MCP connection open through each review. For human review, the chat
shows the preview and its review ID. The human runs this command from their
own local terminal for each candidate they accept.

```sh
/absolute/path/to/atelier approve --folder /absolute/path/to/chapter --review REVIEW_UUID
```

Review IDs bind to one artboard and its artwork baseline. A change to a
different artboard leaves that review valid. Revising or recovering a
candidate on the reviewed artboard invalidates its review. Use the current
saved drawing version for generation, recovery and preview calls. If
publication fails partway, retry the same approved review within the session.
After reconnecting, preview and approve the saved drawing again. The checked
save reuses an identical committed revision if only its link failed to save.
Disconnect Atelier MCP before opening the folder in Studio Preview.

This workflow holds both project leases. Atelier's desktop recovery
reservation still protects a draft after exit. Open it in the original
Atelier workspace and save and close the project to return ownership. A
desktop recovery that cannot reclaim the original opens a separate copy.
Save As creates a copy and keeps the original open.

MCP jobs use the existing Atelier queue and image workflow. Each submission
has a project-local receipt, and the workflow retains completed originals
under `atelier/mcp-jobs/` before attachment. Reconnect and use the same job ID
with the new session. A `retained` result is available for thumbnail and
checked recovery. An `unknown` result has no saved output. Reconcile it with
the provider before starting a new attempt. The server does not resume
remote predictions or automatically resubmit them. Cancellation distinguishes
a pending request (`cancelling`), a locally stopped job with no result
(`cancelled`), and a late `retained` result. Remote cancellation and billing
remain unconfirmed.

## Project ownership and checked edits

The closed-project stdio server uses the same compiler, catalog rules and
checked authoring store as Studio. The live stdio bridges dispatch to the
desktop owners. Studio owns open Writer buffers, source and authoring
associations. Atelier owns open visual documents, candidate review and
publication. Each app serializes agent requests with its normal editor
commands on the app's state thread. Compile and run storage or inference on
the app's workers. Keep Studio core GUI-free.
The existing Atelier inbox serves an open-and-focus request and its lease
covers a data directory. Studio now holds a project writer lease while editing
a shared folder, including through background refresh and recovery. Atelier
now holds a separate per-project drawing lease across desktop data directories
and MCP.

### Connect to the running apps

On macOS or Linux, launch each app with the selected absolute project folder
and its own new absolute channel directory under a parent you control. Keep
the channel path short enough for a Unix socket. Studio grants inspection by
default. Set `STORYFELLA_MCP_EDIT=1` on the Studio process to allow checked
draft edits.
Atelier grants drawing inspection.

```sh
STORYFELLA_MCP_FOLDER="$project" STORYFELLA_MCP_CHANNEL="$writer_channel" \
  STORYFELLA_MCP_EDIT=1 /absolute/path/to/storyfella-studio
ATELIER_MCP_FOLDER="$project" ATELIER_MCP_CHANNEL="$drawing_channel" \
  /absolute/path/to/atelier
```

Open the project in Studio and its drawing in Atelier. Configure the two MCP
clients with the respective arguments below.

```sh
svs mcp --folder "$project" --studio-channel "$writer_channel"
atelier mcp --folder "$project" --atelier-channel "$drawing_channel"
```

`storyfella_draft_inspect` returns the connection's session, the draft version
and separate entry and saved digests. Use `storyfella_draft_read` to read the
open entry, including unsaved edits. `storyfella_draft_edit` checks a UTF-8
byte range against the expected version and compiles the replacement with the
current includes and catalog. Set `apply: true` to apply a valid replacement as
one Writer undo step. Studio's existing save settings control persistence.
Reread on `stale_version`. A connection that outlives its owner must reconnect
and inspect again. After an app crash, use a new channel directory.

`atelier_drawing_inspect` reports the drawing's in-memory revision, dirty
state and bounded artboard metadata, including candidate presence. Live
included-file edits and live Atelier mutations remain under SF-PLAN-33.

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
commit. Report the files saved if that process stops partway. Source intent
recovery and catalog version checks stay in `ProjectDb`. The MCP edit of an
existing included file journals that file and the catalog together. Review
sessions and pending suggestions remain separate from accepted Writer text.

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
and string content omit native image blocks from outgoing responses. Atelier
uses Agentkit’s underlying MCP transport directly to return structured metadata
and native images. Agentkit’s generic registry adapter still needs
image forwarding and bounded asynchronous dispatch. Keep Storyfella operations
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
  baselines. Atelier's MCP tests cover reconnecting to recover a completed
  result, using the exact published reference and publishing only the
  approved artboard. The desktop queue remains in memory.
- The runtime's `execution.rs` tests cover flags, conditional routing and
  verified checkpoint replay. Studio core's `playback.rs` tests cover pinned
  resources and multi-file source maps. Shared playback UI tests cover markup
  and navigation.

The macOS coordination run opened both apps against one disposable project.
Independent stdio clients observed a dirty Writer draft and a newly linked
unsaved artboard. The Writer bridge rejected an old version, applied a fresh
edit while preserving the local text, and required reconnecting after Studio
released and reacquired the project.

The offline provider fixture verifies restart, ambiguous submission, local
cancellation, and recovery of a late retained result. Native Preview ran both
branches on a copy of *The Hour of the Beacon*. The driver asserted exact
staged and background revisions. Native frames record the opening and both
branches at two aspect ratios, including the missing `ivo_worried` placeholder.
Studio plan launches accept `--window-size WIDTHxHEIGHT` and `--keep-window`
for frame capture, and `--manual-save` to turn off editor autosave for the run.
These checks and their evidence are in `.svs-e2e/coord/acceptance.md`. The paid
run and its review remain recorded separately in
`.svs-e2e/live-branching-chapter/live-run.md`.

The complete visual pass still calls for the pending expression, outfit
variants and review of their visual consistency. Structured bibles, larger
batches, richer narrative state, segmentation, layered sprites and release
features follow under the roadmap.

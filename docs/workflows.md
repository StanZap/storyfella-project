# Application workflows

Studio can open with a hosted image default or a text-only provider registry.
Writer assistance needs only the selected text provider. Studio's Models page
stores its text and vision choices locally and can refresh the shared provider
catalog after setup in Clipforge. Image setup belongs outside Studio. Native-only
shared-core requests report SVS-CFG-110 before starting a server or provisioning
weights when no native image deployment is selected. Shared hosted defaults
remain available to Atelier and are not rewritten.

Studio and Atelier consume the public `agentkit-workflow` runner and durable
controller. Storyfella does not own another graph scheduler or model transport.
The GUI-free adapter lives in `studio-core::workflows`; Agentkit remains an
external dependency and contains no Storyfella types.

`RunStore` implements Agentkit's transactional run and event contracts using
SQLite. A revision check, complete state snapshot and contiguous event batch
commit together. Definitions, source context and results are stored locally;
provider bindings and credentials remain ephemeral. Studio uses a
`<project>.workflows.sqlite3` companion; Atelier uses `workflows.sqlite3` beneath
its existing data root. These files are separate from ordinary project snapshot
saves. Studio also stores writing workspace checkpoints here. Recovery never
automatically repeats a model call. Reactive multi-turn sessions remain tracked
in SF-PLAN-28.

## Writer

### The assistant

Write docks the assistant on its right; dragging the dividers resizes it
and the file list, for as long as Studio runs, and double-clicking one
restores its width. A project keeps any number of conversations in the
story's `workflows.sqlite3` beside `story.db`; the newest shows when the story
opens. The header's sidebar button shows or hides the list of them beside the
transcript — a column that resizes like the others — grouped by how recently
each changed. Choosing one shows it; its menu renames or deletes it, and Clear
history deletes them all, each after asking. Deleting never touches the story.
New conversation starts an empty one, named by its first request; the title in
the header renames it. One reply runs at a time: it keeps writing into its own
conversation while another is shown, and its row shows a dot. A turn is a
request and its reply; a reply lists the steps the
agent took and ends in one outcome: done, stopped, failed, or interrupted
(Studio closed while it ran).

The assistant edits the story as it works. Each edit goes into the draft at
once — the editor shows it, autosave writes it — and is its own undoable
change. While a reply runs, its steps stay open: Starting the text model
(with the model and its address, or "Loading … on this computer" for a
managed one), Thinking, and one step per tool call — Reading, Editing,
Rewriting, Creating, Moving scene — each recording what the compiler said
afterwards. The running step's time counts up each second, and while the
model answers its line says what is arriving — "Thinking · 1,419
characters", then "Writing · 201 characters" — with the answer's readable
part shown as a draft underneath. When the reply ends, the steps fold to
"Worked for 12.7s · 6 steps" — "Ran for 12.7s" when it failed — which opens
them again.

A request names its reach with the scope chip beside Send: Project (the
default), File, Scene, Beat or Selection. The scope is enforced by the
agent's tools, not by the prompt: under Project any file may change and new
files may be created; under File only the current file; under Scene, Beat or
Selection only that range of lines, which grows and shrinks with the agent's
own edits. An edit outside the scope is refused with the reason, and the
agent says in its answer what it could not change. Whatever the scope, the
request carries where the writer is — the file, the caret, the selection,
and the beat and scene around the caret — so "this beat" or "the selection"
means the same under Project. The instructions and Keep facts (on by
default) are captured when the request is sent. Selection with nothing
selected is refused under the prompt, and the prompt keeps its text. Without
a text provider in Settings, the prompt says so and Send is disabled.

A reply ends with what the model said and a card listing each changed file
with the lines added and removed. A request that is not an edit — a
greeting, a question about the story — is answered in prose and changes
nothing. A reply that used every step says so and offers Continue, which
asks again with the same instructions, scope and Keep facts, and with what
the previous answer said was done and left.

Changes not yet decided are listed above the editor: "The assistant changed
chapter.sf" (or "is changing", while it runs), with Review, Undo all and Keep
all, which are unavailable while a reply runs. Review replaces the editor
with a diff of each changed file against how it was before the assistant
first touched it; each change is kept or undone on its own, and the review
bar steps between files. Ending the review applies the decisions as one
undoable change. Keep all accepts everything; Undo all puts every file back
as it was. A file the assistant created is reviewed against nothing; undoing
all of it removes the file, which a save then deletes from disk. Edits made by
a live MCP assistant job wait here the same way. What waits is kept in the
project's `workflows.sqlite3`, so it is still there when the project is opened
again. A file the writer edits after the assistant does is reviewed with both
edits, against how it was before the assistant.

A file the story gains — from the assistant or an undo — exists in the draft
until it is saved: it is written then, and a file the draft no longer has is
deleted then, each only while the disk still matches what Studio last read. A
change that would leave the story unreadable, or remove a file the story still
includes, is refused.

A failed reply says so once, in a card under its steps: the code's label
("The assistant stopped with an error"), Try again, which sends the same words,
and Details, which opens the shared details dialog (DESIGN.md §2). Edits made
before the failure stay in the story, to be kept or undone like any other.
A reply interrupted by Studio closing offers Try again too.

### The story agent

`storyfella_studio_core::workflows::agent` runs a request as one Agentkit
`language_model` step with a tool source, through the text-provider binding
from Settings. A managed (`llamacpp`) provider is reached through the worker
the shared inference service allocates — the address in `clipforge.toml` is
only the provider's identity — and its lease is held for the whole request; a
hosted provider is called at its configured address.

The first turn carries the request, its scope, Keep facts, the editor
location, an outline of every story file with its scenes and beats, the
assets the story may name, and the compiler errors the story already has. The
agent works through six tools:

- `outline` — every file with its scenes and beats.
- `read_file` — numbered lines, a range at most 400 lines and 32 KiB.
- `edit_file` — replaces text that occurs exactly once, inside the scope.
- `write_file` — replaces a file, or creates one under Project scope and adds
  its `include` to the entry file.
- `move_scene` — moves a whole scene, word for word, to another file, under
  Project scope.
- `rename_scene` — renames a scene's declaration and every `goto` and `option`
  that names it, in every file, under Project scope. It uses the editor's
  rename, so the scene's artwork stays with it, and needs a story without
  errors.

Each change is applied to the draft through `StoryDraft::write` (or
`StoryDraft::rename_scene`), refused if
the story moved on since it was read, and answered with the compiler's report
on the whole story: "The story compiles.", or up to twelve errors with which
of them were there before the request, and the story's scene names. Refusals
come back as tool errors with their reason. The model is told to fix the
errors it caused before it finishes; errors that predate the request are the
writer's.

Settings › Models › Assistant shapes the agent; the choices are kept in the
`[assistant]` section of Studio's settings. **Thinking** sets how much the
model reasons before answering — Model default, Off, Low, Medium or High.
A provider that cannot change it fails the run rather than ignoring it.
**Limit reply length** caps each answer's tokens, reasoning included
(`max_output_tokens`); off, the provider's own limit applies. **Context** is
the context a managed text model is started with, saved on its provider in
`clipforge.toml`. **Steps per request** (4–200, default 30) is how many times
the model is called; on the last call it is offered no tools, so it ends by
saying what it did and what is left, and the outcome is marked out of steps.

The same agent serves live MCP's assistant tools, under Project scope; its
edits wait in Studio's review like the column's.

## Atelier

The existing captured queue request is an application step binding in
`creative_workflow.rs`. Two non-repeatable steps execute image iteration and
store its result. Provider execution remains in Agentkit images (or Atelier's
existing ComfyUI adapter). Protected-pixel compositing, provenance, thumbnail
creation and atomic PNG/reference writes stay in Atelier. The durable final
output names the saved reference JSON, with image bytes kept out of the event
stream. Queue progress, per-request Abort, pause, retry and result attachment
continue through the existing worker events. No request is resubmitted by the
workflow integration. `.atelier` save/reopen remains the document's normal path.

## Validation

Core tests cover streamed writing output, automatic invalid-output repair,
bounded repair exhaustion without publication, cancellation between attempts,
invalid-draft preflight without a model call, reopening persisted state/context and
events, stale revision rejection, cancellation of a stalled step, provider
failure, exact ranges and mismatched source/project/file rejection. Studio tests
cover one-edit acceptance/undo and rejection of concurrent or invalid source.
Atelier's local ComfyUI protocol fixture executes both real workflow steps,
checks the stored result and reopens the attached image from an `.atelier`
document. Its pre-cancelled workflow makes no network submission or output.
These are offline contracts, not model-quality or live desktop acceptance.

Isolated macOS desktop acceptance (2026-10-31) used the Dioxus Studio build and a
local streaming HTTP fixture, with no real provider submissions. It verified
the beat scope and instructions, original/proposal review, acceptance and a
single Undo, running cancellation with retained instructions, a source edit
during a delayed reply (proposal retained and Accept disabled), rejection and
invalid proposed DSL diagnostics (Accept disabled). The pass caught and fixed
the desktop query lifetime: a snapshot channel stays alive until Rust consumes
it, then receives an explicit acknowledgement. Evidence/logs live under
`.svs-e2e/workflows/`. Model quality, assistive technology and other platforms
remain unverified.

The review redesign was checked in an isolated macOS release build on
2026-11-01: a long suggestion scrolls with its actions visible, Original switches
the passage, acceptance is one toolbar Undo, existing warnings are omitted from
review, and an HTTP failure produces a single recovery notice. The neutral
technical-details dialog and startup configuration recovery were also inspected
using disposable fixtures. No real provider submissions were made.
The validity-gate pass also verified automatic repair after an invalid first
response and a single recovery notice after three invalid responses, with no
invalid proposal or acceptance action presented.

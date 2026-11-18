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
restores its width. A project has one conversation, kept in the story's
`workflows.sqlite3` beside `story.db` and reloaded when the story opens. New
conversation, in the column's header, clears it; it is unavailable while a
reply runs or a suggestion is under review. A turn is a request and its reply; a reply lists the steps the
workflow took and ends in one outcome: a suggestion, an answer in prose, no
change, stopped, failed, or interrupted (Studio closed while it ran).

While a reply runs, its steps stay open: Starting the text model (with the
model and its address, or "Loading … on this computer" for a managed one),
Writing, Checking the edit, and Fixing the edit (1 of 2) when a check fails.
The running step's time counts up each second, and its line says what is
arriving — "Thinking · 1,419 characters" while a reasoning model reasons,
"Writing · 201 characters" once the answer starts — with the answer's
readable part shown as a draft underneath. A finished step keeps a record of
what it found: "Wrote 201 characters after 3,198 of thinking", "The story
still compiles", or the first of its problems. When the reply ends, the steps
fold to "Worked for 12.7s · 3 steps" — "Ran for 12.7s" when it failed — which
opens them again.

A request names its reach with the scope chip beside Send, which opens a
menu of Selection, the Beat or Scene around the caret, the whole File, or the
Project — every story file. The passage, the instructions and Keep
facts (on by default) are captured when the request is sent; editing
afterwards does not change it. The caption under the request names the
passage, and opens it in the editor while no review is open. An empty file,
or Selection with nothing selected, is refused under the prompt, and the
prompt keeps its text. Without a
text provider in Settings, the prompt says so and Send is disabled.

A request that is not an edit — a greeting, a question about the story — is
answered in prose, and nothing is checked or changed. A suggestion arrives
with what the model said about it, if anything, and a card naming the file
and the lines it adds and removes. Review replaces the
editor with a read-only diff of the file; each change is kept or reset, and
Keep all / Discard all decide the rest. The story changes once, when the
review ends, as one undoable edit of the kept changes. Reviewing is refused if
the file changed since the suggestion was written. Asking again while a review
is open refines it: the kept changes are applied, and the same passage is
asked for again with both instructions. The card records what was decided —
Kept, Kept 2 of 3, Discarded.

A Project request runs the project workflow below. Its steps are Planning
(with the files it will change), Writing each of those files — "Writing
chapter.sf (part 2 of 3)" for a long one — and Checking the story. While it
runs, the reply lists each file it has reached with how it stands — Writing
part 2 of 3, Fixing, Compiles, or its new problems — and the lines added and
removed so far. A file's name shows it being written in the editor's place: a
diff with no Accept or Reject that fills in as each part arrives, and changes
nothing; Back to the file returns to the editor, and the run ending closes it.
Its card
lists every changed file with its line counts. Review opens the first file's
diff; deciding a file's last change moves on to the next file with changes
left, and the review bar steps between files. Nothing is written until the
review ends, when every file's kept changes become one undoable edit. A
review is refused if any of its files changed since the suggestion was
written, whether or not that file is open. A file the suggestion creates is reviewed
like the others, against nothing; keeping it creates the file empty in the
story's folder and puts its text in as an unsaved change, saved with the rest.

A Project request that stops, or fails after writing something, still ends in
a card: what was written is the suggestion, the card says why it is
unfinished and that it may not compile, and the review bar says the same. Its
changes are kept or discarded like any other's, with errors if they have
them. A request that fails before writing anything fails as before.

A failed reply says so once, in a card under its steps: the code's label
("Could not create a writing suggestion"), Try again, which sends the same
words about what the scope names now, and Details, which opens the shared
details dialog (DESIGN.md §2). Expanding the steps marks the one that
failed. A reply interrupted by Studio closing offers Try again too. A file
name on a suggestion's card opens that file while no review is open.

### The writing workflow

`storyfella_studio_core::workflows::writing` runs a request as an Agentkit
workflow with a language-model step, through the text-provider binding from
Settings. A managed (`llamacpp`) provider is reached through the worker the
shared inference service allocates — the address in `clipforge.toml` is only
the provider's identity — and its lease is held for the whole request, repair
attempts included; a hosted provider is called at its configured address.

Settings › Models › Assistant shapes both workflows; the choices are kept in
the `[assistant]` section of Studio's settings. **Thinking** sets how much the
model reasons before answering — Model default, Off, Low, Medium or High —
which is the `thinking` field of the `language_model` step. A provider that
cannot change it fails the run rather than ignoring it. **Limit reply length**
caps each answer's tokens, reasoning included (`max_output_tokens`); off, the
provider's own limit applies. **Context** is the context a managed text model
is started with, shared by its two request slots, and is saved on its provider
in `clipforge.toml`; a provider whose server Studio does not start keeps its
own. **Repair attempts** (0–5, default 2) is how many times a rejected answer
is sent back, and **Whole story up to** (4–256 KiB, default 24) is where a
Project request stops sending every file's text.

The request carries the story once: the passage, and — for anything smaller
than the whole file — the complete source, for surrounding structure and
cross-scene links. Alongside it go the descriptions of the assets the
passage's scene refers to. File scope captures the complete source, including
comments and boundary whitespace, as the passage. Responses regain captured
indentation, line endings and terminal blank lines that a provider may have
trimmed; a partial-line selection is not reindented. The model answers in one of two forms: the replacement in a fenced block,
with an optional sentence about it, or prose with no block when the request
is not an edit. `writing::interpret` reads either, and also an unfenced
replacement — taken as source when most of its lines are statements, or when
the passage is part of one line — and drops `<think>` reasoning, including a
closing tag whose opening tag the server stripped. A prose answer to the
first attempt becomes the reply and skips the repairs; prose during a repair
counts as an empty replacement. A whole-file answer to a smaller passage is
cut back to the passage when the text around it is unchanged. An answer cut
off at the provider's output limit is never checked: it is asked for once more
with thinking off, which is usually what used the room up, and the rest of the
request keeps thinking off. Cut off again, the run fails as cut off, with
advice to ask about less or give the model more room rather than to check the
provider. The project workflow does the same for each of its calls.
A draft that already has errors is worked on like any other: the model is
given them as `existing_problems`, so asking to fix them works, and an edit is
held only to adding none — errors are matched by code and message, as many
times as the draft has each. The check's line says "The story compiles now",
"Fixes 1 of 2 problems" or "No new problems". Before review, the
replacement is inserted into the captured document and compiled. Empty output,
or output that adds errors, triggers up to the configured repair attempts using the
original request, the rejected replacement and compiler diagnostics; repair runs link to their
preceding attempt in the run store. Only a valid result becomes a suggestion,
so compilation guarantees structural validity, not narrative quality or
instruction fidelity. Stop cancels drafting and repair; a provider may
continue remote compute after local cancellation.

Each request stores its exact source, byte range, project and source paths. A
suggestion applies only to that same source.

### The project workflow

`storyfella_studio_core::workflows::novel` runs a request about the whole
story. It plans first: the model reads the instructions against every file's
text when the story is within the whole-story size, or each file's path, scenes and size
when it is larger, and answers either in prose — the reply, after one call —
or with a JSON plan and a note on what it will do. The plan names the files
to rewrite, any new files, and any whole scenes to move. Studio moves the
scenes itself, word for word, before anything is written: the model never
retypes a scene to move it, so splitting a file is one short answer. A new
file must sit beside an existing one; its `include` takes the place of the
first scene moved into it, so the story plays in the same order from the same
first scene, and a new file nothing moves into is included at the end of the
entry for the model to write. A plan that names a file the story does not
have without creating it, moves a scene that does not exist, or creates a file
nothing goes into is sent back, up to the configured repair attempts, with
what the story has instead — its scene names, and that a beat is not a scene.
A plan that still does not fit fails with its own advice: say which scenes or
files are meant, or ask about less.
Each planned file is rewritten in fragments of at most 2 KiB, split between
lines, each answered as one fenced block; an answer in prose is sent back.
The changed files are then compiled together with the rest of the project.
As in the writing workflow, a story that already has errors is worked on: the
plan and each file's fragments are given them, and the result is held to
adding none. A file with new errors is sent back whole, with its errors, up to
the configured repair attempts; a file too long to send back whole fails the run with the errors listed by file and
line. Every model call after the first is a run linked to the first in the
run store. Each file is reported as it goes: first as it was, then with each
finished part in place, then compiled together with the files written before
it. A run that is stopped or fails after writing something returns what it
wrote as a proposal marked unfinished, with the reason — a file that was
being written has its finished parts and the rest as it was. The same
workflow serves live MCP's novel tools; applying a proposal there still
requires it to add no problems.

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

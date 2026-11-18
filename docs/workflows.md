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

Write docks the assistant on its right. A project has one conversation, kept
in the story's `workflows.sqlite3` beside `story.db` and reloaded when the
story opens. A turn is a request and its reply; a reply lists the steps the
workflow took, with their times, and ends in one outcome: a suggestion, no
change, stopped, failed, or interrupted (Studio closed while it ran).

A request names its passage with the scope chips: Selection, the Beat or Scene
around the caret, or the whole File. The passage, the instructions and Keep
facts (on by default) are captured when the request is sent; editing
afterwards does not change it. An empty file, or Selection with nothing
selected, is refused under the prompt, and the prompt keeps its text. Without a
text provider in Settings, the prompt says so and Send is disabled.

A suggestion arrives as a card with its line counts. Review replaces the
editor with a read-only diff of the file; each change is kept or reset, and
Keep all / Discard all decide the rest. The story changes once, when the
review ends, as one undoable edit of the kept changes. Reviewing is refused if
the file changed since the suggestion was written. Asking again while a review
is open refines it: the kept changes are applied, and the same passage is
asked for again with both instructions. The card records what was decided —
Kept, Kept 2 of 3, Discarded.

A failed reply shows the code's label ("Could not create a writing
suggestion") with Details, which opens the shared details dialog (DESIGN.md
§2).

### The writing workflow

`storyfella_studio_core::workflows::writing` runs a request as an Agentkit
workflow with a language-model step, through the text-provider binding from
Settings. A managed (`llamacpp`) provider is reached through the worker the
shared inference service allocates — the address in `clipforge.toml` is only
the provider's identity — and its lease is held for the whole request, repair
attempts included; a hosted provider is called at its configured address.

The request includes the containing scene and descriptions of its resolved
assets; the complete source supplies validation context for surrounding
structure and cross-scene links. File scope captures the complete source,
including comments and boundary whitespace. Responses regain captured
indentation, line endings and terminal blank lines that a provider may have
trimmed; a partial-line selection is not reindented. Before review, the
replacement is inserted into the captured document and compiled. Invalid or
empty output triggers up to two repair attempts using the original request,
the rejected replacement and compiler diagnostics; repair runs link to their
preceding attempt in the run store. Only a valid result becomes a suggestion,
so compilation guarantees structural validity, not narrative quality or
instruction fidelity. Stop cancels drafting and repair; a provider may
continue remote compute after local cancellation.

Each request stores its exact source, byte range, project and source paths. A
suggestion applies only to that same source.

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

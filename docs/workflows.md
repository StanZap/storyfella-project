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

Studio's GPUI Write view edits the story but has no Writing help panel yet;
what follows is the writing workflow and its contracts, which the plan driver,
MCP channel and `sf-cli` still reach.

**Writing help** opens beside the editable source. Choose a selection, the
current beat, current scene or Entire file; choose Rewrite or Expand and enter custom
instructions. Preserve facts is the default; creative changes are explicit.
The request includes the containing scene and descriptions of its resolved
assets. It uses the current text-provider binding from Settings. A managed
(`llamacpp`) provider is reached through the worker the shared inference
service allocates — the address in `clipforge.toml` is only the provider's
identity — and its lease is held for the whole request, repair attempts
included; a hosted provider is called at its configured address.
Entire file captures the complete source, including every scene, comments and
boundary whitespace, without depending on the caret. Retry keeps this scope.

Agentkit streams provisional replacement text into the panel. Before model
submission, the captured draft must compile without source errors; pending
artwork warnings are allowed. Responses first regain captured indentation,
line endings and terminal blank lines that a provider may have trimmed. A
partial-line selection is not reindented. Before review, Studio inserts the
replacement into the complete captured document and runs the shared compiler.
Genuinely invalid or empty output triggers up to two automatic repair attempts using the original
request, rejected replacement and compiler diagnostics. Each attempt uses the
public runner and durable storage; repair runs link to their preceding attempt.
The prompt supplies the DSL rules, complete source and small boundary previews
as validation context. Repair feedback identifies offending source lines and
line/column positions; the original request remains structured JSON. Progress
shows the repair attempt and fixed limit.
Only a valid result is published as a proposal. Exhausted repair produces a
recoverable failure, retaining the draft, request and any prior valid proposal.
Cancellation applies throughout drafting and repair. Compilation guarantees
structural validity, not narrative quality or instruction fidelity.

On completion, the request collapses and the story area becomes a continuous
diff editor. Inline shows deleted lines and editable proposed text together;
Side by side aligns a read-only draft with the editable suggestion. Changing
views preserves text, selection and editor undo history. CodeMirror owns diff
layout and native editing; shared Rust analysis supplies reference-aware syntax
styles. Rust remains the authority for application and saving.

The gutter's reset arrow restores an individual change to the accepted draft;
its checkbox selects that change for group actions. With checked changes, the
request panel offers Keep selected and Reset selected; otherwise it offers
Keep change and Reset change for the current difference. A group is resolved
against one snapshot and kept as one undoable edit. Editing the proposed text
clears checkbox selection so shifted ranges cannot target another change.
The panel also owns change navigation, Try again,
Keep all and Discard all. Navigation buttons and F7/Shift+F7 stop at the
first/last change. Actions have
named icons and tooltips; acceptance and recovery retain visible labels.
Edit request and Try again sit together in the header. Retry uses the current
prompt and request settings; the panel shows the user's prompt without a
separate Rewrite label, repeated count heading or generic editing instructions.
Unavailable actions are disabled, including accepting an invalid selected
change/group or discarding an empty review. Edits anywhere in the proposed version remain pending
and excluded from project saving and Preview. Keep runs through AppState and
source reconciliation as one undoable edit; Keep all applies all remaining
changes in one edit. Discard restores the accepted text for the selected range,
preserving previously kept changes. Compiler errors block acceptance, including
partial edits that depend on another change. Newly introduced notes are grouped;
existing warnings stay in Problems. Retry captures the current accepted draft;
cancelled or failed retries retain the previous editable comparison.
Manual edits and explicit reset choices are preserved across retries using a
three-way source merge. Non-overlapping model changes combine with them; an
overlap or compiler-invalid combination keeps the current review and reports
the conflict; ambiguous adjacent edits are also treated conservatively. Merge
markers never enter the source. Protection survives repeated
retries and reopening, even when an intermediate model reproduces the user's
text. Saved checkpoints from before this protection are upgraded transactionally
using their original proposal as the pre-edit baseline. The Rust `diffy` crate
provides text merging; CodeMirror continues to own editor interaction and layout.

Writing workspaces autosave locally as they change. Reopening the same project
restores the request form, pending review (including manual edits), comparison
mode, checked changes, current difference and unsaved accepted draft. These checkpoints never write
pending suggestions to the `.sf` file or feed them to Preview. Closing during
generation retains the request and received partial text; reopening marks it
interrupted and offers an explicit Try again, without sending a new request.
If the story differs from both the checkpoint's disk snapshot and accepted
draft, recovery preserves both versions and offers Restore writing session
(an unsaved, undoable draft) or Discard saved session. Storage failures remain
visible in Writing help. Editor undo history is scoped to the running app;
recovery restores document text rather than the previous process's undo stack.

Each request stores its exact source, byte range, project and source paths.
Application requires the same complete source snapshot and owning paths; newer
typing or another project never receives a delayed replacement. Selection
changes after dispatch do not change the request. Errors appear once beside the
story with retry, Settings and the shared technical report; request/instructions
are retained. Cancel interrupts the pending step and discards its response
without changing source. A provider may continue
remote compute after local cancellation.

Creative context includes the containing scene and resolved asset descriptions;
the complete source supplies validation context for surrounding structure and
cross-scene links.
Project-wide writing instructions, conversational history and domain tool
calling remain follow-up work.

The desktop editor bridge exchanges UTF-16 positions and monotonic edit
sequences. Rust resolves positions against the exact source, preserving CRLF
and UTF-8 boundaries. Every acceptance action captures current editor text
before dispatch; older host renders cannot overwrite newer typing. CodeMirror
does not write files or contact model providers.

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

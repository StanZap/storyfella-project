# Application workflows

Studio can open with a hosted image default or a text-only provider registry.
Writer assistance needs only the selected text provider. Native image setup is
optional; without a selected native t2i deployment, Studio skips image prewarm
and hides native setup controls. Native-only requests report SVS-CFG-110 before
starting a server or provisioning weights. Shared hosted defaults remain
available to Atelier and are not rewritten.

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
saves. They record executions, not a promise to automatically repeat model calls
after a crash. Automatic restart/recovery UI and reactive multi-turn sessions
remain tracked in SF-PLAN-28.

## Writer

**Writing help** opens beside the editable source. Choose a selection, the
current beat or the current scene; choose Rewrite or Expand and enter custom
instructions. Preserve facts is the default; creative changes are explicit.
The request includes the containing scene and descriptions of its resolved
assets. It uses the current text-provider binding from Settings, including the
existing managed-server readiness path.

Agentkit streams provisional replacement text into the panel. Before model
submission, the captured draft must compile without source errors; pending
artwork warnings are allowed. Before review, Studio inserts the replacement
into the complete captured document and runs the shared compiler. Invalid or
empty output triggers up to two automatic repair attempts using the original
request, rejected replacement and compiler diagnostics. Each attempt uses the
public runner and durable storage; repair runs link to their preceding attempt.
The prompt supplies the DSL rules and complete source as validation context.
Only a valid result is published as a proposal. Exhausted repair produces a
recoverable failure, retaining the draft, request and any prior valid proposal.
Cancellation applies throughout drafting and repair. Compilation guarantees
structural validity, not narrative quality or instruction fidelity.

On completion, the request collapses and the suggestion fills the reading area,
with an Original toggle
and fixed Accept suggestion/Discard actions. Edit request restores the compact
composer. Compiler errors block acceptance; new notes are grouped, while warnings
already present in the captured draft stay in Problems. Accept runs through
AppState and the existing source reconciliation as one undoable edit. Discard
leaves source and registry untouched. Another version reruns the captured
request; submitting the edited composer captures the current scope and
instructions. A failed attempt retains the previous proposal.

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
Project-wide writing instructions, conversational history, domain tool calling,
and automatic recovery of proposals into the panel remain follow-up work.

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

Isolated macOS desktop acceptance (2026-10-31) used the styled dx build and a
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

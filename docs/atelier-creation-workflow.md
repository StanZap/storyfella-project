# Atelier creation workflow · 2026-10-24

## Audit and implementation boundary

The starting checkout was clean at `9c77953`. The two latest commits already
provided Projects, a horizontal Artboards view, multiple drawing canvases, a
shared composer frame and creation briefs. This pass reused those foundations,
the document/history model, immutable image originals, provider adapters,
generation queue, reference library, mask drafts and `.sfart` exchange.

The audit found these differences between the visible vocabulary and storage:

- `Document.layers` held the active canvas and `Document.canvases` held the other
  canvases, but the Artboards screen also displayed every project reference as
  artwork. Generated images went into `Document.references`.
- Drawing selection requests cropped the source before generation. Selected-area
  results attached immediately as image layers; full-canvas and inspector edits
  became additional reference-backed artboard cards.
- The drawing composer had a separate, unpersisted textarea. Other prompts were
  already durable and source-bound. Queue entries already captured settings and
  a project epoch, but selection placement did not identify the destination canvas.
- Layer visibility/order/opacity, image movement, scale buttons, crop/scale canvas
  resizing, B/E/H, painting undo and export were working and were extended.
- The library already owned immutable shared originals and explicit promotion.
  It also offered a project-only view and a generation rail. Credentials were
  edited in Atelier and written through Clipforge's shared registry.
- Clipforge owns hosted route capabilities and the native image server. Its
  desktop package is `clipforge-desktop`, macOS identity `com.clipforge.desktop`.
  No provider-setup deep link or setup-completion callback was found.

No sibling repository was modified.

## Saved roles

`.atelier` schema version 2 keeps one canonical active-artboard representation
(`width`, `height`, `layers`, `canvas_id`) and stores other artboards in
`canvases`. An artboard's reference associations and reference visibility travel
with it. The project embeds referenced originals, accepted artwork, pending
candidates, image parent identities, prompts and drafts.

New compositions create artboards containing image layers. Full-artboard edits
and selection blends create layers marked `candidate`. Candidates can be
previewed and transformed, accepted, hidden/shown, discarded, or saved as
references. Candidate and hidden-layer changes are undoable. Saving a candidate
as a reference also promotes its original to the shared library. Export snapshots
exclude candidates even when their on-screen preview is visible; references
and assisted tracing do not enter PNG/JPG or canvas `.sfart` exports.

Artboard `.sfart` exports preserve the prompts and provenance of accepted
visible image layers and omit pending/hidden layer metadata as well as pixels.

`Reference` remains the immutable image payload type reused inside image layers;
its owning reference association or layer defines its role. Parent links resolve
across project references, artboard image layers and the shared library.

Loading version 1 upgrades generated references with saved generation prompts
into image artboards, removing the redundant reference-role records while keeping
their bytes and metadata in layers. Existing image layers and imported references
keep their roles. Up to the existing 25-artboard limit are promoted; remaining
originals remain available as references. Resaving writes version 2. Old files are
not rewritten merely by reading them. Fingerprints are restored in inactive
artboards as well as in active layers.

## Creation and review

**Create / image prompt** or **⌘P** opens the same floating composer everywhere.

| Context | Input and result |
| --- | --- |
| Artboards | A new composition becomes a new artboard. Selected references use the editing route as image input; the destination remains a new artboard. |
| Open drawing artboard | The accepted artwork is captured at full resolution. The result is a candidate on that artboard. |
| Area → Blend into artboard | Send the complete accepted artboard for surrounding context. Normalize the returned raster to source coordinates and crop the selected rectangle. Attach only that rectangle as a candidate at the captured location and size. Pixels outside it remain original. |
| Area → Create separate image | Send the selected crop as input; expose output dimensions. The result becomes a separate artboard. |
| Inspected library/result image | Create a variation as a new artboard. **Add to canvas** explicitly promotes the image to an artwork layer. |

Blend is an instruction edit with local output cropping, not a claim of native
masked inpainting or guaranteed seamless boundaries. The composer identifies
the selected rectangle and full source dimensions before submission. Provider
artistic fidelity and seam quality still require actual-provider acceptance.

Prompt text is stored by project and context: New image, inspected source image,
or artboard plus full/selection coordinates and blend/separate mode. Last-used
artboard selection contexts survive reopening. Navigation and errors preserve
drafts; drawing undo retains current prompt text. Request settings, references,
destination artboard identity and placement are captured when queued. Opening a
different artboard or project cannot redirect the result. Attachment failures
retain an inspectable queue result and the original generated files.

The reference picker combines shared-library and project originals with search.
Import adds files to the library and selects the last imported image without
leaving the picker flow. Selected artboard references are portable project data.
The drawing dock lets users select between them and hide/show the reference
surface. The Library itself is app-wide and has no project-scoped tab or
generation side panel.

Route limits come from `HostedDeployment`: Qwen Image 3 fal and Qwen Edit 2511
fal/Replicate allow a source plus two additional references; other current routes
allow fewer. Local models allow one source without extras or mask; Krea and
ComfyUI create only. Model capabilities, reference counts, masks, credential
availability, native configuration and Replicate output aspect constraints are
checked before queueing. Backend input-size and account restrictions still apply.
Exact dimensions and creation briefs are disclosed under **Options**; selection
separate-image dimensions are shown directly. LoRA request choices remain in
image Settings and are captured by the queue.

Layers retain painting/erasing, ordering, visibility and opacity. **Move layer**
shows a bounding outline and corner handle for moving and uniform scaling;
stroke marks move with image content. Numeric move offsets, relative scale and
opacity support precise adjustment. H remains canvas pan. Canvas/project settings
retain the existing bounds-crop versus scale-artwork resize behavior.

## Projects and Clipforge

Open projects occupy separate tabs (up to 32), each retaining its document,
undo/redo history, viewport, dirty state and prompt drafts while running. Opening
an already-open path activates its tab. New/Open still save the previous project
through the existing switch worker. Recovery snapshots and a session manifest
restore open documents, paths and dirty flags after restart; undo history and
viewport are session-only. Queue history/running jobs are not resumed on restart.
Tab closing/reordering is not yet implemented.

**Open Clipforge** launches the desktop app using its macOS bundle identity or
`clipforge-desktop`/`clipforge-desktop.exe` on PATH elsewhere. Failures are visible.
Provider connections, credentials and secrets are managed there; Atelier retains
request model presets and ComfyUI workflow mappings. **Refresh readiness** rereads
the shared endpoint/credentials and probes local service state. Configured means
configuration is present, not that a remote account has been successfully charged
or that model files have been verified. Nothing launches a paid request during a
readiness refresh.

The missing cross-app contract is a registered setup URI or equivalent command
that opens Clipforge's provider settings, plus a completion/config-change signal.
Until Clipforge supplies that contract, users navigate its settings and explicitly
refresh after returning. Linux/Windows need the desktop executable on PATH;
installed-app discovery and those platform handoffs remain acceptance work.

## Verification and remaining acceptance

Regression tests cover schema upgrade/idempotence and preserved lineage,
captured-artboard attachment, atomic attachment failure, selection crop geometry,
candidate exclusion from PNG export, layer acceptance/transform/history and
project round-trip, two-project recovery with independent drafts/dirty flags,
and unsupported provider/reference combinations before execution. Existing
painting, mask, `.sfart` and Studio import tests remain active.

Final checks: `cargo check --workspace --all-targets`, `cargo test --workspace`,
`cargo fmt --all -- --check`, strict Atelier Clippy, `git diff --check`, native
macOS packaging and the packaging regression script pass. Atelier has 57 passing
tests and 12 ignored device/fixture tests; no Metal device acceptance run was
performed. Existing vendored GPUI deprecation warnings remain.

The isolated macOS build uses a unique test bundle identity. Live checks exercised
project creation, project-tab switching and quit/restart with draft restoration,
the drawing composer, a local stroke and the transform gizmo. An earlier app-name binding
resolved to an old registered bundle; none of that UI state counts as evidence
for this implementation.

Prepare local UI fixtures with
`cargo test -p storyfella-atelier prepare_creation_workflow_fixture -- --ignored`.
The blue artwork/red candidate project and green reference PNG stay under
`.svs-e2e/atelier-creation-fixture`. The native Open picker was reached during this
pass, but path navigation did not complete reliably under automation; that does
not establish an application file-loading failure.

Still required: first-time-user acceptance of the complete character loop,
real-provider full/selection/separate requests and error recovery, generated
candidate accept/hide/discard through the native UI, imported-reference picker
acceptance, multi-tab failure injection through native controls,
keyboard-only/VoiceOver testing and Linux/Windows interaction. Successful mock
responses and correct placement metadata establish no artistic quality or
identity consistency. See `DESIGN.md` and roadmap §12 for current check results.

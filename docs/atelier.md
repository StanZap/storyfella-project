# Atelier and the shared AI boundary

Atelier is `apps/atelier`, package `storyfella-atelier`, binary `atelier`.
It remains a separate GPUI app in the Storyfella workspace. Studio and Player
remain Dioxus; GUI-free artwork exchange lives in `storyfella-studio-core`.

## Run and data

```sh
cargo xtask atelier
cargo xtask atelier -- --data-dir .svs-e2e/atelier-dev
sh apps/atelier/package-macos.sh
```

Atelier is explicitly separate from Studio's `cargo xtask run` and the `svs`
CLI. The macOS bundle is `dist/atelier/Atelier.app`; its identifier remains
`com.storyfella.atelier`, and data stays
in `~/Library/Application Support/Atelier` (macOS) or `~/.local/share/Atelier`.
The root Cargo patch and vendored GPUI Metal renderer remain in this workspace.
FineWineTTS retains its separate GPUI renderer patch for its review application.
The macOS development bundle uses ad hoc signing without certificate setup.
Provider keys resolve from environment variables first, then the matching hosted
provider entry in the shared `clipforge.toml`. Provider connections, credentials
and secrets are managed in Clipforge. Keys stay out of projects and exports.

## Image workflow

A project contains drawing/image artboards; each artboard owns artwork layers,
reference associations and pending AI candidates. **Artboards → Create** creates
a new composition. Open an artboard to draw or edit; **Create / image prompt**
(**⌘P**) opens the same floating composer with full-artboard, selection **Blend
into artboard**, and selection **Create separate image** contexts. Local edits
are candidates to review, accept, hide, discard or save as references. Separate
images become new artboards. Accepted artwork remains intact until review.

The app-wide **Library** holds explicitly shared originals. The composer/drawing
reference picker searches shared and project images, supports importing into the
library, and embeds selected references in the project. References can be hidden
while working and are excluded from artwork exports. New compositions may use
selected references as image input without changing their new-artboard destination.

Multiple projects stay open in tabs with independent documents, dirty state,
history and drafts. Project/recovery saves retain artboards, candidates, reference
associations, provenance, source/mode-bound drafts and last selection contexts.
Schema version 1 upgrades on load without discarding originals.

Provider setup and credentials belong in **Clipforge**. Use **Open Clipforge**,
then return and **Refresh readiness**. Atelier retains per-request model presets,
size, steps, adapters and ComfyUI workflow mappings. Readiness distinguishes
configuration from a successful remote request.

See [creation workflow](atelier-creation-workflow.md) for the audit, exact
migration, input limits, queue/selection semantics, project recovery, the missing
Clipforge setup deep-link contract and remaining native/provider acceptance.

## Local Qwen and Krea on Linux with CUDA

Clipforge's shared image client serves **Qwen Image 2.1** generation and editing,
and **Krea 2 Turbo** generation through native `stable-diffusion.cpp`. Both are
available in Atelier's Image provider picker without an API key. Native edits
accept one source image; masks and extra references are rejected before startup.

Copy [the shared config example](../config/clipforge.example.toml) to the
platform config directory — Linux `~/.config/clipforge/clipforge.toml` — and
replace `/home/you` with your actual paths. Atelier never asks for a file path:
it reads the machine's shared `clipforge.toml`, so configure exactly one matching
architecture for each requested capability. Relative model, weight and
executable paths resolve against that file's directory.

From the Clipforge repository, provision the CUDA server and both model profiles:

```sh
cargo xtask sd-server --model-dir /home/you/.local/share/clipforge/models
cargo run -p clipforge -- models --provider qwen ensure
cargo run -p clipforge -- models --provider krea ensure
```

In Atelier select a local provider. Generate starts the machine's shared server
on demand — started detached and owned through its marker, so another front end
(Studio, the Clipforge apps) reuses the same process instead of starting a second
one. Qwen generation and subsequent edits reuse the resident model; switching
deployments releases the previously owned server before loading the next model.
The app does not download weights or compile the server on submission. Missing
setup reports an error while keeping the source and prompt draft.

The new-image generation panel carries the prompt, the aspect presets (**1:1 / 3:2 / 2:3**)
and **Add to queue**, and reports whether the machine's shared `sd-server` is
answering (`Local server: running · <model>` or `stopped · starts on demand`),
re-probed on demand, when the provider changes, and after each generation. A gear
in the rail opens the **Image
settings** window with the sampling **steps** (blank uses the model's default —
25 for Qwen, 8 for Krea), the output **width**/**height**, and the **LoRA
adapters** added from a searchable picker (each applied adapter carries its own
weight, default 1.0), all as per-request controls. **Settings** keeps request
model presets and ComfyUI workflow mappings; Clipforge manages connections and credentials. The sampling options ride the shared
image request's native fields (`clipforge_inference::images::Request`); hosted
providers ignore them and use the preset aspect sizes.

**Generation is queued.** **Add to queue** captures the request — provider,
model, prompt, width, height, steps and the applied LoRA adapters with their
weights — and runs it after the requests already waiting, so several images can
be prepared before any finishes. Exactly one request runs at a time, on the same
shared server. **Pause** holds dispatch (a running image always finishes),
**Stop** cancels the running request, a queued request can be removed, and
**Clear finished** drops completed history. Each queue entry keeps the values it
was created with, so editing the live controls afterwards never changes work
already queued. The queue has its own panel, opened from the header badge (which
counts outstanding requests); it lists each request with
its state (Queued, Running with its progress, Done, Failed, Cancelled) and
carries Pause/Stop/Clear.

Configuration and shared loopback tests pass on this Mac; actual Linux CUDA
inference and Linux native interaction still require verification on that machine.

**Add to canvas** creates an image-backed layer. Draw and erase on that layer,
change opacity/visibility/order, move it by dragging with **Move layer**, or scale
it with **Smaller/Larger**. Painted marks move with the image. Placement, deletion
and painting are undoable. Images participate in full-resolution CPU/Metal
exports, project saves and artwork rotation/flip/resize.

**Paint mask**, **Erase mask** and **Box selection** create a source-resolution
draft with undo/redo. **Box selection** fills a dragged rectangle, so an edit
touches only that region. Drafts save with their image and reload when it is
reopened. **Use mask** sends
the draft to a supported editing model. The shared API uses white for editable
pixels and black for protected pixels; Atelier composites protected source pixels
back into a successful result. A masked result must retain source dimensions.
Failures preserve the draft and expose **Retry mask save**. Window close waits
for the latest draft and recovery save; errors keep the app open.

Use the composer's **References** picker to select up to two additional inputs.
The selected source is Image 1; additional references are Images 2 and 3.
New compositions use the first selected reference as their primary image input.
Unsupported combinations fail before queueing. Full-artboard edits and selection
blends arrive as pending layers, while separate-image requests create artboards.
See [creation workflow](atelier-creation-workflow.md) for context and placement.

| Editing route | Ordered images | Mask |
| --- | --- | --- |
| fal Qwen Image 3 edit | up to 3 | no |
| fal Qwen Edit 2511 | up to 3 | no |
| Replicate Qwen Edit 2511 | up to 3 | no |
| fal / Replicate FLUX Fill | 1 | yes |
| Other current adapters | 1 | no |

Provider constraints live in Clipforge. fal Qwen Image 3 inputs are made opaque
on white and must fit its 384–2048 px constraints. Hosted inputs are bounded to
10 MB; originals may be larger, so choose a suitable route/input rather than
expecting silent downscaling. Arbitrary provider-specific sampling parameters
and transparent-output requests are not exposed.

## Approved artwork in Studio

Export an inspected version with **Export for Studio**, or export the painted
canvas from Canvas settings with **Export canvas for Studio**. The portable
`.sfart` package contains full-resolution artwork, name, prompt and provenance.
Studio's Canvas gallery offers **Import Atelier artwork**, an image preview,
editable name and Character/Environment/Object role selection. Import creates
a completed canonical revision through `AppState`, supports undo/redo and normal
project persistence, and never calls an inference provider. The review and
write worker stay bound to the project selected when import began.

The two apps still have different project models. This exchange is an explicit
approval handoff, not a live synchronization layer. Studio's generation runtime
still uses its native `sd` deployment adapter; hosted-provider selection and an
Atelier text/vision assistant are separate future work.

## Execution and limits

`clipforge_inference::images::Client::generate(Request)` owns managed/native and
hosted image execution, route capabilities, masks/reference wire formats, polling,
cancellation and bounded downloads. The GPUI worker owns its Tokio runtime;
Atelier owns files, document history and ComfyUI. Hosted provider keys are saved
to the shared `clipforge.toml` by Clipforge's atomic, owner-only config writer.
Hosted services never start/stop native servers. No paid request is automatically
resubmitted; errors preserve a known job ID. API keys never go to image CDNs.

Original limits: 20 MB encoded file, 8192 px per edge, 32 megapixels. Project
limits: 24 layers, 128 references, 64 MB combined embedded image data. The
current library is loaded on a worker with a 512 MB encoded in-memory budget;
it is a local collection, without cloud synchronization or pagination.

## Verification

See [iteration acceptance](atelier-iteration-acceptance.md) for the real-provider
bookshop walkthrough, saved proof project, passing tasks and remaining UX gaps.

Deterministic fixtures can be prepared without paid APIs:
`cargo test -p storyfella-atelier prepare_workflow_fixture -- --ignored`.
They stay in `.svs-e2e/atelier-workflow`.

Tests cover raw originals, project/library round trips, branch families, mask
draft recovery/corruption, protected pixel compositing, image layers and placement,
provider rejection, credentials, autosaves and ComfyUI mock execution. Physical
Metal tests verify image layer color/alpha/erasure/transforms and GPUI texture
sampling. Studio tests cover approved-package validation, asset storage,
completed revisions, undo/redo and project reload. Clipforge loopback tests
verify ordered references and exact mask pixels on both hosted wire formats,
without real keys or paid calls.

Live macOS checks cover library actions, full-size inspection, fitted comparison,
zoom/pan, mask paint/erase/undo, field typing, image layer paint/erase, artwork
export, and Studio's role picker/import/undo/redo. Linux runtime behavior and paid
provider output quality have not been retested here.

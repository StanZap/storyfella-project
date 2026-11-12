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

Atelier also opens a [shared Storyfella project](shared-project.md) through its
`storyfella.toml` manifest. Its editable document lives at
`atelier/main.atelier`; ordinary Save and reopening use that project-local path.
Studio retains ownership of `story.sf`. Moving the complete project folder
preserves the visual document and its embedded images. Artboards → Assets reads
and edits the shared catalog, creates definitions and explicit variations without
rendering, attaches saved images to existing UUIDs and links working artboards.
The document retains those links across saves and folder moves. Rename of an
item used in source belongs in Studio. Live synchronization and ordinary
canvas-save image publication remain SF-PLAN-04–06.

A project contains drawing/image artboards; each artboard owns artwork layers,
reference associations and pending AI candidates. **Artboards → Create** creates
a new composition. Open an artboard to draw or edit; **Create / image prompt**
(**⌘P**) opens the same floating composer with full-artboard, selection **Blend
into artboard**, and selection **Create separate image** contexts. Local edits
are candidates to review, accept, hide, discard or save as references. Separate
images become new artboards. Accepted artwork remains intact until review.

### Drawing and painting

Open **Brushes** to choose graphite, ink, charcoal, marker, soft brush or dry
brush. The brush color picker supports opacity and exact hex values. **Sample**
picks a color from accepted canvas artwork. Recent colors are available in the
picker, and **Save swatch** keeps a color in the editable project's palette.
Brush width, stroke opacity, smoothing and pen pressure can be adjusted while
working.

Open **Shapes** and drag to place a rectangle, rounded rectangle, ellipse, line,
triangle or pentagon on the paint target. Hold Shift to constrain proportions
or snap a line to 45°. Click a shape to edit its fill, outline and outline
width, rotate or resize it, or delete it. Option/Alt-drag moves one shape.
Several shapes and freehand strokes can share one layer, in the order drawn.
The normal selection tool moves or transforms the whole layer.

With a closed shape selected, **Clip new paint layer** adds a paint target above
it. The clip follows changes to the shape's position and geometry. Hiding the
source shape's layer leaves the clip in effect. Removing the source shape hides
the clipped artwork until you undo the removal or choose **Clear clip** in the
paint layer's properties. Shapes, color, clipping and the palette survive
saving and reopening. Linked artboards publish to Studio through ordinary Save.

**Generate from canvas** opens the composer. Use **Preview source** to see the
artwork captured for Image 1. **Edit artboard** returns a candidate layer, and
**New artboard variation** uses the full composition as input for a new
artboard. The source preview reflects Atelier's image before any provider
format conversion.

### Canvas interaction

In the drawing workspace, **Select** (V) selects the topmost visible, unlocked
layer under the pointer, including visible image pixels and nearby paint strokes.
Shift-click adds or removes a layer; Command/Control-click cycles overlapping
layers. Drag from empty canvas to select intersecting layers, or Shift-drag to
add them. Option/Alt-drag starts a marquee over existing artwork. Clicking
empty canvas clears the selection. Drag a selection to move it, its edge or
corner handles to resize it, or its outer handle to rotate it. Shift temporarily
changes the aspect lock during resize and snaps rotation to 15°; Option/Alt
resizes from the center. Escape cancels a drag. Arrow keys nudge the selection
by one pixel, or ten with Shift. Pan remains available with Hand (H) or middle
drag. Layer properties include precise transforms, alignment, distribution,
exact spacing and separate actions for positioning the selection or aligning
each item to the artboard. Open **Arrange…** in layer properties to show the
arrangement controls; options for multiple items appear when they are useful.
Position and size appear when the selected layers contain artwork. Edit a value
and press Enter or move focus to apply it; Escape cancels the edit. Numeric
positions are exact even when snapping is enabled for canvas dragging. With
snapping enabled, guides show alignments to the artboard and other visible layers.
In Layers, Shift-click selects a range and Command/Control-click toggles rows.
Duplicate, delete, stacking, flipping and zoom-to-selection actions are also
available from the context menu.

The paint and eraser target is shown in Layers and in the status bar, separately
from the transform selection. Select a layer and choose **Set as paint target**
in its properties to change it. Undo restores the previous target.
When no target is set, drawing uses the selected paint layer or creates one above
the selected image. Choosing an image as the paint target explicitly permits
painting and erasing on that image-backed layer.

Select adjacent, ungrouped layers in Layers and choose **Group** to keep them
together without changing stacking order. A group appears as an expandable row;
select its row or click its artwork to move, resize, align, distribute, reorder,
or delete it as one unit. Select a child in Layers or double-click its artwork
to edit the group's children; **Done** or Escape returns to the group. Group
names are editable in Layer properties. Drag rows to reorder adjacent selections;
the group controls can add an adjacent layer or remove an edge child without
changing stacking. **Ungroup** releases all children while
preserving their order. Group membership, names, visibility, locking and opacity
are saved with the project. Hiding or locking a group preserves each child's
own settings; group opacity applies to the composited group. To ungroup while
preserving its appearance, first show and unlock the group and return its
opacity to 100%. Nested groups are not yet available.
From image inspection, **Add to canvas** creates and selects an image layer.
The Layers panel can import image files directly as artwork; dropping files onto
the canvas or pasting an image with Command/Control-V also creates image layers.
Each imported image is placed on the artboard captured when the import began.
The current selection and paint target stay in place.
The drawing composer also offers **New image layer**; generated results arrive
as candidates for review on the artboard captured when the request was queued.
If that artboard was undone, Atelier reports an attachment failure and keeps
the generated result in the queue for recovery.
With one unlocked, accepted image layer selected, **Replace selected · Fit** fits
the inspected image within the old image's frame, preserving its orientation.
**Replace selected · Fill** fills that frame using a reversible crop. Replacement
is undoable; pending candidates retain their separate review flow.
For one selected image layer, open Layer properties and choose **Crop image**.
Drag the area to keep on the canvas, then choose **Apply** or **Cancel**. The
shaded area is outside the proposed image crop; other layers are unaffected.
**Reset** restores the full original image when applied. Crops preserve the
embedded original, participate in undo, and are saved with the project.

The app-wide **Library** holds explicitly shared originals. The composer/drawing
reference picker searches shared and project images, supports importing into the
library, and embeds selected references in the project. References can be hidden
while working and are excluded from artwork exports. New compositions may use
selected references as image input without changing their new-artboard destination.

Click a project card anywhere on the Projects home page to open it. Cards also
support keyboard focus and activation without a separate Open project button.

Multiple projects stay open in tabs with independent documents, dirty state,
history and drafts. Tabs share one header row with the project actions; each tab
groups its title, edited dot and close icon on one surface. The close icon changes
color on hover without a separate background. Click **Atelier** to return to the
Projects home page, which also opens **Library**. Drag to reorder, or use
Ctrl+Shift+Page Up/Down. The close
icon or Cmd/Ctrl+W saves before closing; a failed save keeps the tab open. Tab
actions also live in File. Closing the final tab returns to Projects, and Library
remains accessible without project controls. Tab order and active identity
survive restart. Project/recovery saves retain artboards, candidates, reference
associations, provenance, source/mode-bound drafts and drawing targets.
Atelier writes one set of recovery snapshots in `workspace-recovery/`. Files
from earlier development builds remain in their original locations.

The composer's ellipsis opens a menu with **Image options** and **Provider
settings**. Provider setup and credentials belong in **Clipforge**, reached from
Settings through **Manage providers in Clipforge**. Return to Settings and use
**Reload providers** after changes. Atelier retains per-request model presets,
size, steps, adapters and ComfyUI workflow mappings. The composer has no provider
setup row or routine setup subtext; blocked submissions show validation beside
the prompt.

See the document and selection behavior below for stored roles and request placement.

## Repeatable live native acceptance

The opt-in `studio-core` integration tests exercise the public Agentkit image
and lifecycle APIs used by the applications. Copy
[`native-acceptance.example.toml`](../config/native-acceptance.example.toml) to
`.svs-e2e/native-acceptance/providers.toml`, set existing executable/model paths
and any weight overrides, and start the configured `resident-qwen` server.
The `acceptance-qwen` and `acceptance-krea` providers must share a separate,
initially unused loopback port. Keep `lora_dir` inside the acceptance directory.
Run the scenarios sequentially to avoid concurrent GPU loads:

```sh
cargo test -p storyfella-studio-core --test native_acceptance native_generation_reuse_abort_and_saved_output -- --ignored --nocapture
cargo test -p storyfella-studio-core --test native_acceptance native_model_switching_and_reuse -- --ignored --nocapture
```

The first scenario reuses a resident server, generates at 512×512 with 20 steps
and seed 42, saves and decodes the PNG from disk, aborts a submitted two-step
request, measures local cancellation latency below one second, drains remaining
server work and verifies another request succeeds. Active GPU sampling can
continue after the local abort; this test does not claim remote interruption.
The second scenario loads Qwen → Krea → Qwen through shared lifecycle controls,
checks that loading an already resident model starts no new process, and saves
and reopens each result. It uses two Qwen steps and eight Krea steps as lifecycle
smoke checks, not image-quality acceptance. It stops its owned server on success
or a returned error and leaves the separately running resident server alone.
Outputs and switching-server logs remain under `.svs-e2e/native-acceptance/`.
After the first scenario, exercise Atelier's actual result attachment and project
storage with both generated PNGs:

```sh
cargo test -p storyfella-atelier native_acceptance_outputs_survive_atelier_project_reopen -- --ignored --nocapture
```

This writes `native-results.atelier`, reloads it and verifies both artboards retain
their embedded images, dimensions and provenance. These tests establish
provider/lifecycle and project-storage behavior; live Atelier queue interaction
and visual quality need their own acceptance.

## Native image capability probes

The opt-in RGBA probe uses the resident Qwen deployment in the native acceptance
configuration above and the public low-level Agentkit `native::sd::Client`.
It generates a transparent cup, then edits that PNG as an RGBA reference at
512×512, 20 steps and seed 42. Each output must encode alpha, contain more than
10% nearly transparent pixels and more than 1% nearly opaque pixels, and survive
a byte-identical disk round trip. Pixel counts and job timings are printed.
The second probe consumes those two outputs and adds a horizontally flipped
reference through Agentkit's shared image client with both alpha requirement
flags enabled. It uses eight steps as an input/transport smoke check. Run the
probes sequentially; the second depends on the first. The separate Atelier check
verifies result attachment and project save/reopen preserve all three encoded
PNGs, their dimensions and provenance.

```sh
cargo test -p storyfella-studio-core --test image_capabilities_acceptance native_qwen_rgba_generation_and_reference_editing -- --ignored --nocapture
cargo test -p storyfella-studio-core --test image_capabilities_acceptance native_qwen_three_ordered_rgba_inputs -- --ignored --nocapture
cargo test -p storyfella-atelier native_rgba_acceptance_outputs_survive_atelier_project_reopen -- --ignored --nocapture
```

Evidence stays under `.svs-e2e/image-capabilities/`, including
`qwen-alpha-create.png`, `qwen-alpha-edit.png`, `qwen-three-references.png` and
`qwen-alpha-results.atelier`.
The probe verifies alpha transport, usable transparency and persistence. Subject
identity and edit fidelity require separate visual evaluation. Application
preflight uses Agentkit's profile capabilities: Qwen accepts one source plus two
ordered references, retains PNG alpha input and supports transparent output.
When transparent output is required, an opaque result fails with the submitted
job ID; callers supply the transparency prompt. Krea remains generation-only.
Qwen also supports optional latent masking, verified separately below.
The pinned server documents RGBA and ordered references in
[its Qwen guide](https://github.com/leejet/stable-diffusion.cpp/blob/3f8527a46c54ecf4cb4ed6003da8e8982283c73c/docs/qwen_image_2.1.md)
and [wire contract](https://github.com/leejet/stable-diffusion.cpp/blob/3f8527a46c54ecf4cb4ed6003da8e8982283c73c/examples/server/api.md).
Its generic mask feature flag does not establish model-specific editing behavior
or exact preservation of protected pixels.

## Native mask acceptance

Qwen's native route advertises `MaskSupport::OptionalLatent`. White mask pixels
are editable; black pixels anchor the source's denoising latents. The adapter
sends the source as both `init_image` and reference 1, retains additional
references in order, sends the mask unchanged and sets `strength = 1` to keep the
requested sampling schedule. Source, mask and requested output dimensions must
match. Invalid image data and geometry fail before model startup. Krea remains
generation-only and rejects masked editing.

Latent protection passes through VAE reconstruction, so unmasked pixels can
change slightly. The existing Use mask tooltip explains this. Exact protection
requires application compositing, as used by Atelier's selection blends; model
masking and compositing are separate mechanisms.

The opt-in controls reuse `native-create.png` from the native acceptance above.
They submit the same source, prompt, two-step sampling and seed 42 with all-black,
all-white and left-half-white masks. The shared-client scenario then combines
the half mask with the two RGBA reference PNGs from the capability probes.
Run sequentially:

```sh
cargo test -p storyfella-studio-core --test native_mask_acceptance native_qwen_mask_controls_and_protected_pixel_drift -- --ignored --nocapture
cargo test -p storyfella-studio-core --test native_mask_acceptance native_qwen_shared_masked_edit_with_ordered_rgba_references -- --ignored --nocapture
cargo test -p storyfella-atelier native_mask_acceptance_outputs_survive_atelier_project_reopen -- --ignored --nocapture
```

The tests measure RGB mean absolute error on a 0–255 scale. A white mask must
change the result, and the half mask must change the editable region much more
than the far-protected region, compared with the all-black reconstruction. The
comparison deliberately excludes the boundary when checking protected latents.
Direct source comparisons separately measure reconstruction drift. The Atelier
check saves and reopens all four outputs with embedded bytes and provenance.
Inputs, masks, outputs, metrics and logs stay in
`.svs-e2e/native-mask-acceptance/`. These are mechanism and persistence checks;
two-step runs do not establish artistic editing quality.

## Repeatable live hosted acceptance

The opt-in Replicate scenario uses the existing shared `clipforge.toml` resolved
by `SVS_AI_CONFIG` or the normal platform path. It does not modify configuration
or print credentials. The provider named `replicate` must declare `t2i` and
`edit`, with its model and credential configured; `SF_ACCEPTANCE_PROVIDER` can
select another Replicate entry. This scenario submits four real, potentially
billed requests and spaces them by twelve seconds for limited-rate accounts.

```sh
cargo test -p storyfella-studio-core --test hosted_acceptance -- --ignored --nocapture
cargo test -p storyfella-atelier hosted_acceptance_outputs_survive_atelier_project_reopen -- --ignored --nocapture
```

The first request aborts after submitted-job progress and requires the remote
cancellation endpoint to accept the request within seven seconds. The second
uses a positive one-millisecond polling deadline and requires a timeout carrying
the submitted job ID and accepted remote cancellation. Submission has its own
network timeout; the elapsed deadline check includes that time. An accepted
cancellation response establishes acknowledgement, not the exact time remote
compute stops. Recovery generation and reference editing must then complete;
both images are saved and decoded from disk. Replicate advertises aspect-ratio
sizing: the square request need not return exactly 512×512. The Atelier scenario
attaches them through the real result path and verifies embedded bytes,
dimensions and provenance after saving and reopening `hosted-results.atelier`.
Evidence stays under `.svs-e2e/hosted-acceptance/`, including `RESULTS.txt` on a
successful provider run. This verifies lifecycle, transport and persistence;
it does not establish image fidelity or other unconfigured hosted providers.

## Local Qwen and Krea on Linux with CUDA

Agentkit's shared image client serves **Qwen Image 2.1** generation and editing,
and **Krea 2 Turbo** generation through native `stable-diffusion.cpp`. Both are
available in Atelier's Image provider picker without an API key. Qwen edits accept
one source plus two ordered references, preserve input alpha and support optional
latent masks. Inputs beyond the verified limit and invalid masked-image geometry
are rejected before startup. The RGBA and
multi-reference live evidence above is from macOS Metal; clean Linux/CUDA
packaging and acceptance remain SF-PLAN-30.

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
with an unavailable status when provider configuration cannot be resolved. It is
re-probed on demand, when the provider changes, and after each generation.
Clipforge publishes its effective native server address and absolute model and
executable paths at desktop startup; hosted defaults and credentials are kept. A gear
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
shared server. **Pause** holds dispatch (a running image always finishes).
Each queued or running row has **Abort**, which targets that request alone.
Queued requests become **Aborted** without being submitted; running requests
show **Aborting** until their worker acknowledges cancellation and their result
is discarded. Other requests and the pause state are preserved. The native
server cannot interrupt GPU sampling already in progress, so its computation
may finish after Atelier stops waiting. Hosted cancellation depends on the
provider and charges may still apply.

Open app-wide Settings through **Atelier → Settings…** or **⌘,** (Ctrl+, on
other platforms), including from Projects. **Generation → Request time limit
(minutes)** saves a global default of 30 minutes, configurable from 1 minute to
24 hours. New requests capture this limit; changing it does not affect requests
already queued or running. Waiting in Atelier's queue does not count. Native
server startup has its own readiness limit, and a native timeout stops waiting
without guaranteeing remote cancellation.

**Clear completed** drops completed or dismissed history, preserving unresolved
failures. Each queue entry keeps the values it
was created with, so editing the live controls afterwards never changes work
already queued. The app-wide **Activity** control exposes failures and queue work,
including Pause/Abort/retry/result-opening actions. Failures have a concise
explanation, recovery action and optional **Details** with a selectable read-only
report and verified **Copy report**. Short contextual hints replace raw footer
errors. Generation hints stay with their project and prompt context; save
failures retain a Not saved hint. All side panels use Activity's full-height,
360px-wide docked frame, with a shared title/close header and scrolling body.
Settings retains a pinned Save footer. In drawing, auxiliary panels replace
Layers/References and restore the inspector preference when closed. Escape
closes a panel from its controls.
Reports/queue history last for this app session; project and draft recovery are
unchanged. Retries preserve captured inputs and resolve their report on success;
there is no automatic retry or guessed provider cooldown.

Configuration and shared loopback tests pass on this Mac; actual Linux CUDA
inference and Linux native interaction still require verification on that machine.

**Add to canvas** creates an image-backed layer ready to position. Select a
paint target in Layers; explicitly choosing an image-backed layer makes paint
and eraser strokes part of that layer. Placement, deletion, painting, cropping
and replacement are undoable. Images participate in full-resolution CPU/Metal
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
See the canvas request contexts below for placement.

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

## Document storage and image roles

`.atelier` schema version 4 stores the active artboard in the document and
other artboards in `canvases`. Each artboard requires a `drawing_target` that
selects the full canvas, a new layer, or a rectangular region in blend or
separate mode. Reference associations and visibility travel with each artboard.
The document embeds referenced originals, accepted artwork, pending candidates,
image parent identities, prompts and drafts. Image crops and group properties use
this same schema.

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

The version 1 artwork migration upgrades generated references with saved generation prompts
into image artboards, removing the redundant reference-role records while keeping
their bytes and metadata in layers. Existing image layers and imported references
keep their roles. Up to the existing 25-artboard limit are promoted; remaining
originals remain available as references. The loader restores image fingerprints
in both active and inactive artboards. Converting v2 and v3 drawings is a
separate data task.


## Canvas request contexts

| Context | Input and result |
| --- | --- |
| Artboards | A new composition becomes a new artboard. Selected references use the editing route as image input; the destination remains a new artboard. |
| Open drawing artboard | The accepted artwork is captured at full resolution. The result is a candidate on that artboard. |
| New artboard variation | Use the complete accepted artwork as input. The result opens on a separate artboard. |
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


## Verification

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

### MOSS Image SFT

Atelier offers **MOSS Image SFT · local** through Agentkit's `moss-image-sft`
profile. It supports generation only, using 50 Euler steps, CFG 3 and the
discrete scheduler by default. Editing, masks and alpha output are not advertised.
The profile provisions pinned Q8_0 diffusion, Qwen3.5-9B Q4_K_M text weights,
FLUX.2 VAE and an external tokenizer. Clipforge owns the native runtime build;
its pin is `c6dfa9def12fd0cb36ac7149ba30a75792bedbf0`. An older installed runtime
must be replaced with `cargo xtask sd-server` from Clipforge before loading MOSS.

The downloaded profile and external tokenizer successfully loaded on an Apple
M3 Max through Clipforge's updated Metal server. This verifies startup only;
SF-PLAN-24 still requires generation timing and switching with saved artwork.

For isolated acceptance, copy
[`moss-acceptance.example.toml`](../config/moss-acceptance.example.toml) to
`.svs-e2e/moss-integration/providers.toml`. Set the existing executable and model
tree there; the default executable assumes the isolated MOSS backend build in
`target/moss-native-backend`. Keep the acceptance port at 17864, separate from
the resident user server. Provisioning resumes partial files and verifies
checksums; it uses the configured model tree.

```sh
cargo test -p storyfella-studio-core --test moss_acceptance provision_moss_model -- --ignored --nocapture
cargo test -p storyfella-studio-core --test moss_acceptance moss_generation_and_server_reuse -- --ignored --nocapture
```

The generation probe records startup and 512×512/50-step generation timings in
`.svs-e2e/moss-integration/RESULTS.txt`, saves and reopens `moss-create.png`, and
cleans up its owned server on errors or panic. Neither probe changes a user
server or marks the roadmap acceptance complete.

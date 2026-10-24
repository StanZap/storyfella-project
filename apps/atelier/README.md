# Atelier

A GPUI sketchbook for drawing practice and art concepts in the Storyfella
workspace. Draw from a reference, reflect on a study, and keep your progress.
Atelier is a separate desktop app from Dioxus Studio; see
[workspace boundaries and planned integration](../../docs/atelier.md).

## Run

```sh
cargo xtask atelier
```

On macOS, create a native development bundle with:

```sh
sh apps/atelier/package-macos.sh
open dist/atelier/Atelier.app
```

The bundle is written to `dist/atelier/Atelier.app`. It is a local development build,
not an Apple-signed or notarized distribution. App data defaults to
`~/Library/Application Support/Atelier` on macOS, or `~/.local/share/Atelier`
on other desktop platforms. For isolated development sessions:

```sh
cargo xtask atelier -- --data-dir .svs-e2e/atelier-dev
```

The macOS development bundle uses ad hoc signing and needs no certificate setup.

## Available

- Graphite pencil, Ink, Charcoal and Marker; layer-local eraser, pressure, size and opacity controls, optional stabilization.
- Native Metal brush rasterization and layer compositing on macOS, with full-resolution 512 px canvas tiles.
- Up to 24 layers with visibility, opacity, reordering, deletion and naming in canvas settings.
- Undo/redo for document changes; 64 history checkpoints.
- Square, portrait, landscape, A4 and custom canvas sizes. Resize bounds to crop at the top-left, or scale artwork with the canvas.
- Canvas pan, zoom, fit, 90° view rotation, mirrored view and optional proportion guides. Separate undoable artwork rotation and flipping.
- Atomic `.atelier` project saves with embedded references, recovery autosave and shutdown saves.
- PNG export with optional transparency; JPG export with a warm paper background and adjustable quality. Exports use document coordinates and exclude practice overlays.
- Reference imports (PNG/JPG/WebP), favorites, editable names/tags, search and independent reference zoom/pan.
- Optional reference grid, grayscale values and a clearly labeled assisted tracing overlay.
- 30-second, 2-, 5- and 10-minute studies, pause/reset, practice prompts and project reflection notes.
- Study gallery for the latest 48 archived sessions. New/Open archives the current study before switching; archive/open failures preserve the current document.
- fal.ai, Replicate, Alibaba Model Studio and GPT Image generation/editing, plus local ComfyUI generation, with background jobs, errors and cancellation.
- A generation queue with its own panel, reached from a header badge that counts outstanding requests: pause, stop the running request, remove queued requests and clear finished history. Each request keeps its own prompt, size, steps and LoRA adapters.
- Shared icon controls, surface panels and selection styles, accessible icon labels, gentle panel reveal and a reduced-motion setting.

The **Library** is the app-wide shared reference collection. The floating
composers and drawing dock open a picker that searches shared and project
references, imports files into the collection, and embeds selected originals
in the project. Artboards own their reference associations and visibility.
References enter artwork only through explicit layer placement.

Projects can stay open in separate tabs with their own document, dirty state,
history and prompt drafts. Version-2 saves distinguish artboard layers, pending
AI candidates and references; existing projects upgrade on load. See the
[audit and creation workflow](../../docs/atelier-creation-workflow.md) for exact
migration/recovery behavior and remaining acceptance work.

## Generation setup

Use **Create / image prompt** (**⌘P**) in the workspace. Artboards creates a new
composition; Draw edits the full artboard or a selected area. **Blend into
artboard** uses surrounding artwork as context and returns a placed candidate.
**Create separate image** uses the crop and user-selected output size for a new
artboard. Candidates can be accepted, hidden/shown, discarded or saved as
references; exports exclude unaccepted candidates. The same composer serves
image inspection; no generation side panel is required.

Provider connections, credentials and secrets belong in Clipforge. **Open
Clipforge** launches its desktop app; return and **Refresh readiness** to reread
shared setup. Atelier retains request model presets, sampling, LoRA choices and
ComfyUI workflow mappings. Environment credentials still take precedence over
the matching shared provider entry.

| Provider | Credential | Default generation / editing models |
| --- | --- | --- |
| fal.ai | `FAL_KEY` | `alibaba/qwen-image-3/text-to-image` / `alibaba/qwen-image-3/edit` |
| Replicate | `REPLICATE_API_TOKEN` | `alibaba/qwen-image-3` for both |
| Alibaba Model Studio | `DASHSCOPE_API_KEY` | `qwen-image-3.0` for both |
| GPT Image | `OPENAI_API_KEY` | `gpt-image-2.5-flare` for both, medium quality |

Alibaba also offers presets for `qwen-image-3.0-pro` and `qwen-image-2.1-pro`.
Enter your workspace's **OpenAI-compatible base URL**, for example
`https://WORKSPACE_ID.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1`.
Keys, endpoints and available models must belong to the same region. This adapter
uses `/images/generations` with an `image` field for editing, as documented by
[Alibaba](https://www.alibabacloud.com/help/en/model-studio/qwen-image-generation-and-editing-api-reference).

fal presets also include Qwen Image 2512 / Edit 2511, Z-Image Turbo and FLUX Schnell
(generation only). Replicate includes Qwen Image 2512 / Image Edit. Replicate
model IDs use `owner/model` and must support its official-model predictions API.
Custom model IDs must accept the same inputs as their adapter's presets; this is
not a universal mapper for arbitrary model schemas.

**Create** captures the prompt, requested size, source/reference images, sampling
and destination in the generation queue. The header queue badge exposes
Pause/Stop/remove/retry/history controls. One request runs at a time.
Unsent drafts remain bound to their project, artboard/source and selection mode.
Full-artboard edits and blends preserve accepted originals and arrive as
candidate layers; separate compositions become artboards. Failed requests retain
the source and draft. Every result retains parent, prompt and provider provenance.

Layers support visibility, ordering, opacity, painting and undo/redo. **Move
layer** displays an outline and a corner scale handle; numeric offsets, relative
scale and opacity support exact adjustments. **Start variation as new artboard**
creates a separate composition from an accepted image layer.

fal's Qwen Image 3 accepts only opaque editing inputs, so that adapter composites
transparent references onto white before upload. Other adapters retain source
alpha. Qwen Image 3 (fal) and Qwen Edit 2511 (fal/Replicate) support a source plus
two ordered references. FLUX Fill (fal/Replicate) supports a painted mask; other
current routes accept one source and no mask. Unsupported combinations fail
before submission. Mask drafts are durable; protected source pixels are composited
back into the edited result. Explicit transparent-output controls are not exposed.
Qwen prompt expansion is disabled for the default Qwen Image 3 adapters so an edit
instruction is sent without provider-added detail.

API references: [fal generation](https://fal.ai/models/alibaba/qwen-image-3/text-to-image/api),
[fal editing](https://fal.ai/models/alibaba/qwen-image-3/edit/api),
[Replicate](https://replicate.com/alibaba/qwen-image-3),
[OpenAI editing](https://developers.openai.com/api/reference/resources/images/methods/edit).

Approved originals/versions and painted canvases export as `.sfart` packages.
In Studio's Canvas gallery, **Import Atelier artwork** previews the package and
adds a completed Character/Environment/Object revision, with undo and project
persistence. See [workflow and boundaries](../../docs/atelier.md) for the full flow.

For local ComfyUI, configure its HTTP(S) address and load a workflow exported in
**API format**. Set the positive text node ID, the width/height node ID and the
sampler seed node ID. Defaults are `6`, `5` and `3`, matching the
[official basic API example](https://github.com/Comfy-Org/ComfyUI/blob/master/script_examples/basic_api_example.py).
Leave size or seed mappings blank to retain workflow values. Include an image
output node. The adapter submits `/prompt`, polls `/history/{id}`, and downloads
from `/view`; custom workflows must have the nodes and models installed on the
server. Comfy Cloud authentication is not implemented.

Cancellation stops result attachment. fal and Replicate poll job status and send
cancellation to only the submitted job; a running job may still finish remotely.
ComfyUI deletes only this job if it is queued. OpenAI and Alibaba use synchronous
requests that cannot be stopped remotely by this adapter once submitted. Charges
may apply after cancellation. Jobs time out after five minutes; synchronous requests
may take that long to return. Local mock tests cover all hosted providers' creation
and editing, polling, cancellation, credential isolation and legacy settings.

## Shortcuts

Click the canvas to focus drawing shortcuts. Text fields retain normal editing.

| Key | Action |
| --- | --- |
| B / E / H | Pencil / eraser / pan |
| [ / ] | Brush size |
| R / F | Rotate view / mirror view |
| 0 / Tab | Fit/reset view / toggle dock |
| ⌘Z / ⇧⌘Z | Undo / redo |
| ⌘S / ⇧⌘S / ⌘O | Save / save as / open |
| Scroll / ⌘Scroll / pinch | Pan / zoom / zoom |
| ⌘Q | Save recovery and quit |

Control is also accepted for canvas save/undo/open shortcuts.

## Implementation and validation

`document.rs` owns projects, shared stroke snapshots, bounded history, validation, atomic writes and CPU raster utilities. `viewport.rs` provides inverse transforms. `queue.rs` owns the generation queue — request snapshots, ordering, pause and states. `worker.rs` separates storage,
raster work, session previews and generation from UI interaction. `providers.rs`
implements ComfyUI and provider profiles; `image_execution.rs`
adapts shared image generation/editing, and `native_images.rs` retains the managed
Qwen/Krea client across iterations. See [Linux CUDA setup](../../docs/atelier.md#local-qwen-and-krea-on-linux-with-cuda).
`ui.rs` holds shared visual
primitives and the drawing/library/settings views; `app.rs` coordinates state.

Run `cargo test -p storyfella-atelier`. Tests cover transform round trips, layer
compositing and erasing, straight alpha, PNG/JPG exports, embedded project recovery,
save ordering, failed project switching and a local mock ComfyUI round trip.
The mock server requires loopback socket access.

Graphite pencil is the default, with deterministic paper grain and pigment buildup
between strokes. Ink, Charcoal and Marker are available from the brush name in the
bottom toolbar. The Brushes panel offers pressure on/off and Light, Natural and
Firm response curves. Pen pressure controls width and pigment; the eraser uses
pressure for width. Mouse input retains constant width and opacity. Shared curve interpolation rounds
gradual bends between sparse input points while preserving turns of 60° or more
and reversals. It interpolates pressure, limits curve handles to prevent loops,
and leaves already-painted intervals unchanged as input arrives. The same curves
are used for live painting, exports and thumbnails; saved input points are retained.
The Smooth control adds optional input stabilization on top of interpolation.

On macOS, `tablet.rs` reads normalized AppKit pressure from tablet mouse events
and monitors standalone tablet-point events, bridging the information GPUI omits.
Pressure samples and brush type are saved with each stroke and used in previews
and exports. Older projects retain their solid strokes. Physical tablet hardware
has not been exercised; the driver must provide AppKit tablet events. Windows and
Linux currently use constant-pressure pointer input. Tilt, arbitrary-angle
rotation, freeform stroke selections and a full lesson curriculum remain extensions.


## GPU canvas

On macOS, `gpu_canvas.rs` and `brushes.metal` run brush coverage, graphite grain,
pressure, layer-local erasing, layer opacity/compositing and assisted tracing on
Metal. Painted layer tiles persist on the GPU. Appending points sends only new
segments, binned into affected tiles; each brush dispatch covers a bounded dirty
rectangle in batches of at most 64 segments. A no-op update does no brush work.
Undo, canvas transforms, layer reordering and other edits to existing strokes
invalidate and rebuild the tile cache. Layer visibility/opacity changes reuse
painted tiles and recomposite.

`canvas_renderer.rs` delivers full-resolution 512×512 tiles, including clipped
edge tiles. GPUI draws visible tiles at the current pan/zoom. The former 1400 px
preview cap is removed. macOS PNG/JPG export uses the same Metal renderer, with
straight-alpha transparent PNG and paper-backed JPG; tracing overlays are excluded.
Completed stroke sample arrays are shared between history, UI and worker snapshots
using copy-on-write, preserving the existing project JSON format.

`gpu_surface.rs` publishes Metal-compatible, IOSurface-backed BGRA pixel buffers
directly to GPUI's surface API. The pinned Apple renderer patch in
`vendor/gpui-pre-apple` samples those shared textures without CPU tile readback or
image-atlas uploads. Surfaces become immutable after Metal completes; GPUI retains
their buffers and texture mappings through presentation command completion.
Quarter-turns and mirroring also run on the GPU. CPU pixel reads remain for
PNG/JPG encoding and verification. Gallery thumbnails and brush swatches use CPU
raster utilities. Windows/Linux retain a CPU canvas fallback; their native GPU
backends have not been implemented.

Limits remain 8192 px per edge, 32 megapixels, 24 layers and two million stored
points. GPU layer textures have a 768 MiB safety budget (12 bytes per occupied
layer pixel: painted color, pre-stroke color and coverage). Dense multilayer art
can reach that budget; it is not a promise of unlimited canvas/layer capacity.
Reference import preserves the original image; separate thumbnails serve the
library and project reference panels. Legacy projects keep the resolution their
older versions stored.

Hardware verification on an Apple M3 Max (2026-10-04), with a synthetic 4096×4096
painting, two layers, 800 strokes and 25,600 points: full rebuild about 98 ms with curve interpolation;
appending one input interval in one tile about 1 ms; GPU layer textures 384 MiB.
The previous readback/upload path took about 355–400 ms and 6 ms respectively.
These are render-worker timings, excluding GPUI presentation. They are not
end-to-end frame-rate guarantees.

```sh
cargo test -p storyfella-atelier
cargo test -p storyfella-atelier metal_ -- --ignored --nocapture --test-threads=1
cargo clippy -p storyfella-atelier --all-targets -- -D warnings
```

The ignored tests require a real Metal device. They check incremental rendering
against a fresh rebuild, tile boundaries, pressure/grain pixels, layer erasing,
visibility/opacity, undo, document/view transforms, transparent export and tracing
exclusion. Shared-surface tests verify GPUI color/clipping, immutable published
tiles, GPU view orientation and zero live-render readback. The dense-scene test prints device and timing information. Metal tests
must run outside a sandbox that hides the GPU. Native UI smoke checks exercise
painting across tiles, erasing, undo/redo, zoom and view rotation in an isolated app.

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
Provider keys use `api-keys.json` in Atelier's user app data directory on every
platform. Environment keys take precedence; blank input preserves saved keys.
Keys stay out of projects and exports. Existing Keychain entries are not read
or migrated; enter the key once in Settings.

## Image workflow

**Library → All projects** is the reusable image collection. Importing there
stores an immutable original without attaching it to the current study.
**This project** shows embedded references; **Use** adds a library image to the
project. Saved projects retain their own copies and do not depend on library
paths. Search matches names/tags; favorites filter the collection. Open an image
to edit its global name/tags. Project reference metadata remains project-local.

Imported and generated originals retain their full dimensions and original
bytes. Display thumbnails are separate. Old projects that already contain a
1600 px normalized reference retain those pixels; importing the original file
again is necessary to recover detail that was previously discarded.

**Open** enters the large image workspace: fit, 100%, scroll zoom, pan,
full-resolution PNG export, versions and a labeled before/after comparison.
Editing an earlier version creates a branch; the source is kept. Parent identity,
prompt and provider/model/job provenance are stored alongside library originals.
Browsing another image while a job runs does not redirect that job or force the
completed version over the image being inspected.

Unsent prompts belong to their source image; New image has a separate draft.
Navigation restores the relevant draft. Project and recovery saves retain the
active source and drafts, including on quit. Canvas undo does not undo prompt
typing. The prompt-reuse icon loads the inspected version's original prompt.
Version chips use saved names; their tooltips identify the parent branch.
Save/Open/Save as shortcuts work while typing a prompt as well as on the canvas;
native text fields keep their own editing and undo behavior.

## Local Qwen and Krea on Linux with CUDA

Clipforge's shared image client serves **Qwen Image 2.1** generation and editing,
and **Krea 2 Turbo** generation through native `stable-diffusion.cpp`. Both are
available in Atelier's Image provider picker without an API key. Native edits
accept one source image; masks and extra references are rejected before startup.

Copy [the providers example](../config/atelier-native.example.toml) to your user
app data directory, for example `/home/you/.local/share/Atelier/providers.toml`,
and replace `/home/you` with your actual paths. An existing Clipforge providers
file also works: configure exactly one matching architecture for each requested
capability. Relative model, weight and executable paths resolve against that
file's directory.

From the Clipforge repository, provision the CUDA server and both model profiles:

```sh
cargo xtask sd-server --model-dir /home/you/.local/share/Atelier/models
cargo run -p clipforge -- --config /home/you/.local/share/Atelier/providers.toml models --provider qwen ensure
cargo run -p clipforge -- --config /home/you/.local/share/Atelier/providers.toml models --provider krea ensure
```

In Atelier select a local provider, open Provider settings, enter the absolute
**Local providers file** path and Save settings. Generate starts the managed
server on demand. Qwen generation and subsequent edits reuse the resident model;
switching deployments releases the previously owned server before loading the
next model. The app does not download weights or compile the server on submission.
Missing setup reports an error while keeping the source and prompt draft.

Configuration and shared loopback tests pass on this Mac; actual Linux CUDA
inference and Linux native interaction still require verification on that machine.

**Add to canvas** creates an image-backed layer. Draw and erase on that layer,
change opacity/visibility/order, move it by dragging with **Move layer**, or scale
it with **Smaller/Larger**. Painted marks move with the image. Placement, deletion
and painting are undoable. Images participate in full-resolution CPU/Metal
exports, project saves and artwork rotation/flip/resize.

**Paint mask / Erase mask** create a source-resolution draft with undo/redo.
Drafts save with their image and reload when it is reopened. **Use mask** sends
the draft to a supported editing model. The shared API uses white for editable
pixels and black for protected pixels; Atelier composites protected source pixels
back into a successful result. A masked result must retain source dimensions.
Failures preserve the draft and expose **Retry mask save**. Window close waits
for the latest draft and recovery save; errors keep the app open.

Use card **Reference** actions to add up to two additional inputs. The selected
source is Image 1; additional references are Images 2 and 3. New-image generation
does not use the editing basket. Unsupported mask/multi-reference combinations
fail before credentials are requested or a paid job is submitted.

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
Atelier owns app-data API-key storage, files, document history and ComfyUI.
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

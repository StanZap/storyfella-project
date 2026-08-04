# Python vision runtime — HTTP API

The Python runtime (`python/main.py`, FastAPI/uvicorn) exposes a stable HTTP
contract that Rust consumes through `VisionClient` (`src/vision/mod.rs`). The
contract lives in `python/models/schemas.py`; the routing lives in
`python/api/routes.py`; the orchestration lives in `python/runtime/service.py`.

The service binds to `127.0.0.1:8765` when launched by the desktop app:

```bash
cd python
.venv/bin/python main.py --host 127.0.0.1 --port 8765
```

All endpoints accept and return JSON. Validation failures return FastAPI's
standard `422` with a validation body; runtime failures are reported **in
band** with `"status": "failed"` and an `error` field rather than as HTTP
errors. The Rust client surfaces both kinds: non-2xx becomes
`VisionClientError::Status`, in-band failures are interpreted by callers such
as `CreativeRuntime::wait_for_job`.

## Endpoints

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/health` | Liveness probe |
| `GET` | `/capabilities` | torch/CUDA/MPS device discovery |
| `POST` | `/segment` | SAM 2.1 masks with box/point/text grounding |
| `POST` | `/generate` | Synchronous generation (native Krea or Diffusers POC) |
| `POST` | `/generation/jobs?priority=` | Asynchronous native Krea generation |
| `GET` | `/generation/jobs/{id}` | Poll a generation job |
| `POST` | `/generation/jobs/{id}/cancel` | Cancel a queued job |
| `GET` | `/generation/capabilities` | Native runtime model-readiness |
| `POST` | `/caption` | Placeholder; not implemented |

## `GET /health`

```json
{ "status": "ok" }
```

## `GET /capabilities`

Reports what the optional torch environment can see. Without the
`image-generation`/`segmentation` extras, `torch_available` is `false`.

```json
{
  "torch_available": true,
  "torch_version": "2.13.0",
  "cuda_available": false,
  "cuda_devices": [],
  "mps_available": true,
  "recommended_device": "mps"
}
```

`recommended_device` is `cuda`, `mps`, or `cpu` in that preference order.
Device discovery lives in `python/runtime/device.py`.

## Shared request fields

### Compute device

Every model request accepts `device`: `"auto" | "cuda" | "mps" | "cpu"`
(default `"auto"`). `auto` resolves through `recommended_device`. Requesting an
unavailable device is an in-band failure.

### Generation requests

`GenerateRequest`:

| Field | Type | Default | Constraints |
| --- | --- | --- | --- |
| `prompt` | string | — | required |
| `reference_image_path` | string | none | app-owned image for edit mode |
| `width` | int | `1024` | 256–2048, multiple of 32 |
| `height` | int | `1024` | 256–2048, multiple of 32 |
| `steps` | int | `8` | 1–50 |
| `seed` | int | `0` | ≥ 0 |
| `model` | string | none | profile id; defaults per backend |
| `device` | string | `"auto"` | see above |
| `loras` | array | `[]` | max 8, see below |

`loras` entries: `{ "path": string, "multiplier": float }`. `path` must be
relative to the configured LoRA directory — absolute paths and `..` traversal
are rejected at validation time. `multiplier` is clamped to −2.0…2.0.

The interactive draft profile used by the desktop app is 768×448 at four steps
(`src/ui/editor.rs`).

## `POST /segment`

Request — one of three prompt styles:

```json
{
  "image_path": "/abs/path/frame.png",
  "prompt": "lighthouse",
  "device": "auto"
}
```

```json
{
  "image_path": "/abs/path/frame.png",
  "boxes": [{ "x_min": 10, "y_min": 20, "x_max": 100, "y_max": 200 }]
}
```

```json
{
  "image_path": "/abs/path/frame.png",
  "points": [{ "x": 50, "y": 80, "label": 1 }]
}
```

- A `prompt` with no points/boxes routes through Grounding DINO to produce
  boxes, which are then fed to SAM (`python/runtime/grounder.py`).
- `points` use `label` `1` (positive) or `0` (negative) and describe one object.
- Boxes must be ordered (`x_max > x_min`, `y_max > y_min`).

Response:

```json
{
  "status": "completed",
  "masks": [
    {
      "path": "/repo/cache/masks/abc.png",
      "score": 0.966,
      "area_pixels": 36445,
      "bounding_box": { "x_min": 10, "y_min": 20, "x_max": 100, "y_max": 200 }
    }
  ],
  "detections": [
    { "label": "lighthouse", "score": 0.95, "bounding_box": { "x_min": 10, "y_min": 20, "x_max": 100, "y_max": 200 } }
  ],
  "model": "facebook/sam2.1-tiny",
  "device": "mps",
  "dtype": "float32",
  "duration_ms": 9433
}
```

`detections` is populated only when text grounding produced the boxes. Masks
are written to `SVS_MASK_DIR` (default `<repo>/cache/masks`).

## `POST /generate` — synchronous generation

This is the legacy blocking endpoint. `RoutedImageGenerator`
(`python/runtime/image_generator.py`) dispatches by model name: a Krea profile
id (`krea-2-turbo-q2` / `krea-2-turbo-q4`, the default) is sent to the native
sd.cpp backend and waits for completion; any other model name is sent to the
Diffusers SANA Sprint POC pipeline. The product path uses the async jobs
endpoint instead.

```bash
curl -s http://127.0.0.1:8765/generate \
  -H 'Content-Type: application/json' \
  -d '{"prompt":"a lighthouse at dusk","width":768,"height":448,"steps":4}'
```

Response (native route):

```json
{
  "status": "completed",
  "image_path": "/repo/cache/generated/xxx.png",
  "model": "krea-2-turbo-q2",
  "device": "native",
  "dtype": "Q2_K",
  "seed": 0,
  "width": 768,
  "height": 448,
  "duration_ms": 12000
}
```

## `POST /generation/jobs?priority=interactive|background` — async native generation

The product-facing entry point. Only native Krea profiles (`krea-2-turbo-q2`,
`krea-2-turbo-q4`) are accepted; anything else returns a `failed` job with
`id: "not_submitted"`. `priority` is `interactive` (default) or `background`;
it is retained for the future application scheduler.

```bash
curl -s -X POST 'http://127.0.0.1:8765/generation/jobs?priority=interactive' \
  -H 'Content-Type: application/json' \
  -d '{
    "prompt": "A quiet lighthouse above a silver sea at dusk",
    "width": 768, "height": 448, "steps": 4, "seed": 0,
    "model": "krea-2-turbo-q2"
  }'
```

Response (`202 Accepted`):

```json
{
  "id": "job-uuid",
  "status": "queued",
  "queue_position": 0,
  "image_path": null,
  "model": "krea-2-turbo-q2",
  "priority": "interactive",
  "error": null
}
```

`reference_image_path` is honored by the adapter: the app-owned image is read
and base64-encoded into sd.cpp's `ref_images` field for Krea edit mode
(`python/runtime/sd_cpp.py`).

## `GET /generation/jobs/{id}` — polling

```json
{
  "id": "job-uuid",
  "status": "completed",
  "queue_position": 0,
  "image_path": "/repo/cache/generated/yyy.png",
  "model": "krea-2-turbo-q2",
  "priority": "interactive",
  "error": null
}
```

`status` is one of `queued`, `generating`, `completed`, `failed`, `cancelled`.
`image_path` is only present on completion. On completion, the adapter writes
the PNG to `SVS_GENERATED_DIR` (default `<repo>/cache/generated`) and keeps the
job→path mapping in memory so repeated polls do not re-write the file.

## `POST /generation/jobs/{id}/cancel`

Cancels a queued job. An already-generating diffusion graph cannot currently be
preempted safely, so the response reflects the actual job state; a cancel on a
generating job may report it still `generating` with the error attached.

## `GET /generation/capabilities`

Used during cold startup to distinguish "backend not up" from "backend up,
model still loading":

```json
{ "status": "ready", "model": "krea2_turbo-q2_k.gguf" }
```

or

```json
{ "status": "unavailable", "model": null, "error": "sd.cpp is unavailable at ..." }
```

The reported model name must match the diffusion artifact for the active
profile (`StableDiffusionCppClient::assert_profile`); a mismatch means the
wrong quantization is resident and the native server must be restarted.

## `POST /caption`

Placeholder. Always returns:

```json
{ "status": "not_implemented", "caption": "Placeholder caption" }
```

## Error conventions

- **Validation (422):** schema violations — bad dimensions, unordered boxes,
  unsafe LoRA paths.
- **Transport (in band):** missing files, unavailable backends, model/profile
  mismatches → `status: "failed"` plus a human-readable `error`.
- **Job lifecycle:** failures are attached to the job's `status`/`error`, not
  raised as HTTP errors, so the Rust poller can surface them exactly once.

## Rust client

`VisionClient` (`src/vision/mod.rs`) wraps these endpoints with typed request
and response structs. Notable behaviors:

- `submit_generation(request, false)` posts with `priority=interactive`.
- `wait_for_job` (in `CreativeRuntime`) polls until completion with a
  three-minute deadline and maps failed/cancelled jobs into
  `CreativeRuntimeError::Job`.
- Generated images are imported into the app asset directory by
  `CreativeRuntime::import_asset` before the UI references them, so job paths
  from `SVS_GENERATED_DIR` are an intermediate artifact, not the persisted
  project reference.

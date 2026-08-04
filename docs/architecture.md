# Architecture

Smart Visual Sequencer is a desktop application with a deliberate boundary
between product code and machine-learning code:

- **Rust** owns the Dioxus UI, application state, domain models, project
  persistence, model download/provisioning, process lifecycle, directory
  resolution, and the LM Studio client.
- **Python** owns every ML framework and vision implementation detail. Rust
  never imports or knows about PyTorch, Transformers, or SAM.
- **HTTP** is the only boundary between them, and the Python side is a normal
  FastAPI service with stable contracts (`python/models/schemas.py`).
- **stable-diffusion.cpp** (`sd-server`) is a separate native process owned by
  Rust that keeps the Krea 2 diffusion model resident.
- **LM Studio** is an external OpenAI-compatible service accessed directly from
  Rust. It currently ships a client but no planner business logic.

## Component map

```text
┌────────────────────────────── Rust (Dioxus desktop) ──────────────────────────────┐
│                                                                                    │
│  src/ui/          Workspace shell, Studio (Canvas/Storyboard/Timeline), Settings    │
│  src/state/       AppState: current project, selection, revision lifecycle          │
│  src/models/      Project / StoryboardFrame / ImageRevision, TOML persistence       │
│  src/timeline/    Clip / Timeline domain types                                      │
│  src/llm/         LmStudioClient (OpenAI-compatible chat completions)               │
│  src/vision/      VisionClient (typed HTTP client for the Python runtime)           │
│  src/runtime/     CreativeRuntime facade + PythonRuntime + GenerationRuntime +      │
│                   ModelStore (download, resume, checksum verify)                    │
│  src/assets/      AssetCatalog (import/index boundary; currently minimal)           │
│                                                                                    │
│  spawns and supervises:                                                            │
│   ┌─────────────────────┐        ┌──────────────────────────────────────────────┐  │
│   │ python/main.py      │  HTTP  │ sd-server (stable-diffusion.cpp)             │  │
│   │ FastAPI on :8765    │ ─────▶ │ resident Krea 2 + Qwen3-VL + VAE on :7861    │  │
│   └─────────────────────┘        └──────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────────────────┘
  │
  │ OpenAI-compatible HTTP (src/llm/)
  ▼
┌──────────────────────────────┐
│ LM Studio (external, user)   │
└──────────────────────────────┘
```

The native generation server is started with `--listen-ip 127.0.0.1
--listen-port 7861`; the Python runtime listens on `127.0.0.1:8765`
(see [`runtime-lifecycle.md`](runtime-lifecycle.md)). Both are loopback-only.

## The prompt-to-revision flow

The Canvas is the primary workspace. Submitting a prompt creates a story beat;
generation turns that beat into an image; follow-ups turn the current frame
into a reference image for the next revision.

```text
Canvas prompt
    │  state::AppState::add_storyboard_beat
    ▼
Storyboard beat (StoryboardFrame) ──▶ Timeline clip (5 s, appended)
    │  user clicks "Generate image"
    ▼
start_generation (src/ui/editor.rs)
    │  app_state.start_revision → Queued
    │  CreativeRuntime::ensure_ready (cold start: launch sd-server + python)
    │  app_state.update_revision → Generating
    │  VisionClient::submit_generation(POST /generation/jobs, priority=interactive)
    ▼
sd.cpp generates into python's SVS_GENERATED_DIR
    │  CreativeRuntime::wait_for_job polls GET /generation/jobs/{id}
    ▼
completed job exposes image_path
    │  CreativeRuntime::import_asset copies into <asset_dir>/generated/<uuid>.png
    ▼
app_state.update_revision → Completed + asset_path
    │  UI serves the file through the `generated` asset handler (`/generated/<file>`)
    ▼
Frame image + revision history in Properties
```

A follow-up ("Apply change") repeats the same path with
`GenerateRequest.reference_image_path` set to the current frame's asset path.
The Python adapter reads that app-owned file and sends it to sd.cpp as
`ref_images`, invoking Krea's edit mode. Every completed result is stored as a
selectable `ImageRevision`, and restoring an old revision rewrites the frame's
active asset.

The interactive draft profile requests 768×448 at four steps to favor iteration
speed (`src/ui/editor.rs`, `generate_image`).

## Process boundaries

| Process | Owner | Lifetime | Purpose |
| --- | --- | --- | --- |
| Dioxus desktop app | Rust | Application session | UI, state, orchestration |
| `python/main.py` (uvicorn) | Rust (`PythonRuntime`) | On demand, resident after first generation | HTTP adapter, result store, segmentation, Diffusers POC |
| `sd-server` | Rust (`GenerationRuntime`) | On demand, resident | Persistent Krea 2 inference |
| LM Studio | User (external) | External | LLM planning/VLM (future) |

Rust does not start the Python runtime at app launch; the shell reports the
runtime as idle until a generation request triggers `ensure_ready`
(see [`runtime-lifecycle.md`](runtime-lifecycle.md#readiness)).

## Directory resolution

Runtime, model, cache, and asset paths resolve through the `directories` crate
(`ProjectDirs::from("dev", "Smart Visual Sequencer", "Smart Visual Sequencer")`)
and can be overridden in [`config/app.toml`](../config/app.toml). The checked-in
config points everything at the repository (`models/`, `cache/`, `assets/`) for
development.

## What is deliberately absent

- **No planner business logic.** `src/llm/` is a client only; the typed planner
  output and the Canvas → LM Studio connection are roadmap work.
- **No ComfyUI.** Image generation is native (`stable-diffusion.cpp`) or a
  Diffusers POC behind `/generate`; ComfyUI is not a dependency.
- **No web/mobile targets.** The app is desktop-only; `cargo` features are
  limited to `desktop`.
- **Captioning is a placeholder.** `POST /caption` returns
  `status = "not_implemented"` until a captioning model is selected.

See the [README roadmap](../README.md#roadmap) for the ordered backlog.

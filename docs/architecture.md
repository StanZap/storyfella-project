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
│  src/ui/          Workspace shell, Canvas (artifact workspace), Settings             │
│  src/state/       AppState: registry, selection, undo/redo stacks                   │
│  src/registry/    Artifact model, typed ops + pipelines, slash parser,               │
│                   CreativeBackend (live backend)                                    │
│  src/persistence/ ProjectDb (SQLite, versioned migrations)                          │
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

The Canvas is the primary workspace. A slash command in the composer parses
into a typed operation (`src/registry/slash.rs`); model-only operations
(`create`, `variant`) apply instantly, while `regenerate`/`modify` run their
pipeline against `CreativeBackend` in a spawned task.

```text
Composer: /create character Mia…  /variant c:mia …  /regenerate c:mia …
    │  slash::parse_slash → Operation (compile validates against the registry)
    ▼
ops::execute (src/registry/ops.rs)
    ├─ Direct (create/variant): registry mutation + operation-log entry
    └─ Pipeline (regenerate/modify):
         │  AppState::snapshot_for_undo (registry snapshot — undo basis)
         ▼
       CreativeBackend::generate (src/registry/backend.rs)
         │  CreativeRuntime::ensure_profile_ready (cold start: sd-server + python)
         │  VisionClient::submit_generation(POST /generation/jobs)
         ▼
       sd.cpp generates into python's SVS_GENERATED_DIR
         │  CreativeRuntime::wait_for_job polls GET /generation/jobs/{id}
         ▼
       completed job exposes image_path
         │  CreativeRuntime::import_asset copies into <asset_dir>/generated/<uuid>.png
         ▼
       registry.finish_revision → completed revision + active image
         │  has_unsaved_changes → autosave writes the SQLite snapshot
         ▼
Artifact detail: image + revision history (served via /generated/<file>)
```

A `modify` follows the canonical regional-edit flow (segment → confirm mask →
inpaint, composite fallback) — see `docs/ROADMAP.md` §5. The canvas resolves
checkpoints automatically (best mask candidate, accept text); the CLI offers
interactive approval for careful runs (`svs op modify --approve interactive`).
The interactive draft profile requests 768×448 at four steps to favor
iteration speed on slower hardware.

## Process boundaries

| Process | Owner | Lifetime | Purpose |
| --- | --- | --- | --- |
| Dioxus desktop app | Rust | Application session | UI, state, orchestration |
| `python/main.py` (uvicorn) | Rust (`PythonRuntime`) | On demand, resident after first generation | HTTP adapter, result store, segmentation, Diffusers POC |
| `sd-server` | Rust (`GenerationRuntime`) | On demand, resident | Persistent Krea 2 inference |
| LM Studio | User (external) | External | LLM planning/VLM (future) |

Rust does not start the Python runtime at app launch; the shell reports the
runtime as idle until a generation request triggers `ensure_ready` — or until
it is started manually from the Settings Status section
(see [`runtime-lifecycle.md`](runtime-lifecycle.md#on-demand-start-and-stop)).

## Directory resolution

Runtime, model, cache, and asset paths resolve through the `directories` crate
(`ProjectDirs::from("dev", "Smart Visual Sequencer", "Smart Visual Sequencer")`)
and can be overridden in [`config/app.toml`](../config/app.toml). The checked-in
config points everything at the repository (`models/`, `cache/`, `assets/`) for
development.

## What is deliberately absent

- **The planner lives in `src/registry/`.** The typed operation set, the
  pipeline layer, the slash parser, and the `svs` CLI (see
  `docs/api-slice-1.md`) are implemented; `src/llm/` is still a client
  only. The Canvas → LM Studio connection (free-form composer messages
  proposed as operation stacks) is roadmap work.
- **No ComfyUI.** Image generation is native (`stable-diffusion.cpp`) or a
  Diffusers POC behind `/generate`; ComfyUI is not a dependency.
- **No web/mobile targets.** The app is desktop-only; `cargo` features are
  limited to `desktop`.
- **Captioning is a placeholder.** `POST /caption` returns
  `status = "not_implemented"` until a captioning model is selected.

See the [README roadmap](../README.md#roadmap) for the ordered backlog.

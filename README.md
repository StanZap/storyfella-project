# Smart Visual Sequencer

Smart Visual Sequencer is a cross-platform Dioxus desktop application for planning, generating, and arranging visual stories. The first product workflow includes project creation, the artifact canvas (characters, environments, objects, scenes with typed operations, variants, revisions), resident local Krea generation, and sectioned runtime settings. The typed operation/planner layer is shared by the `svs` CLI and the canvas; Studio sequencing and presentation/export orchestration are not wired yet.

The supported targets are macOS on Apple Silicon (MPS) and Linux with NVIDIA CUDA. LM Studio is an external prerequisite and is never installed or bundled by this project.

This is intentionally a desktop-only application. Web, mobile, and WASM targets are not supported.

## Architecture

- **Rust** owns the Dioxus UI, application state, the artifact registry, project persistence boundary, model-download boundary, directory resolution, process lifecycle, LM Studio integration, and Python runtime client.
- **Python** owns all future model framework and vision implementation details. Rust must not depend on or know about PyTorch. Krea 2 generation uses a separate native `stable-diffusion.cpp` process owned by Rust; it is not ComfyUI.
- **HTTP** is the explicit boundary between them. The local FastAPI runtime exposes `GET /health`, `GET /capabilities`, `POST /segment`, `/generate`, and `/caption`, plus asynchronous generation-job endpoints. Generation and geometric-prompt segmentation have working POC backends; captioning remains a placeholder.
- **LM Studio** is accessed directly from Rust through its OpenAI-compatible `/v1/chat/completions` endpoint. The typed planner vocabulary (operations + pipelines) lives in `src/registry/`; `src/llm/` is the client only.

## Product surface

- **Projects** creates a fresh story and reopens existing `.svs-project.db` files.
- **Canvas** is the artifact workspace: a sidebar grouped by kind
  (characters, environments, objects, scenes, stories), a detail pane with
  the active image, description, variants, revisions, children, and the
  operation log, plus a slash-command composer (`/create`, `/variant`,
  `/regenerate`, `/modify` — the same typed vocabulary as the `svs` CLI).
  Undo/redo restores registry snapshots.
- **Image generation** starts the configured native Krea Q2/Q4 runtime and
  lightweight Python adapter on demand, waits for both to become ready, and
  leaves the model resident for subsequent requests. `regenerate` and
  `modify` (mask-guided regional edits) run through the same pipelines as
  the CLI.
- **Variants** are first-class artifacts (`c:mia-outfit`), created with the
  `/variant` command or the detail-pane action; axes (outfit, weather, …)
  are tags for organization.
- **Settings** separates service status, general behavior, LM Studio/VLM
  configuration, Krea generation, and storage. A Status section probes LM
  Studio, the vision runtime, and the native generation backend on demand,
  reports readiness, model residency, and provisioning gaps, and can start,
  stop, or restart the two local runtimes. The Intelligence section
  discovers LM Studio's model list into a planner dropdown (selection
  applies for the session; `config/app.toml` holds the default).

The shell deliberately uses a small reusable component vocabulary, low-contrast boundaries, and progressive disclosure instead of a dense card dashboard. Styling is primarily Tailwind CSS 4 utility classes in RSX, with a small shared CSS layer for global browser behavior and form controls.

### Current creative workflow

1. Create artifacts with the composer: `/create character "Mia, a lighthouse keeper" --name mia`, `/create environment "a storm-tossed harbor" --name harbor`, `/create object "an antique brass lantern" --name lantern`.
2. Iterate versions: `/variant c:mia "in yellow rain gear" --axis outfit` — variants appear under the base artifact and can be selected like any artifact.
3. Generate images (Krea provisioned): `/regenerate c:mia "make it warmer"` — the result is a new revision of the active image.
4. Regional edits: `/modify c:mia "change her hair" --mask "her hair" --inpaint "a bob cut"` (or omit the prompts when an LLM is configured to plan them).
5. Browse revisions in the detail pane; click any completed revision to show it; undo/redo in the header.

The interactive draft profile currently requests a 768×448 image at four steps to favor iteration speed. Final-quality rendering, planner-assisted prompt rewriting, Studio sequencing, and presentation export remain roadmap work.

## Project layout

```text
src/
  app/          Dioxus root and configuration loading
  ui/           Desktop UI components (canvas, projects, settings)
  state/        AppState: registry, selection, undo/redo
  llm/          LM Studio OpenAI-compatible client
  vision/       Typed Python runtime HTTP client
  registry/     Artifact registry, typed ops + pipelines, slash parser,
                CreativeBackend, image ops
  runtime/      Python process lifecycle and model storage/download boundary
  persistence/  SQLite project store (versioned migrations)
  assets/       Asset catalog boundary
  bin/svs.rs    Operation CLI (see docs/api-slice-1.md)
python/
  api/           FastAPI construction and routes
  models/        Pydantic request/response contracts
  runtime/       Device discovery and model adapters
  main.py        Runtime entry point
models/          Optional project-local model storage
assets/          Dioxus/static and project assets
cache/           Optional project-local cache
config/app.toml  Runtime and path configuration
```

By default, generated model, cache, and asset data resolve through platform-appropriate application directories using the Rust `directories` crate. Paths can be overridden in `config/app.toml`.

## Documentation

In-depth material lives in [`docs/`](docs/README.md): a system [architecture overview](docs/architecture.md), the [developer guide](docs/development.md), the [vision runtime HTTP contract](docs/http-api.md), the [runtime lifecycle](docs/runtime-lifecycle.md), and the [data model and persistence format](docs/data-model.md). Proof-of-concept notes are indexed in [`docs/poc/`](docs/poc/README.md).

## Prerequisites

- Rust stable
- The current stable Dioxus CLI (`dx`)
- Node.js/npm (for the pinned Tailwind CSS 4 toolchain)
- `uv`
- LM Studio, running separately when LLM features are exercised
- Linux desktop builds: the system packages required by Dioxus/WebKitGTK

## Set up the Python runtime

From the repository root:

```bash
uv sync --project python
```

This creates `python/.venv` and installs FastAPI, Uvicorn, Pydantic, Pillow, and NumPy. ML frameworks are intentionally absent from the base environment.

To install optional POC dependencies:

```bash
uv sync --project python --extra image-generation
uv sync --project python --extra segmentation
```

For development, start the service directly:

```bash
cd python
.venv/bin/python main.py --host 127.0.0.1 --port 8765
```

Then verify it with `curl http://127.0.0.1:8765/health`.

## Run the desktop application

Install the frontend toolchain once:

```bash
npm install
```

The root `tailwind.css` file selects Tailwind CSS 4 and lets `dx` automatically compile `assets/tailwind.css`. Most component styling is expressed as Tailwind utility classes in RSX. To build or watch CSS independently, use `npm run css:build` or `npm run css:watch`.

```bash
dx serve --platform desktop
```

For a compile-only check:

```bash
cargo check --features desktop
```

This checkout includes `.cargo/config.toml` because its current parent directory contains a literal `:`. On macOS and Linux that character conflicts with dynamic-library path lists, so build artifacts are placed under `/tmp/smart-visual-sequencer-target`.

Configuration lives in `config/app.toml`. The default LM Studio URL is `http://localhost:1234/v1`; update `model` to the identifier loaded in LM Studio. `api_key` is optional.

## How Rust starts Python

`runtime::PythonRuntime` resolves the interpreter inside `python/.venv`, launches `python/main.py` with an explicit host and port, inherits logs, and retains the child handle for shutdown. The UI does not start it automatically yet; lifecycle wiring will be added with project/session management. Until then the shell reports the runtime as idle. Readiness must be determined through `/health`, not merely by successful process creation.

## Intended pipeline (documentation only)

```text
Prompt
  ↓
LLM Planner (LM Studio)
  ↓
Vision Runtime
  ↓
Segmenter
  ↓
Image Generator
  ↓
Evaluator
  ↓
Repeat until quality threshold
```

No part of this orchestration loop is implemented in the bootstrap.

## Visual LLM probe

The first isolated proof of concept exercises vision-capable models through LM Studio's OpenAI-compatible chat-completions endpoint. It uses deterministic synthetic fixtures, structured JSON output, per-request latency and token capture, and machine-readable results. See [`docs/poc/vlm/README.md`](docs/poc/vlm/README.md) for commands and scoring limitations.

## Image-generation probe

The second proof of concept runs a small SANA Sprint Diffusers pipeline behind the Python `/generate` contract. It selects CUDA, MPS, or CPU at runtime, loads lazily, serializes accelerator access, and records deterministic output metadata. See [`docs/poc/image-generation/README.md`](docs/poc/image-generation/README.md) for setup, the first checked-in result, licensing notes, and limitations.

The product-oriented native path supports resident Krea 2 Turbo Q2_K and IQ4_XS profiles, a shared quantized Qwen3-VL 4B encoder, explicit request-time LoRAs, Metal/CUDA, and background jobs without ComfyUI. See [`docs/poc/native-generation/README.md`](docs/poc/native-generation/README.md) for model layout, build instructions, memory assumptions, and remaining benchmark gates.

## Segmentation probe

The third proof of concept runs SAM 2.1 Tiny behind `/segment` with point or box prompts. Text prompts are grounded to boxes by Grounding DINO Tiny before SAM refinement. Both stages expose their own scores, and the chain runs on MPS, CUDA, or CPU. See [`docs/poc/segmentation/README.md`](docs/poc/segmentation/README.md) for the SAM decision and [`docs/poc/grounded-segmentation/README.md`](docs/poc/grounded-segmentation/README.md) for the connected pipeline result.

## Roadmap

The ordered backlog with statuses lives in `docs/ROADMAP.md` §12
(this is the tracker). Upcoming work in brief:

1. Connect free-form Canvas messages to LM Studio proposals (the vocabulary
   exists; `svs stack propose` is the contract test bed) and add
   `c:` autocomplete.
2. Add the interactive-first job scheduler and Studio sequencing (scenes
   group the timeline; compose beats in the canvas).
3. Wire Python/native runtime lifecycle, health, and resident-model state into the shell.
4. Add versioned project persistence, generated-asset indexing, and autosave.
5. Wire model-download progress/cancellation, LoRA discovery, and runtime installation into Settings.
6. Connect grounded segmentation to image editing and benchmark SAM 3.1 on Linux/CUDA.
7. Complete Q2/Q4 warm-latency and peak-memory gates on 24 GiB Metal and CUDA targets.
8. Add evaluator metrics, API integration tests, and macOS/Linux platform CI coverage.

# Smart Visual Sequencer

Smart Visual Sequencer is a cross-platform Dioxus desktop application for planning, generating, and arranging visual stories. The application shell, process boundary, configuration, HTTP contracts, and isolated Visual LLM, image-generation, and segmentation proofs of concept are present. Product orchestration is not implemented yet.

The supported targets are macOS on Apple Silicon (MPS) and Linux with NVIDIA CUDA. LM Studio is an external prerequisite and is never installed or bundled by this project.

## Architecture

- **Rust** owns the Dioxus UI, application state, storyboard/timeline models, project persistence boundary, model-download boundary, directory resolution, process lifecycle, LM Studio integration, and Python runtime client.
- **Python** owns all future model framework and vision implementation details. Rust must not depend on or know about PyTorch.
- **HTTP** is the explicit boundary between them. The local FastAPI runtime exposes `GET /health`, `GET /capabilities`, and `POST /segment`, `/generate`, and `/caption`. Generation and geometric-prompt segmentation have working POC backends; captioning remains a placeholder.
- **LM Studio** is accessed directly from Rust through its OpenAI-compatible `/v1/chat/completions` endpoint. The bootstrap contains the client but no planner business logic.

## Project layout

```text
src/
  app/          Dioxus root and configuration loading
  ui/           Desktop UI components
  state/        Shared application state
  timeline/     Sequencer and clip domain types
  llm/          LM Studio OpenAI-compatible client
  vision/       Typed Python runtime HTTP client
  runtime/      Python process lifecycle and model storage/download boundary
  models/       Project/storyboard data and persistence boundary
  assets/       Asset catalog boundary
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

`runtime::PythonRuntime` resolves the interpreter inside `python/.venv`, launches `python/main.py` with an explicit host and port, inherits logs, and retains the child handle for shutdown. The UI does not start it automatically yet; lifecycle wiring will be added with project/session management. Readiness should be determined through `/health`, not merely by successful process creation.

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

## Segmentation probe

The third proof of concept runs SAM 2.1 Tiny behind `/segment` with point or box prompts. It produces scored mask PNGs and runs on MPS, CUDA, or CPU. Text-only segmentation is explicitly deferred to a separate grounding adapter. See [`docs/poc/segmentation/README.md`](docs/poc/segmentation/README.md) for the model decision, commands, first checked-in mask, and SAM 3.1 constraints.

## Roadmap

1. Add versioned project persistence and asset indexing.
2. Wire Python lifecycle and health status into application startup/shutdown.
3. Add resumable model downloads with checksums and platform-aware storage.
4. Add text-to-box grounding and benchmark SAM 3.1 on Linux/CUDA.
5. Benchmark warm image-generation latency and add Rust-provisioned model manifests.
6. Add planner schemas, evaluator metrics, cancellation, and job progress events.
7. Add API integration tests and macOS/Linux platform CI coverage.

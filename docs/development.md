# Development

This document covers setting up a machine and the day-to-day commands for the
Rust application and the Python vision runtime.

## Prerequisites

- Rust stable
- The current stable Dioxus CLI (`dx`) — see <https://dioxuslabs.com>
- Node.js/npm (Tailwind CSS 4 toolchain, pinned in `package-lock.json`)
- `uv` (Python environment management)
- LM Studio, running separately, only when LLM features are exercised
- Linux desktop builds: the system packages required by Dioxus/WebKitGTK

Supported targets: macOS on Apple Silicon (MPS) and Linux with NVIDIA CUDA.

## The `:` in the repository path

The repository currently lives under a directory containing a literal `:` (for
example `lab:storycreator/`). On macOS and Linux that character is a separator
in dynamic-library path variables, so build artifacts are redirected out of the
checkout by `.cargo/config.toml`:

```toml
[build]
target-dir = "/tmp/smart-visual-sequencer-target"
```

This makes Cargo and `dx` work. Be aware that tools that resolve the checkout
as a "project path" (rather than a build path) can still trip over the `:`; if
a tool complains about a path separator, that is the cause.

## First-time setup

```bash
# Rust UI toolchain
npm install

# Python runtime base environment (FastAPI, uvicorn, pydantic, pillow, numpy)
uv sync --project python

# Optional POC extras — only needed for the Diffusers/SAM backends
uv sync --project python --extra image-generation
uv sync --project python --extra segmentation
```

The ML frameworks are intentionally optional; the API and contract tests do
not require torch.

## Run the desktop application

```bash
dx serve --platform desktop
```

`dx` compiles `assets/tailwind.css` from the root `tailwind.css` automatically.
To build or watch the CSS independently:

```bash
npm run css:build
npm run css:watch
```

Most component styling is expressed as Tailwind utility classes in RSX, so
editing Rust usually requires no CSS work. The committed `assets/tailwind.css`
is generated output; a regeneration that only changes that file is a build
artifact update, not a feature change.

### Compile-only checks

```bash
cargo check --features desktop
cargo clippy --features desktop --all-targets
```

## Run the Python vision runtime

For development, start the service directly:

```bash
cd python
.venv/bin/python main.py --host 127.0.0.1 --port 8765
```

Then verify it:

```bash
curl http://127.0.0.1:8765/health
```

When the desktop app launches the runtime itself it inherits its logs, so the
console output appears in the app's terminal. See
[`runtime-lifecycle.md`](runtime-lifecycle.md) for the environment variables
(`SVS_SD_CPP_URL`, `SVS_GENERATED_DIR`, `SVS_MASK_DIR`) the runtime reads.

## Tests

### Rust

```bash
cargo test --features desktop
```

The tests cover state transitions (`src/state/`), request serialization
(`src/vision/`), multimodal message shape (`src/llm/`), and profile manifests
(`src/runtime/`). They do not require a running Python runtime or LM Studio.

### Python

```bash
cd python
uv run python -m unittest discover -s tests
```

Some tests skip themselves when the optional torch environment is absent, so a
run that reports `OK (skipped=N)` is expected on a base environment.

## CLI tools

| Command | Purpose |
| --- | --- |
| `cargo run --bin model_setup -- --profile q2 --accept-krea-license` | Download, resume, and checksum-verify the Krea 2 Q2 profile |
| `cargo run --bin model_setup -- --profile q4 --accept-krea-license` | Same for Q4 |
| `cargo run --bin vlm_probe -- --list-models` | List models visible through LM Studio |
| `cargo run --bin vlm_probe -- --model <id>` | Run the visual-LLM probe against one model |
| `cargo run --example generate_vlm_fixtures` | Regenerate the deterministic probe fixtures |

`model_setup` refuses to run without `--accept-krea-license`; review the Krea 2
licensing terms before distributing weights.

## Configuration reference

Configuration lives in [`config/app.toml`](../config/app.toml) and is loaded by
`src/app/config.rs`. Missing sections fall back to defaults.

| Key | Default | Meaning |
| --- | --- | --- |
| `lm_studio.base_url` | `http://localhost:1234/v1` | LM Studio OpenAI-compatible endpoint |
| `lm_studio.model` | `local-model` | Model identifier loaded in LM Studio |
| `lm_studio.api_key` | none | Optional bearer token |
| `lm_studio.timeout_seconds` | `60` | Request timeout for planning/vision calls |
| `generation.base_url` | `http://127.0.0.1:7861` | sd.cpp endpoint the Python adapter calls |
| `generation.profile` | `krea-2-turbo-q2` | `krea-2-turbo-q2` or `krea-2-turbo-q4` |
| `generation.executable` | `<model_dir>/runtime/stable-diffusion.cpp/bin/sd-server` | Path to `sd-server` |
| `generation.lora_dir` | `<model_dir>/loras` | LoRA root; request paths are relative to it |
| `paths.python_runtime` | `python` | Directory containing `.venv` and `main.py` |
| `paths.model_dir` | platform data dir + `models` | Model artifacts root |
| `paths.cache_dir` | platform cache dir | Disposable cache data |
| `paths.asset_dir` | platform data dir + `assets` | Imported/generated assets |

An invalid `generation.profile` value is a hard configuration error and the app
shows the error screen on launch.

## Common issues

- **`cargo`/`dx` fail with path errors** — the `:` in the parent directory; the
  `.cargo/config.toml` target-dir redirect is the fix, do not remove it.
- **Generation says the runtime is not ready / job fails** — the sd-server
  binary or model artifacts are missing. Run `cargo run --bin model_setup`
  after reviewing the license, and confirm `generation.executable` in
  `config/app.toml`.
- **`/segment` fails with a torch import error** — the segmentation extra is
  not installed; run `uv sync --project python --extra segmentation`.
- **LM Studio calls fail** — confirm LM Studio is running, the configured model
  is loaded, and `lm_studio.base_url` matches the server's `/v1` endpoint.

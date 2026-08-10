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

# Compiled stylesheet: `assets/tailwind.css` is git-ignored generated output.
# `dx serve` compiles it automatically; for `cargo check`/`cargo test` on a
# fresh clone, generate it once (the asset macro embeds the file at compile time):
npm run css:build

# Python runtime base environment (FastAPI, uvicorn, pydantic, pillow, numpy)
uv sync --project python

# Optional POC extras — only needed for the Diffusers/SAM backends
uv sync --project python --extra image-generation
uv sync --project python --extra segmentation
```

The ML frameworks are intentionally optional; the API and contract tests do
not require torch.

### Native generation backend (`sd-server`)

The app spawns the `sd-server` binary from the vendored
`stable-diffusion.cpp` checkout (it is git-ignored, like `models/`). Build it
once and place it where `generation.executable` points:

```bash
# CUDA build for NVIDIA GPUs — without it generation runs on CPU (~11 GB
# weights; expect 30-45 s/step). Pick the compute capability of your card,
# e.g. 86 for RTX 3090 (Ampere), 89 for RTX 40-series.
cmake -S stable-diffusion.cpp -B stable-diffusion.cpp/build \
  -DSD_CUDA=ON -DCMAKE_CUDA_ARCHITECTURES=86
cmake --build stable-diffusion.cpp/build --target sd-server -j
cp stable-diffusion.cpp/build/bin/sd-server models/runtime/stable-diffusion.cpp/bin/
```

On Apple Silicon, build with MPS (`-DSD_MPS=ON`) instead.

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

### End-to-end: plans (the app's own regression suite)

Unit tests do not exercise the app. The automation driver (`src/driver/`)
does: it plays a **plan** — a YAML script written in the app's own vocabulary
— through the same code paths the UI uses (`src/commands.rs`), and asserts
outcomes against app state. Plans never mention selectors, coordinates or
DOM, so they do not break when the UI is restyled.

```bash
# Fast, backend-free: project, story, beats. Runs anywhere.
cargo run --features desktop --bin smart-visual-sequencer -- \
  --plan config/plans/smoke.yaml --mode check
```

`--mode check` runs the steps back to back and prints a verdict
(`verdict: 7 passed, 0 failed`, non-zero exit on failure) — this is the
regression mode. `--mode guide` (the default) runs the same plan slowly with
an assistant card narrating each step in the UI, which is the tutorial
experience and the better way to eyeball UX.

| Plan | Needs a backend | Covers |
| --- | --- | --- |
| `smoke.yaml` | no | Project creation, story and beat artifacts, registry state |
| `edit.yaml` | yes | Create a character, then recolour / re-cut / re-dress her by instruction |
| `iteration.yaml` | yes | The creation loop — create, variant, regenerate, beat |
| `tour.yaml` | yes | Guided walkthrough of the product surface |

Writing a plan: steps are `verb: value` pairs, `expect_*` steps gate
progress, `soft: true` downgrades a failure to a warning (use it for
`expect_image`, which depends on generation timing), and `message:` is the
narration shown in guide mode. `compose:` types text into the canvas
composer character by character and submits it exactly like a user pressing
Enter — prefer it over `command:` when the point is to exercise the UI.

Steps that stop for approval cannot be driven from a plan: the driver
auto-rejects checkpoints, so the op fails fast with the checkpoint message.
`modify` is checkpoint-free (instruction edits segment nothing), but its
opt-in mask path is not.

Runs are worth keeping — `.svs-e2e/logs/` is the gitignored home for plan
output and generation logs, and `.svs-e2e/quality/` for the images a run
produced. See `AGENTS.md` for why experiment scratch stays inside the
checkout rather than `/tmp`.

## CLI tools

| Command | Purpose |
| --- | --- |
| `cargo run --bin model_setup -- --profile q2 --accept-krea-license` | Download, resume, and checksum-verify the Krea 2 Q2 profile |
| `cargo run --bin model_setup -- --profile q4 --accept-krea-license` | Same for Q4 |
| `cargo run --bin vlm_probe -- --list-models` | List models visible through LM Studio |
| `cargo run --bin vlm_probe -- --model <id>` | Run the visual-LLM probe against one model |
| `cargo run --example generate_vlm_fixtures` | Regenerate the deterministic probe fixtures |
| `cargo run --bin svs -- --help` | Operation CLI (typed ops, stacks, runtime serve, log) — see [`api-slice-1.md`](api-slice-1.md) for the command reference and the light/LLM/generation session split |

`model_setup` refuses to run without `--accept-krea-license`; review the Krea 2
licensing terms before distributing weights. The `svs` CLI needs no
provisioned models for the model-only and LLM-assisted tiers; only
`regenerate`/`modify` touch the Krea backend.

## Configuration reference

Configuration lives in [`config/app.toml`](../config/app.toml) and is loaded by
`src/app/config.rs`. Missing sections fall back to defaults.

| Key | Default | Meaning |
| --- | --- | --- |
| `lm_studio.base_url` | `http://localhost:1234/v1` | LM Studio OpenAI-compatible endpoint |
| `lm_studio.model` | `local-model` | Model identifier loaded in LM Studio (editable in Settings → Intelligence; the picker applies it immediately and saves it back to this file) |
| `lm_studio.api_key` | none | Optional bearer token |
| `lm_studio.timeout_seconds` | `60` | Request timeout for planning/vision calls |
| `generation.base_url` | `http://127.0.0.1:7861` | sd.cpp endpoint the Python adapter calls |
| `generation.profile` | `krea-2-turbo-q4` | `krea-2-turbo-q2` or `krea-2-turbo-q4` |
| `generation.executable` | `<model_dir>/runtime/stable-diffusion.cpp/bin/sd-server` | Path to `sd-server` |
| `generation.lora_dir` | `<model_dir>/loras` | LoRA root; adapter files are discovered here (subfolders and symlinked folders included) |
| `generation.steps` | `4` | Default steps per generation (Settings → Image generation) |
| `generation.sizes.character` | `512x768` | Default canvas for character images (`WxH`, multiples of 32) |
| `generation.sizes.environment` | `1024x576` | Default canvas for environment images |
| `generation.sizes.object` | `768x768` | Default canvas for object images |
| `generation.sizes.scene` | `1024x576` | Default canvas for scene images |
| `generation.sizes.beat` | `1024x576` | Default canvas for beat images |
| `lora.inpaint` | none | Adapter file applied to the opt-in mask path of `modify` (`--mask-prompt` + `--inpaint-prompt`), e.g. `inpaintkrea.safetensors`. Instruction edits do not load it |
| `lora.character` | none | Adapter file applied to character generations without a project binding |
| `lora.environment` | none | Adapter file applied to environment generations without a project binding |
| `lora.object` | none | Adapter file applied to object generations without a project binding |
| `lora.multiplier` | `1.0` | Weight for the bindings above (per-artifact bindings carry their own) |
| `paths.python_runtime` | `python` | Directory containing `.venv` and `main.py` |
| `paths.model_dir` | platform data dir + `models` | Model artifacts root |
| `paths.cache_dir` | platform cache dir | Disposable cache data |
| `paths.asset_dir` | platform data dir + `assets` | Imported/generated assets |
| `paths.project_dir` | platform data dir + `projects` | Where the Projects screen keeps `.svs-project.db` files |

An invalid `generation.profile` value is a hard configuration error and the app
shows the error screen on launch. The checked-in `config/app.toml` sets
`lm_studio.model` to the gemma model id used on the dev machine — point it at
the model id your LM Studio serves (`curl http://localhost:1234/v1/models`).

## Common issues

- **`cargo`/`dx` fail with path errors** — the `:` in the parent directory; the
  `.cargo/config.toml` target-dir redirect is the fix, do not remove it.
- **Starting a service from Settings fails with `failed to start Python runtime: No such file or directory`** — this was a resolved bug where a relative `paths.python_runtime` combined with a relative working directory made the child resolve `python/.venv/bin/python` against the wrong directory. Spawn paths are now absolutized; if you still see this, the venv itself is missing or broken — recreate it with `uv sync --project python`.
- **`Python virtual environment executable not found`** — the venv is not provisioned; run `uv sync --project python` from the repository root.
- **Generation says the runtime is not ready / job fails** — the sd-server
  binary or model artifacts are missing. Run `cargo run --bin model_setup`
  after reviewing the license, and confirm `generation.executable` in
  `config/app.toml`.
- **`generation job failed: resident backend has … loaded; … requires …`** —
  the resident sd-server runs a different Krea profile than the request.
  Ops restart their own server automatically; a stale server from another
  session needs `svs runtime serve --force --model krea-2-turbo-q4` (or
  `pkill -f "sd-server --diffusion"`), then rerun.
- **`/segment` fails with a torch import error** — the segmentation extra is
  not installed; run `uv sync --project python --extra segmentation`.
- **LM Studio calls fail** — confirm LM Studio is running, the configured model
  is loaded, and `lm_studio.base_url` matches the server's `/v1` endpoint.

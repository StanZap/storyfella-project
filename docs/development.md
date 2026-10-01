# Development

This document covers setting up a machine and the day-to-day commands for the
Rust application.

## Prerequisites

- Rust stable
- The current stable Dioxus CLI (`dx`) — see <https://dioxuslabs.com>
- Node.js/npm (Tailwind CSS 4 toolchain, pinned in `apps/desktop/package-lock.json`)
- An OpenAI-compatible LLM endpoint (llama.cpp / LM Studio), running separately, only when LLM features are exercised
- A CUDA toolchain and an NVIDIA GPU to build and run `sd-server` (see below)
- Linux desktop builds: the WebKitGTK/GTK development packages (see below)

Validated target: Linux with an NVIDIA GPU (CUDA). The Qwen-Image-2.1 int8
weights run on a CUDA build of `sd-server`; other sd.cpp backends are not
validated here.

### Linux system packages

`dx doctor` prints the set it expects on Linux:

```
libwebkit2gtk-4.1-dev libgtk-3-dev libasound2-dev libudev-dev \
  libayatana-appindicator3-dev libxdo-dev libglib2.0-dev
```

On the dev machine (`Pop!_OS 24.04`) everything except `libasound2-dev` and
`libudev-dev` is installed, and the app builds without those two — nothing in
the tree links `libasound` or `libudev` yet. Install them when audio or
gamepad/input-device support arrives, not before.

Bundling (below) adds no system package: the AppImage is assembled by
`linuxdeploy`, which the Dioxus CLI downloads on demand.

## Workspace layout

The repository is a Cargo workspace. `crates/core` (package `svs-core`) holds
the domain layer — the artifact registry, typed operations, persistence, and
the vision/LLM/runtime clients — and has **no GUI dependency**, so the CLI and
the core tests never compile the toolkit or the WebKit stack. `apps/desktop`
(package `smart-visual-sequencer`) is the Dioxus application and owns the
`desktop` feature; `apps/cli` (package `svs-cli`) builds the `svs`,
`model_setup`, and `vlm_probe` binaries. Both application crates depend on
`svs-core` and name it explicitly (`svs_core::…`) rather than re-exporting it.

The native-inference runtime is **not** owned by this workspace: it is the
sibling `clipforge` project's crate, package **`clipforge-inference`**, pulled in
as a **path dependency** (`clipforge-inference = { path = "../clipforge/crates/inference" }`
in the root `[workspace.dependencies]`). It owns the `sd-server` process,
the `sdcpp` client, the served profile, and model provisioning. The sibling
checkout must be present at `../clipforge/crates/inference` or the workspace will
not build; the crate is not a member here, so `cargo test --workspace` does not
run its tests.

`apps/vision` was the Python vision runtime; it was removed with the Krea/SAM
migration — the project is pure Rust plus the native `sd-server`. Data —
`config/` and `assets/generated/` — stays at the workspace root, because project
databases store absolute asset paths. `xtask/` is the developer task runner
described below; nothing shipped depends on it.

The payoff is a cheap domain loop: `svs-core`'s dependency tree is 464 entries
with no `dioxus`/`webkit`/`gtk`/`tao`/`wry` in it at all, against 1183 for the
desktop crate, so

```sh
cargo test -p svs-core                  # domain tests, no toolkit
cargo run -p svs-cli --bin svs -- --help
```

skip the GUI stack entirely. The workspace-wide `cargo check --workspace
--all-targets` / `cargo test --workspace` cover everything, including the
desktop crate's 77 tests.

## Task runner (`cargo xtask`)

The recurring `dx` / `cargo` / `npm` invocations are wrapped by a
small task runner (`xtask/`, exposed through the `.cargo/config.toml` alias as
`cargo xtask`). It runs the same commands the sections below document, from the
workspace root, with every path resolved absolutely so it behaves the same from
any directory:

```bash
cargo xtask build                 # dx build --platform desktop (debug)
cargo xtask run [-r]              # build, then launch the app
cargo xtask run -l                # …teeing its output to .svs-e2e/logs/
cargo xtask plan config/plans/smoke.yaml --mode check
cargo xtask serve                 # dx serve (hot reload)
cargo xtask bundle                # release AppImage into dist/
cargo xtask bundle --package-types appimage deb
cargo xtask check                 # cargo check --workspace --all-targets
cargo xtask clippy [--deny]        # cargo clippy …; --deny errors on warnings
cargo xtask fmt [--check]         # cargo fmt --all
cargo xtask test                  # cargo test --workspace
cargo xtask css [--install]       # npm run css:build
cargo xtask doctor                # prerequisite tools + provisioned paths
cargo xtask clean                 # cargo clean + remove dist/
```

Debug is the default for `build`/`run`/`serve`; `bundle` defaults to release
(shipping a debug bundle is almost never intended). `-r`/`--release` and
`-d`/`--debug` override either, and conflict rather than silently picking one.
`run` and `plan` propagate the app's exit code, so `cargo xtask plan … --mode
check` gates a script.

`run` and `plan` build with `dx` (never plain `cargo`, which skips the asset
pipeline) and launch the binary from the workspace root so the app's
project-root path resolution finds `config/app.toml`. `-l`/`--log` tees the
app's stdout/stderr to `.svs-e2e/logs/app-<unix>.log` for the GUI live-test
workflow. `bundle` writes to `dist/` (git-ignored) and lists each artifact.

`cargo xtask <command> --help` documents the flags. The sections below remain
the reference for the underlying commands, for when you need a flag the task
runner does not expose.

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
# Rust UI toolchain — run from apps/desktop (which holds package.json)
cd apps/desktop
npm install

# Compiled stylesheet: `assets/tailwind.css` is git-ignored generated output.
# `dx serve` compiles it automatically; for `cargo check`/`cargo test` on a
# fresh clone, generate it once (the asset macro embeds the file at compile time):
npm run css:build
cd ..
```

### Native generation backend (`sd-server`)

The app spawns the `sd-server` binary from the vendored
`stable-diffusion.cpp` checkout (it is git-ignored, like `models/`). Qwen-Image-2.1
support landed upstream in `3f8527a` (`master-929-3f8527a`), so check the tree
out at least that revision before building. Build it once and place it where the
image provider's `executable` points (default
`<model_dir>/runtime/stable-diffusion.cpp/bin/sd-server`, see the
[`providers.toml`](#providerstoml) section below):

```bash
# CUDA build for NVIDIA GPUs — the Qwen-Image-2.1 int8 convrot weights need it.
# Pick the compute capability of your card, e.g. 86 for RTX 3090 (Ampere),
# 89 for RTX 40-series.
cmake -S stable-diffusion.cpp -B stable-diffusion.cpp/build \
  -DSD_CUDA=ON -DCMAKE_CUDA_ARCHITECTURES=86
cmake --build stable-diffusion.cpp/build --target sd-server -j
cp stable-diffusion.cpp/build/bin/sd-server models/runtime/stable-diffusion.cpp/bin/
```

The launcher passes `--model-args qwen_image_2_1_prefix_cache_type=q8_0` and
`--diffusion-fa`, and points `--diffusion-model`/`--llm`/`--vae` at the
profile's three safetensors (see [`runtime-lifecycle.md`](runtime-lifecycle.md)).
Provision those weights with `model_setup` (see the CLI tools table below).

## Run the desktop application

Commands are run from `apps/desktop` (where `Dioxus.toml` and `package.json`
live):

```bash
cd apps/desktop && dx serve --platform desktop
```

`cargo xtask serve` runs the same command (and `cargo xtask run` builds once
and launches without the dev server).

`dx` compiles `apps/desktop/assets/tailwind.css` from `apps/desktop/tailwind.css`
automatically. To build or watch the CSS independently:

```bash
cd apps/desktop && npm run css:build
cd apps/desktop && npm run css:watch
```

Most component styling is expressed as Tailwind utility classes in RSX, so
editing Rust usually requires no CSS work. The committed `assets/tailwind.css`
is generated output; a regeneration that only changes that file is a build
artifact update, not a feature change.

### Compile-only checks

```bash
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets
```

`cargo xtask check` and `cargo xtask clippy` are the same commands; the latter
takes `--deny` to fail on warnings (the tree is not warning-clean yet).

## Bundling

`cargo xtask bundle` wraps this section's command and drops the result in
`dist/`; the rest of the section is the underlying `dx` invocation and the
packaging details.

`dx bundle` produces the distributable artifacts; on Linux the useful package
types are `appimage` and `deb`. Run it from `apps/desktop`:

```bash
# Release build + AppImage (prints `Created AppImage: <path>` when it lands)
cd apps/desktop && dx bundle --platform desktop --package-types appimage -r

# One value per flag occurrence — `--package-types appimage deb` and
# `--package-types appimage,deb` are both rejected. Build several artifacts in
# one run: each run rebuilds into a cleaned bundle directory, so an appimage-only
# run deletes the .deb produced by the run before it.
cd apps/desktop && dx bundle --platform desktop --package-types appimage --package-types deb -r
```

No extra Rust crate is involved — bundling is the CLI's own job. For an
AppImage it lays out an `.AppDir` and runs **`linuxdeploy`**, downloading that
one tool from GitHub (`github.com/tauri-apps/binary-releases`, which redirects
to `objects.githubusercontent.com`) into `~/.local/share/.dx/tools` on first
use. The build itself can be offline; **the bundle step needs network access the
first time** (and on any machine without that cache). `linuxdeploy` is run with
`APPIMAGE_EXTRACT_AND_RUN=1`, so FUSE is not required.

Identity comes from `Dioxus.toml` — `[application] name` plus the `[bundle]`
`identifier`, `publisher`, `category` and descriptions. (`[linux]` is documented
as overriding those for Linux builds, but the *nothing* row below shows it is
not wired through.) `dx config schema` documents every key. What actually
reaches the Linux packages, verified on both package types:

| Field | Source |
| --- | --- |
| `Comment`, deb `Description` | `bundle.short_description` (+ `long_description`) |
| `Name` in the desktop entry | a sanitised product name — `Name=SmartVisualSequencer`, not `[application] name` verbatim |
| deb `Maintainer` | Cargo `authors`, not `bundle.publisher` |
| `Categories` / `Keywords` | **nothing** — `bundle.category` and `[linux] categories`/`keywords` are ignored; a categorised menu entry needs a custom desktop template |
| `Icon` | the default Dioxus icon while `[bundle] icon` is unset (the deb also drops a stray `.dx-default-icon.png` at its root) |

The AppImage does **not** carry the GUI stack: it links the host's GTK/
WebKitGTK (`libwebkit2gtk-4.1`), so it runs on a normal desktop that already has
WebKitGTK 4.1. Bundling the GUI stack itself needs `linuxdeploy-plugin-gtk` and
is not wired here.

Two inputs are still missing before a bundle is more than an empty shell (see
[`ROADMAP.md`](ROADMAP.md) §12 item 11):

- **An app icon.** `[bundle] icon` takes PNG paths (512×512 is the safe size).
  The repo has none — `apps/desktop/assets/favicon.ico` and
  `apps/desktop/assets/header.svg` came with the Dioxus scaffold and are
  referenced nowhere. With no icon configured, `dx` writes a default one, so
  the AppImage builds but is unlabelled.
- **The runtime payload.** `sd-server` is bundled with `[bundle] external_bin`
  (sidecar binaries) and/or `[bundle] resources` (data files); both keys are inert
  until the file exists. Shipping it also needs the code change named in item 11:
  resolve it relative to the bundle root instead of the provider's `executable`
  in `providers.toml`.

## Run the generation backend

An op starts its own `sd-server` when none is running, and that server dies when
the process exits (cold start per command). For a longer session, keep the
resident server in one terminal and let ops in other terminals reuse it:

```bash
cargo run -p svs-cli --bin svs -- runtime serve --force   # Ctrl-C to stop
```

`--force` kills stale `sd-server` processes from interrupted sessions that hold
the configured port (7861 in the checked-in `providers.toml`). Only the
`qwen-image-2.1` profile is served. See
[`runtime-lifecycle.md`](runtime-lifecycle.md).

## Tests

### Rust

```bash
cargo test --workspace
```

The workspace runs 131 tests in `svs-core` plus 77 in the desktop crate, and 3
in the CLI binaries (211 total). They cover state transitions
(`apps/desktop/src/state/`), the runtime adapter and health probes
(`crates/core/src/runtime/`), and multimodal message shape
(`crates/core/src/llm/`). They do not require a running `sd-server` or an LLM
endpoint.
The `sdcpp` wire serialization tests (request body + job decoding) live in the
shared `clipforge-inference` crate (`../clipforge/crates/inference/src/sd/`) and
are not part of this workspace's run. `cargo xtask test` runs the same command.

### End-to-end: plans (the app's own regression suite)

Unit tests do not exercise the app. The automation driver (`apps/desktop/src/driver/`)
does: it plays a **plan** — a YAML script written in the app's own vocabulary
— through the same code paths the UI uses (`apps/desktop/src/commands.rs`), and asserts
outcomes against app state. Plans never mention selectors, coordinates or
DOM, so they do not break when the UI is restyled.

```bash
# Fast, backend-free: project, story, beats. Runs anywhere.
cargo run -p smart-visual-sequencer --bin smart-visual-sequencer -- \
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
| `story-setup.yaml` | yes | The artifact kinds and how they relate — mostly text, so it is quick (~30s) |
| `edit-vocabulary.yaml` | yes | `modify` vs `describe` vs `variant` vs `regenerate`, including an edit vanishing and then surviving |
| `character-sheet.yaml` | yes | One character fanned across outfit / time-of-day / weather / mood axes |
| `canvas-and-quality.yaml` | yes | Draft small, iterate, then `/resize` once the composition is settled |
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
`modify` is checkpoint-free (an instruction edit segments nothing).

Two traps worth knowing before writing one. A bare `- wait_ms: 800` is **not**
a step — every step needs an action, and `wait_ms`/`message`/`soft` are
modifiers on one; serde reports that mistake against the first step in the
file rather than the offending one, so trust the message over the line
number. And a plan runs against the live app on a spawned task, sharing the
same state as the user: `create_project` (which every plan starts with)
switches away from whatever project is open, and typing in the composer
mid-run collides with the plan's own `compose` steps. Guide mode's assistant
card has pause/stop for exactly this; check mode calls `std::process::exit`
when the report prints.

Runs are worth keeping — `.svs-e2e/logs/` is the gitignored home for plan
output and generation logs, and `.svs-e2e/quality/` for the images a run
produced. See `AGENTS.md` for why experiment scratch stays inside the
checkout rather than `/tmp`.

## CLI tools

| Command | Purpose |
| --- | --- |
| `cargo run -p svs-cli --bin model_setup` | Download, resume, and checksum-verify the Qwen-Image-2.1 profile (diffusion + text encoder + VAE) into the image provider's model tree |
| `cargo run -p svs-cli --bin vlm_probe -- --list-models` | List models visible through the LLM endpoint |
| `cargo run -p svs-cli --bin vlm_probe -- --model <id>` | Run the visual-LLM probe against one model |
| `cargo run -p svs-cli --example generate_vlm_fixtures` | Regenerate the deterministic probe fixtures |
| `cargo run -p svs-cli --bin svs -- --help` | Operation CLI (typed ops, stacks, runtime serve, log, eval) — see [`api-slice-1.md`](api-slice-1.md) for the command reference and the light/LLM/generation session split |

`model_setup` provisions into the image provider's model tree (the
`model_dir` named in `providers.toml`, rooted at `models/` in this checkout).
The `svs` CLI needs no provisioned models for the model-only and
LLM-assisted tiers; only the visual ops (`create`/`variant`/`regenerate`/`modify`)
touch the generation backend.

## Configuration reference

Configuration lives in [`config/app.toml`](../config/app.toml) and is loaded by
`crates/core/src/config.rs`. Missing sections fall back to defaults. Relative
paths in the file resolve against the project root (see the `paths` note below).

| Key | Default | Meaning |
| --- | --- | --- |
| `llm.base_url` | `http://localhost:1234/v1` | OpenAI-compatible LLM endpoint (llama.cpp, LM Studio, …) |
| `llm.model` | `local-model` | Model identifier loaded in the endpoint (editable in Settings → Intelligence; the picker applies it immediately and saves it back to this file) |
| `llm.api_key` | none | Optional bearer token |
| `llm.timeout_seconds` | `60` | Request timeout for planning/vision calls |
| `generation.steps` | `25` | Default steps per generation (Settings → Image generation; Qwen-Image-2.1 has no few-step turbo path) |
| `generation.cfg_scale` | `6.0` | Text-guidance scale (`sample_params.guidance.txt_cfg`) |
| `generation.sample_method` | `euler` | Sampler (`sample_params.sample_method`) |
| `generation.sizes.character` | `512x768` | Default canvas for character images (`WxH`, multiples of 32) |
| `generation.sizes.environment` | `1024x576` | Default canvas for environment images |
| `generation.sizes.object` | `768x768` | Default canvas for object images |
| `generation.sizes.scene` | `1024x576` | Default canvas for scene images |
| `generation.sizes.beat` | `1024x576` | Default canvas for beat images |
| `lora.character` | none | Adapter file applied to character generations without a project binding |
| `lora.environment` | none | Adapter file applied to environment generations without a project binding |
| `lora.object` | none | Adapter file applied to object generations without a project binding |
| `lora.multiplier` | `1.0` | Weight for the bindings above (per-artifact bindings carry their own) |
| `paths.cache_dir` | platform cache dir | Disposable cache data |
| `paths.asset_dir` | platform data dir + `assets` | Imported/generated assets |
| `paths.project_dir` | platform data dir + `projects` | Where the Projects screen keeps `.svs-project.db` files |

Configured relative paths (`paths.*` and the plan directory) resolve against the
**project root**, not the working directory: `crates/core/src/paths.rs` picks
`$SVS_HOME` first, otherwise the nearest ancestor of the working directory (then
of the executable) that holds `config/app.toml`, so commands do not need to be
run from the repository root. The checked-in `config/app.toml` raises
`generation.sizes.character` to `1024x1536` (the full Qwen portrait canvas).

The checked-in `config/app.toml` sets `llm.model` to `prism-ml/bonsai-27b` —
point it at the model id your LLM endpoint serves
(`curl http://localhost:1234/v1/models`).

### providers.toml

The image backend — the `sd-server` executable, its model tree, LoRA root,
endpoint, and weights — is resolved from `<root>/providers.toml`, anchored to the
project root like every other path (`crates/core/src/paths.rs`,
`providers_path()`). `crates/core/src/config.rs` loads it through the shared
`clipforge_inference::Providers` registry and keeps the resolved
`ImageDeployment` on `AppConfig::image`; the LoRA root shown in Settings is the
provider's `lora_dir`.

```toml
[defaults]                       # capability → provider id
t2i = "qwen-image"
edit = "qwen-image"

[providers.qwen-image]
kind = "sdcpp"                   # the native sd.cpp server
architecture = "qwen-image-2.1"  # must be a profile the app serves
capabilities = ["t2i", "edit"]
base_url = "http://127.0.0.1:7861"
model_dir = "models"             # relative to this file (the repo root)
# lora_dir / executable default beneath model_dir.

# Override any artifact with a local file instead of downloading it.
[providers.qwen-image.weights]
diffusion = "/abs/path/to/qwen-image-2.1-UC-int8_convrot.safetensors"
```

`architecture` is validated against the served profile list: a provider may
change *where* a model comes from, not *which* model it is, and any other value
is rejected at load. Relative paths in the file (`model_dir`, `lora_dir`,
`executable`, and the `weights` entries) resolve against the file's directory.
Removing the file falls back to the built-in provider (the standard
Qwen-Image-2.1 tree under `models/`). `model_setup` provisions through this same
deployment: it downloads the artifacts whose `needs_download()` is true and
prints the local override it skips for the rest.

## Common issues

- **`cargo`/`dx` fail with path errors** — the `:` in the parent directory; the
  `.cargo/config.toml` target-dir redirect is the fix, do not remove it.
- **Generation says the runtime is not ready / job fails** — the `sd-server`
  binary or model artifacts are missing. Run `cargo run -p svs-cli --bin model_setup`
  and confirm the provider's `executable` in `providers.toml`.
- **`generation server on 127.0.0.1:7861 has … loaded; … needs the server restarted`** —
  the resident `sd-server` has a different model loaded than the requested
  profile. Ops restart their own server automatically; a stale server from
  another session needs `svs runtime serve --force` (or
  `pkill -f "sd-server --diffusion"`), then rerun.
- **LLM endpoint calls fail** — confirm the endpoint is running, the configured
  model is loaded, and `llm.base_url` matches its `/v1` endpoint.

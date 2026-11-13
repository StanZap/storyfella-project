# Development

This document covers setting up a machine and the day-to-day commands for the
Rust application.

For Studio UI changes, consult the [reusable component catalog](ui-components.md)
before creating another field, icon action, dialog or feedback surface.

The repository directory is `storyfella`, beside the separate `clipforge` and
`finewinetts` repositories. Studio and the visual Player use Dioxus desktop:

```sh
cargo build -p storyfella-player
cargo run -p storyfella-player -- /absolute/project.sf
```

Language/compiler/runtime/LSP responsibilities are documented in
[`architecture.md`](architecture.md#storyfella-workspace-boundaries), and
[playback reference](ROADMAP.md#story-execution-and-playback-reference) documents preview, Player inputs and fixtures. Existing CLI and user-data identities
remain unchanged. CI checks formatting, all workspace targets and tests, builds
CLI/Player, and builds and runs Studio's backend-free smoke plan through dx.
Plan runs keep created projects, generated artwork and cache under `.svs-e2e/`,
along with their hermetic config and logs. Normal launches retain user project
storage. If the Clipforge repository is private, configure the Actions secret
`CLIPFORGE_READ_TOKEN` with read access to `StanZap/clipforge`; the normal
workflow token only grants access to this repository. Local builds consume the
sibling checkout and do not need that secret.

## Prerequisites

- Rust stable
- The current stable Dioxus CLI (`dx`) — see <https://dioxuslabs.com>
- Node.js/npm (Tailwind CSS and CodeMirror toolchain, pinned in `apps/studio/package-lock.json`)
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

Atelier's GPUI X11/Wayland and font backends also require
`libfontconfig1-dev`, `libfreetype6-dev`, `libxcb1-dev`, `libxkbcommon-dev`,
`libxkbcommon-x11-dev` and `libwayland-dev`. The CI desktop dependency step
includes these packages now that Atelier is a workspace member. The moved app
has been validated on macOS; Linux runtime verification remains separate.

## Workspace layout

The repository is a Cargo workspace. `crates/studio-core` (package `storyfella-studio-core`) holds
the domain layer — the artifact registry, typed operations, persistence, and
the vision/LLM/runtime clients — and has **no GUI dependency**, so the CLI and
the core tests never compile the toolkit or the WebKit stack. `apps/studio`
(package `storyfella-studio`) is the Dioxus application and owns the
`desktop` feature; `apps/cli` (package `svs-cli`) builds the `svs`,
`model_setup`, and `vlm_probe` binaries. Both application crates depend on
`storyfella-studio-core` and name it explicitly (`storyfella_studio_core::…`) rather than re-exporting it.

`apps/player` (package `storyfella-player`) uses shared Dioxus presentation from
`crates/playback-ui`, the GUI-free host/resource adapter in Studio core and the
independent `crates/runtime` execution engine. It does not consume Studio state.

`apps/atelier` (package `storyfella-atelier`, binary `atelier`) is the independent
GPUI drawing and image-generation app. It uses Studio core only for portable
artwork exchange and has no Dioxus dependency.
Run it from the workspace root with `cargo xtask atelier`; use
`-- --data-dir .svs-e2e/atelier-dev` for isolated sessions. Use
`cargo xtask atelier bundle` to [package Atelier](#atelier-packaging).
Its Metal surface renderer uses the pinned patch in `vendor/gpui-pre-apple`;
keep that source and its licenses with the workspace. See [Atelier](atelier.md).

The native-inference runtime is **not** owned by this workspace: it is the
shared `agentkit-ai` crate at
`../../zerune_labs/agentkit/crates/agentkit-ai`. Native profiles, provisioning,
HTTP clients, provider resolution and lifecycle controls are exposed through
its `native-inference` feature. That checkout must be present; no Clipforge
crate or installed Clipforge application is required.

`apps/vision` was the Python vision runtime; it was removed with the Krea/SAM
migration — the project is pure Rust plus the native `sd-server`. `config/` — the
plan fixtures, the example provider config and the `config/plans` root marker —
stays at the workspace root; generated assets now default to the platform data
dir. `xtask/` is the developer task runner described below; nothing shipped
depends on it.

The payoff is a cheap domain loop: `storyfella-studio-core`'s dependency tree is 464 entries
with no `dioxus`/`webkit`/`gtk`/`tao`/`wry` in it at all, against 1183 for the
desktop crate, so

```sh
cargo test -p storyfella-studio-core                  # domain tests, no toolkit
cargo run -p svs-cli --bin svs -- --help
```

skip the GUI stack entirely. The workspace-wide `cargo check --workspace
--all-targets` / `cargo test --workspace` cover everything, including the
desktop crate's tests.

## Task runner (`cargo xtask`)

The recurring `dx` / `cargo` / `npm` invocations are wrapped by a
small task runner (`xtask/`, exposed through the `.cargo/config.toml` alias as
`cargo xtask`). It runs the same commands the sections below document, from the
workspace root, with every path resolved absolutely so it behaves the same from
any directory:

```bash
cargo xtask build                 # dx build --platform desktop (debug)
cargo xtask run [-r]              # build, then launch Dioxus Studio
cargo xtask atelier [-r]          # build, then launch the separate GPUI app
cargo xtask atelier --restart     # build, then safely restart the same data-directory instance
cargo xtask atelier -- --data-dir .svs-e2e/atelier-dev
cargo xtask atelier --restart -- --data-dir .svs-e2e/atelier-dev
cargo xtask atelier bundle        # macOS .app or Linux AppImage in dist/atelier/
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
project-root path resolution finds `config/plans`. `-l`/`--log` tees the
app's stdout/stderr to `.svs-e2e/logs/app-<unix>.log` for the GUI live-test
workflow. `bundle` writes to `dist/` (git-ignored) and lists each artifact.

`cargo xtask <command> --help` documents the flags. The sections below remain
the reference for the underlying commands, for when you need a flag the task
runner does not expose.

`atelier` uses `cargo run --package storyfella-atelier --bin atelier`, forwards
arguments after `--` and supports `-l`/`--log` with
`.svs-e2e/logs/atelier-<unix>.log`. It does not invoke Studio's Dioxus build or
the `svs` CLI.

### Atelier packaging

Run `cargo xtask atelier bundle` on the target platform. On macOS it builds
`dist/atelier/Atelier.app`, signs it ad hoc, and defaults to debug. Pass
`--release` for an optimized build.

On Linux, install the [desktop build dependencies](#linux-system-packages). The
task builds the executable with Cargo and packages it with
[cargo-packager](https://github.com/crabnebula-dev/cargo-packager) `0.11.8`,
which it installs into `target/tools/cargo-packager` the first time it is needed
— there is no manual `cargo install` step, and a matching install on `PATH` is
reused instead. The first AppImage build also downloads the AppImage packaging
tools, so that run needs network access and later runs reuse both.

```sh
cargo xtask atelier bundle                               # AppImage
cargo xtask atelier bundle --package-types appimage deb    # AppImage and Debian package
```

Linux packages go to `dist/atelier/` and default to release. Pass `--debug`
for a debug package. Use a build host compatible with the intended Linux
distribution and architecture. Atelier's native inference runtime and weights
are provisioned separately through the [provider setup](atelier.md#local-qwen-and-krea-on-linux-with-cuda).

## Build artifact directory

Build artifacts stay in the checkout's gitignored `target/`, selected by
`.cargo/config.toml`. Cargo also sets `TMPDIR`, `TMP` and `TEMP` to the ignored
`.cargo/tmp/` directory, so tests and inherited subprocesses keep temporary
files inside the checkout. Its tracked `.gitignore` creates it on a fresh clone:

```toml
[build]
target-dir = "target"
```

Keep task output local; do not set an external `CARGO_TARGET_DIR` or temporary
directory override. Use a checkout path without `:`: macOS/Linux interpret it
as a separator in dynamic-library path variables. See `AGENTS.md` for the
project-directory boundary and `.svs-e2e/` scratch layout.

## First-time setup

```bash
# Rust UI toolchain — run from apps/studio (which holds package.json)
cd apps/studio
npm install

# Compiled stylesheet: `assets/tailwind.css` is git-ignored generated output.
# `dx serve` compiles it automatically; for `cargo check`/`cargo test` on a
# fresh clone, generate it once (the asset macro embeds the file at compile time):
npm run css:build
cd ../..
```

### Native generation backend (`sd-server`)

The app spawns the `sd-server` binary the image provider's `executable` points
at (default `<model_dir>/runtime/stable-diffusion.cpp/bin/sd-server`, see the
[inference settings](#inference-settings) section below). The native server is **not**
part of this project: the sibling `clipforge` workspace owns the vendored
`stable-diffusion.cpp` checkout (git-ignored, pinned to the `3f8527a` revision
that added Qwen-Image-2.1 support) and builds it. From the `clipforge` checkout:

```bash
cd ../clipforge
cargo xtask sd-server --model-dir "${XDG_DATA_HOME:-$HOME/.local/share}/storyfella/models"
```

The task clones the pinned source, builds it with CUDA, and installs the binary
into the model tree — pass the Models folder shown in Studio Settings (the
example uses Linux's default). `--cuda-architectures` picks the compute capability: 86 for RTX
3090 (Ampere), 89 for RTX 40-series; the default is 86.

The launcher passes `--model-args qwen_image_2_1_prefix_cache_type=q8_0` and
`--diffusion-fa`, and points `--diffusion-model`/`--llm`/`--vae` at the
profile's three safetensors (see [`architecture.md`](architecture.md)).
Provision those weights with `model_setup` (see the CLI tools table below).

## Run the desktop application

Commands are run from `apps/studio` (where `Dioxus.toml` and `package.json`
live):

```bash
cd apps/studio && dx serve --platform desktop
```

`cargo xtask serve` runs the same command (and `cargo xtask run` builds once
and launches without the dev server).

Studio's continuous diff editor uses CodeMirror merge, bundled locally with
esbuild. Run `npm ci` in `apps/studio` after checkout or dependency updates.
The Studio Cargo build script runs `npm run editor:build` when editor source or
the npm manifests change, before `asset!` embeds the generated JavaScript.
No CDN or runtime package fetch is used. The generated `assets/writing-diff.js`
is ignored; edit `ui-js/writing-diff.js`, not the bundle. Rust still owns source
validation, application, persistence and workflow execution.

`dx` compiles `apps/studio/assets/tailwind.css` from `apps/studio/tailwind.css`
automatically. To build or watch the CSS independently:

```bash
cd apps/studio && npm run css:build
cd apps/studio && npm run css:watch
```

Most component styling is expressed as Tailwind utility classes in RSX, so
editing Rust usually requires no CSS work. The ignored `assets/tailwind.css`
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
types are `appimage` and `deb`. Run it from `apps/studio`:

```bash
# Release build + AppImage (prints `Created AppImage: <path>` when it lands)
cd apps/studio && dx bundle --platform desktop --package-types appimage -r

# One value per flag occurrence — `--package-types appimage deb` and
# `--package-types appimage,deb` are both rejected. Build several artifacts in
# one run: each run rebuilds into a cleaned bundle directory, so an appimage-only
# run deletes the .deb produced by the run before it.
cd apps/studio && dx bundle --platform desktop --package-types appimage --package-types deb -r
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
| `Name` in the desktop entry | a sanitised product name derived from `[application] name`; revalidate packaging for the renamed Studio |
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
  The repo has none — `apps/studio/assets/favicon.ico` and
  `apps/studio/assets/header.svg` came with the Dioxus scaffold and are
  referenced nowhere. With no icon configured, `dx` writes a default one, so
  the AppImage builds but is unlabelled.
- **The runtime payload.** `sd-server` is bundled with `[bundle] external_bin`
  (sidecar binaries) and/or `[bundle] resources` (data files); both keys are inert
  until the file exists. Shipping it also needs the code change named in item 11:
  resolve it relative to the bundle root instead of the provider's `executable`
  in `clipforge.toml`.

## Run the generation backend

An op starts its own `sd-server` when none is running, and that server dies when
the process exits (cold start per command). For a longer session, keep the
resident server in one terminal and let ops in other terminals reuse it:

```bash
cargo run -p svs-cli --bin svs -- runtime serve --force   # Ctrl-C to stop
```

`--force` kills stale `sd-server` processes from interrupted sessions that hold
the configured port (7861 by default in the shared `clipforge.toml`). Only the
`qwen-image-2.1` profile is served. See
[`architecture.md`](architecture.md).

## Tests

### Rust

```bash
cargo test --workspace
```

The workspace runs 149 tests in `storyfella-studio-core`, 78 in Studio, 3
in the CLI binaries, and 1 in xtask (231 total). They cover state transitions
(`apps/studio/src/state/`), the runtime adapter and health probes
(`crates/studio-core/src/runtime/`), and multimodal message shape
(`crates/studio-core/src/llm/`). They do not require a running `sd-server` or an LLM
endpoint.
The `sdcpp` wire serialization tests (request body + job decoding) live in the
shared `agentkit-ai` crate (`native::sd`) and
are not part of this workspace's run. `cargo xtask test` runs the same command.

### End-to-end: plans (the app's own regression suite)

Unit tests do not exercise the app. The automation driver (`apps/studio/src/driver/`)
does: it plays a **plan** — a YAML script written in the app's own vocabulary
— through the same code paths the UI uses (`apps/studio/src/commands.rs`), and asserts
outcomes against app state. Plans never mention selectors, coordinates or
DOM, so they do not break when the UI is restyled.

```bash
# Fast, backend-free: project, story, beats. Runs anywhere.
cargo xtask plan config/plans/smoke.yaml --mode check --log
```

`--mode check` runs the steps back to back and prints a verdict
(`verdict: 7 passed, 0 failed`, non-zero exit on failure) — this is the
regression mode. `--mode guide` runs the same plan slowly with
an assistant card narrating each step in the UI, which is the tutorial
experience and the better way to eyeball UX.

For a first look at the authoring UX, run:

```bash
cargo xtask plan config/plans/ui-tour.yaml --mode guide --log
```

Or open **Plans**, choose **Story authoring walkthrough**, select **Guide**,
and run it. The walkthrough creates a fresh sample project,
types its entire story from a blank Writer at 80 characters per second,
shows the Writer's resolved and pending references, opens saved artwork and
changes a character's story-facing description. Pause, resume, change speed,
or stop from the assistant card.
It ends in the Writer so you can try editing the story yourself.
Pictures and revision history are fixtures. No inference backend is needed.
Visual production belongs to Atelier; Studio plans cannot generate or edit images.

| Plan | Needs a backend | Covers |
| --- | --- | --- |
| `smoke.yaml` | no | Project creation, story and beat artifacts, registry state |
| `ui-tour.yaml` | no | Narrated authoring walkthrough: source, asset references, saved artwork, descriptions and palette |
| `playback.yaml` | no | Saved fixture playback, choices, bookmark and restore |

Writing a plan: steps are `verb: value` pairs, `expect_*` steps gate
progress, `soft: true` downgrades a failure to a warning (for example, an
optional vision endpoint), and `message:` is the
narration shown in guide mode. `command:` supports `/describe` and text-only
story/scene/beat creation. Visual operations are rejected when the plan loads,
before any step changes state. Core and `svs` retain the full operation vocabulary.
`type_story:` appends text in the Writer through its normal source setter,
keeping the caret and scope panel on the typed line. Guide mode animates typing;
`sample_project:` also types its complete source using the same controls.
The rate is `typing_cps` characters per second (default 60, allowed 1–200).
Unknown plan settings are rejected with their name, so a typo cannot silently
fall back to a different typing speed.
The assistant card's **Typing** slider changes that rate live, independently
of the narration's 1x/2x/4x controls. Pause stops typing and delays command
execution; stopping leaves the partial text available to inspect. Check mode
applies typed text immediately without the animation.

Two traps worth knowing before writing one. A bare `- wait_ms: 800` is **not**
a step — every step needs an action, and `wait_ms`/`message`/`soft` are
modifiers on one; serde reports that mistake against the first step in the
file rather than the offending one, so trust the message over the line
number. And a plan runs against the live app on a spawned task, sharing the
same state as the user: project-open/create steps
switch away from whatever project is open, and editing the story
mid-run can affect subsequent expectations. Guide mode's assistant
card has pause/stop for exactly this; check mode calls `std::process::exit`
when the report prints.

Runs are worth keeping — `.svs-e2e/logs/` is the gitignored home for plan
output and generation logs, and `.svs-e2e/quality/` for the images a run
produced. See `AGENTS.md` for why experiment scratch stays inside the
checkout rather than `/tmp`.

### End-to-end: the grounding suite (planner quality)

Planner quality is model-sensitive: the `modify`/`variant` grounding is a
subtraction task (restate the artifact's traits as invariants, minus the one the
edit changes), and a weaker planner either keeps the old trait or invents one.
Compare models through the Rust `svs` CLI using the same base image and edit
instructions, then judge outputs with `svs eval --expect`. Keep a separate
project copy per model so each starts from the identical revision. Configure
planner and vision providers in a scratch `clipforge.toml` under `.svs-e2e/` and
point `SVS_AI_CONFIG` at that file; retain the vision endpoint when overriding
the planner. Run the image backend with `svs runtime serve`.
Store outputs and judgments under `.svs-e2e/quality/` and `.svs-e2e/logs/`.
Scratch experiment scripts are local tools, not a shipped regression suite.
Overriding `model` selects the served model only for an `openai` provider; a
managed `llamacpp` provider serves `model_path`.

## CLI tools

| Command | Purpose |
| --- | --- |
| `cargo run -p svs-cli --bin model_setup` | Download, resume, and checksum-verify the Qwen-Image-2.1 profile (diffusion + text encoder + VAE) into the image provider's model tree |
| `cargo run -p svs-cli --bin vlm_probe -- --list-models` | List models visible through the LLM endpoint |
| `cargo run -p svs-cli --bin vlm_probe -- --model <id>` | Run the visual-LLM probe against one model |
| `cargo run -p svs-cli --example generate_vlm_fixtures` | Regenerate the deterministic probe fixtures |
| `cargo run -p svs-cli --bin svs -- --help` | Operation CLI (typed ops, stacks, runtime serve, log, eval) — see [`development.md`](development.md) for the command reference and the light/LLM/generation session split |

`model_setup` provisions into the image provider's model tree: the OS-local
Storyfella models folder by default, or the explicit `model_dir` override
in `clipforge.toml`.
The `svs` CLI needs no provisioned models for the model-only and
LLM-assisted tiers; only the visual ops (`create`/`variant`/`regenerate`/`modify`)
touch the generation backend.

## Configuration reference

Configuration is split by concern into two files, both loaded by
`crates/studio-core/src/config.rs`. **App config** — `[paths]`, `[generation]`,
`[generation.sizes]`, `[lora]` — lives in the platform
`studio.toml` (see the `app config` note below). **AI config**
— the `[defaults]` capability map and the `[providers.<id>]` registry — lives in
the machine's shared `clipforge.toml` (see the
[inference settings](#inference-settings) note below). Missing sections fall back
to defaults. The keys below are app settings unless the note says otherwise.

| Key | Default | Meaning |
| --- | --- | --- |
| `appearance.theme` | `dark` | Studio interface: `dark`, `light` or live OS-following `system`; applies without restart |
| `models.separate_vision` | Infer from configured providers and models | `false` makes Vision follow Text. `true` uses the separately selected Vision provider and model and shows their fields. When absent, Studio links identical selections and preserves distinct selections, including an unconfigured capability. |
| `generation.steps` | `25` | CLI default steps per generation (Qwen-Image-2.1 has no few-step turbo path) |
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

The **shared native-server state directory** (process markers and detached-server
logs) is not a `paths.*` key: it always resolves to
`storyfella_studio_core::ai_config::default_state_dir()` (on Linux
`~/.local/share/clipforge`), overridable with `$SVS_STATE_DIR` for tests and
hermetic `--plan` runs. It is the same directory Atelier and the Clipforge apps
use, so one resident server is shared.

Configured relative paths (`paths.*` and the plan directory) resolve against the
**project root**, not the working directory: `crates/studio-core/src/paths.rs` picks
`$SVS_HOME` first, otherwise the nearest ancestor of the working directory (then
of the executable) that holds the `config/plans` directory, so commands do not need
to be run from the repository root. The platform `studio.toml` is created with
`generation.sizes.character` = `1024x1536` (the full Qwen portrait canvas).

### App config

The appearance and Vision-mode preferences belong to `studio.toml`; provider
URLs, models, credentials and the separate Vision selection remain in the shared
`clipforge.toml`. Turning separate Vision off does not rewrite those provider
settings. Studio-core's effective `AppConfig::vision` follows Text while linked,
including text-provider/model/key changes; `svs eval` and guide vision checks
consume that same effective deployment. The selected Text model must accept
images in this mode. Re-enabling separate Vision restores its saved selection.

The app config is one platform-local file, `studio.toml`, in the platform
application config directory resolved by `directories::ProjectDirs`
(`paths::config_path()`) — on Linux `~/.config/dev.storyfella.studio/studio.toml`
(`$XDG_CONFIG_HOME`), on macOS
`~/Library/Application Support/dev.storyfella.studio/studio.toml`,
and on Windows `%APPDATA%\dev.storyfella.studio\config\studio.toml`.
`$SVS_USER_CONFIG` overrides the path, which is how tests and scratch roots keep
it out of the picture. There is no repository base file: when the platform config
directory exists and the file is absent, `AppConfig::load()` creates it, and every
key it does not set falls back to the built-in code defaults.

Studio uses its bundle identifier, `dev.storyfella.studio`, as the flat platform
namespace for configuration, data and cache. On first startup after the rename,
it copies existing application directories into absent destinations, retaining
originals as backups. It updates recent-project links, explicit settings paths,
stored artwork references and writing-session owner paths within those roots.
External project locations, source text, stable IDs and request provenance stay
unchanged. SQLite's backup API includes committed WAL data; separate WAL/SHM files
are not copied over a restored database. A failed root copy is not published and
can be retried. Existing destinations are never merged or overwritten.

Close Studio and Atelier before starting the rebuilt Studio, then reopen shared
projects from their new location in both apps. The old directories are backups;
ongoing edits there do not synchronize with the migrated copies. An explicit
`SVS_USER_CONFIG` bypasses this default-directory migration for isolated/custom
setups. Shared `clipforge.toml`, inference state, model storage and Atelier's own
application identity are separate and remain in their existing locations.

The created file is seeded so the character canvas is the full Qwen portrait:

```toml
# ~/.config/dev.storyfella.studio/studio.toml
[generation.sizes]
character = "1024x1536"
```

The Settings generation pickers write here, which keeps the per-machine
generation defaults. The file lives outside the checkout, so it is not in the
repository to begin with. On first run, legacy user settings in the platform
config directory — the unified `storyfella.toml`, or the older `app.toml` and
`providers.toml` — are split once into this file (the app sections) and the shared
`clipforge.toml` (the provider sections). The originals remain untouched for
recovery; the new files are authoritative once present, and an existing
`clipforge.toml` is never overwritten. `SVS_USER_CONFIG` overrides the app config
path.

### Inference settings

The **shared AI config** — the `[defaults]` capability map and the
`[providers.<id>]` registry — lives in the machine's `clipforge.toml`, resolved by
`storyfella_studio_core::paths::clipforge_config_path()` (via
`storyfella_studio_core::ai_config::default_config_path()`; Linux
`~/.config/clipforge/clipforge.toml`), overridable with `$SVS_AI_CONFIG`. It is the
machine's single provider registry, shared verbatim by Storyfella Studio, Atelier
and Clipforge, so a provider edit is visible to every front end. When the file is
absent, `AppConfig::load()` creates it with Storyfella's defaults and never
overwrites an existing one. `crates/studio-core/src/config.rs` validates it
through the shared `agentkit_ai::deployments::Providers` registry: the image backend —
the `sd-server` executable, model tree, LoRA root, endpoint, and weights — is
resolved to a provider and kept on `AppConfig::image`, and `text`/`vision` resolve
to `AppConfig::text`/`vision`. The CLI's LoRA root is the provider's
`lora_dir`.

```toml
# ~/.config/clipforge/clipforge.toml
[defaults]                       # capability → provider id
# The seed also sets text = "local" and vision = "vision".
t2i = "qwen-image"
edit = "qwen-image"

[providers.qwen-image]
kind = "sdcpp"                   # the native sd.cpp server
architecture = "qwen-image-2.1"  # must be a profile the app serves
capabilities = ["t2i", "edit"]
base_url = "http://127.0.0.1:7861"
model_dir = "/abs/path/to/models"  # the seed writes this absolute
# lora_dir / executable default beneath model_dir.

# Override any artifact with a local file instead of downloading it.
[providers.qwen-image.weights]
diffusion = "/abs/path/to/qwen-image-2.1-UC-int8_convrot.safetensors"
```

`architecture` is validated against the served profile list: a provider may
change *where* a model comes from, not *which* model it is, and any other value
is rejected at load. Relative paths in `clipforge.toml` (`model_dir`, `lora_dir`,
`executable`, and the `weights` entries) resolve against that file's own
directory, not the project root; the seed writes `model_dir` absolute under the
OS-local models folder. When no shared config is available (the platform exposes
no config directory), the app falls back to the built-in image provider (the
standard Qwen-Image-2.1 tree under the OS-native models directory). The `text`
and `vision` capabilities have no built-in default: when no provider serves one,
`AppConfig` leaves it `None` and the app **degrades** rather than failing to
start — the planner falls back to manual slash commands and `svs eval` reports
that no `vision` provider is configured. `model_setup` provisions through this
same deployment: it downloads the artifacts whose `needs_download()` is true and
prints the local override it skips for the rest.

Large model data uses `directories::BaseDirs::data_local_dir()` and the Storyfella
application folder, with the same policy in development and release builds:

| OS | Default model directory |
| --- | --- |
| macOS | `~/Library/Application Support/Storyfella/models` |
| Windows | `%LOCALAPPDATA%\Storyfella\models` |
| Linux | `$XDG_DATA_HOME/storyfella/models`, normally `~/.local/share/storyfella/models` |

Settings → Services → Model files offers **Change location…** and
**Open folder**. A selected folder is saved before the success message appears;
restart applies it consistently to downloads and the runtime. Downloads are
paused until that restart. Existing model files are not moved or deleted: choose
their model-tree folder to reuse them. An explicit executable, LoRA root or weight
override remains independent of the model folder. CLI provisioning reads the
same configuration as Studio. Configuration writes are staged atomically.

The same registry routes the LLM capabilities. `text` (the planner) and
`vision` (`svs eval`, future vision steps) each name a provider:

```toml
[defaults]
text = "local"
vision = "vision"

[providers.local]
kind = "openai"                  # external endpoint the app calls but never starts
capabilities = ["text"]
base_url = "http://localhost:8080/v1"
model = "prism-ml/bonsai-27b"

[providers.vision]
kind = "openai"
capabilities = ["vision"]
base_url = "http://localhost:8080/v1"
model = "google/gemma-4-e4b"
```

Two provider kinds serve chat: `openai` is an external OpenAI-compatible service
the app never starts (the default); `llamacpp` is a managed native `llama-server`
the app starts on demand from `executable` + `model_path` (plus `mmproj` for a
vision model). The kind is an internal detail — Studio presents text and vision
as the same card (provider, base URL, model, API key), and editing a managed
provider's base URL moves the server it launches. A managed server's
**Start/Stop/Restart** live on Settings → Services. Because text and vision are
separate capabilities, the planner can point at a remote, text-only model — for
example DeepSeek — while vision keeps a local multimodal endpoint. `api_key` is
an inline secret; prefer `api_key_env`, which reads it from the named environment
variable at load time.

Because the shared `clipforge.toml` is one file, a hand edit only needs the
keys it changes:

```toml
# ~/.config/clipforge/clipforge.toml
[defaults]
text = "deepseek"

[providers.deepseek]
kind = "openai"
capabilities = ["text"]
base_url = "https://api.deepseek.com/v1"
model = "deepseek-chat"
api_key_env = "DEEPSEEK_API_KEY"
```
This keeps the key out of the repository. The Settings **Models** page shows
the resolved endpoints and lets each chat capability's provider, base URL, model
and API key be edited in place, persisting to `[providers.<id>]` (or `[defaults]
text`/`vision` for a provider switch) in this file. Its **Providers** section
can also add a provider (validated as part of the registry before it is
written) or remove one the shared file defines.

## Common issues

- **`cargo`/`dx` fail with path errors** — check for `:` in the checkout's parent
  path; macOS/Linux library-path variables treat it as a separator. Use a
  checkout path without that character and keep build output in local `target/`.
- **Generation says the runtime is not ready / job fails** — the `sd-server`
  binary or model artifacts are missing. Run `cargo run -p svs-cli --bin model_setup`
  and confirm the provider's `executable` in `clipforge.toml`.
- **`generation server on 127.0.0.1:7861 has … loaded; … needs the server restarted`** —
  the resident `sd-server` has a different model loaded than the requested
  profile. The shared owner restarts a server it started automatically (Settings
  → Services → **Start**); a foreign server (a hand-run binary, no marker in the
  shared state dir) is stopped with `svs runtime stop --force` (or
  `svs runtime serve --force`), then rerun.
- **LLM endpoint calls fail** — confirm the endpoint is running, the configured
  model is loaded, and the relevant provider's `base_url` in `clipforge.toml`
  matches its `/v1` endpoint (planning uses `text`, `svs eval` uses `vision`).
  A managed `llamacpp` provider must have `model_path` (and `mmproj` for vision)
  on disk.

## Operation CLI reference

The current Studio registry operations are also available through `svs`. Commands
that generate images require a configured image deployment; source analysis and
playback do not start inference. Consult `svs --help` and subcommand help for the
complete current argument surface.

```sh
svs --project p.db project p.db                       # load or create (SQLite registry)
svs --project p.db op create character "Mia, a lighthouse keeper" --name mia
svs --project p.db op create scene "The kitchen at dusk" --name kitchen
svs --project p.db op create object "an antique brass lantern" --name lantern --size 768x768
svs --project p.db op compose c:<scene> "Mia lights the lantern" --background c:<env> --layer c:<char>
svs --project p.db op variant c:<char> "in rain gear" --axis outfit
svs --project p.db op regenerate c:<char> "make it warmer" --seed 42 --steps 25 --size 1024x1536 --out golden/
svs --project p.db op draft c:<story> "write the opening"          # LLM + approval checkpoint
svs --project p.db op draft c:<story> "write the opening" --text "…"   # manual, no checkpoint
svs --project p.db op modify c:<char> "give her a short bob haircut"   # instruction edit
svs --project p.db op describe c:<char> "Mia, a lighthouse keeper, short bob"  # make an edit canonical
svs --project p.db op revert c:<char>                                # re-activate the previous image
svs --project p.db stack run stack.json --approve auto             # the VLLM contract test bed
svs --project p.db stack propose "Add a rainy variant of the kitchen scene"   # LLM → JSON
svs --project p.db runtime serve --force                           # resident generation server
svs eval -i image.png -p "answer YES or NO: …"                     # LLM vision judge
svs --project p.db log [c:<ref>]
```

`create --size WxH` sets the artifact's default generation size (kind
defaults otherwise: character 512×768, object 768×768, environment/scene/beat
1024×576; the platform `studio.toml` is created with character at 1024×1536);
`regenerate`/`modify` honor it, and `--size`/`--steps` on those
commands override it per run. Variants inherit the base artifact's size.
`create`/`variant` of visual kinds generate the first image at that size as
part of the op (so they need the generation backend — see the table below);
variants use the base's active image as the reference.

`describe` rewrites an artifact's canonical description; `revert` re-activates
its previous completed image (the newest revision made before the current one).
Neither renders anything: both are registry mutations, so they need no
backend. `revert` never deletes — the revision reverted from stays in the
history and can be re-activated from the canvas.

`--approve auto|interactive` (default interactive) resolves checkpoints;
`--out <dir>` drops every intermediate image into a folder for human review —
the manual golden-run tier. `stack run` persists already-applied ops even
when a later op fails (fail-fast, intermediates kept).

## Vision probe output

`cargo run -p svs-cli --bin vlm_probe -- --help` lists model selection, endpoint,
fixture and output options. Keep new results in `.svs-e2e/` by passing
`--output .svs-e2e/vision-probe`; the current binary's default output location
still points to the former proof-of-concept directory. Probe fixtures are under
`apps/cli/tests/fixtures/vlm`. Results measure the selected models and fixtures,
not general visual reasoning or artwork fidelity.

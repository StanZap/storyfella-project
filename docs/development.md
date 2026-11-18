# Development

This document covers setting up a machine and the day-to-day commands for the
Rust application.

For Studio UI changes, consult the [component guide](ui-components.md) before
creating another field, icon action, dialog or feedback surface.

The repository directory is `storyfella`, beside the separate `clipforge` and
`finewinetts` repositories. Studio and the visual Player are GPUI apps built
on the shared design system in `../gpui-design-system`:

```sh
cargo build -p storyfella-player
cargo run -p storyfella-player -- /absolute/project.sf
```

Language/compiler/runtime/LSP responsibilities are documented in
[`architecture.md`](architecture.md#storyfella-workspace-boundaries), and
[playback reference](ROADMAP.md#story-execution-and-playback-reference) documents preview, Player inputs and fixtures. The `sf-cli` executable provides story tools and the project MCP server. CI checks formatting, all workspace targets, clippy and tests, builds
CLI/Player, and builds and runs Studio's backend-free smoke plan.
Plan runs keep created projects, generated artwork and cache under `.svs-e2e/`,
along with their hermetic config and logs. Normal launches retain user project
storage. If the Clipforge repository is private, configure the Actions secret
`CLIPFORGE_READ_TOKEN` with read access to `StanZap/clipforge`; the normal
workflow token only grants access to this repository. Local builds consume the
sibling checkout and do not need that secret.

## Prerequisites

- Rust stable
- An OpenAI-compatible LLM endpoint (llama.cpp / LM Studio), running separately, only when LLM features are exercised
- Linux desktop builds: the packages GPUI links (see below)

### Linux system packages

CI installs the set GPUI and the audio stack need:

```
libasound2-dev libudev-dev libglib2.0-dev libfontconfig1-dev libfreetype6-dev \
  libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev
```

## Workspace layout

The repository is a Cargo workspace. `crates/studio-core` (package `storyfella-studio-core`) holds
the domain layer — the artifact registry, typed operations, persistence, and
the vision/LLM/runtime clients — and has **no GUI dependency**, so the CLI and
the core tests never compile the toolkit. `apps/studio`
(package `storyfella-studio`) is the GPUI application; `apps/cli` builds the `sf-cli` story and MCP executable. Both application crates depend on
`storyfella-studio-core` and name it explicitly (`storyfella_studio_core::…`) rather than re-exporting it.

`apps/player` (package `storyfella-player`) draws the GPUI panel from
`crates/playback-panel` over the GUI-free playback model, stage composition and
resource adapter in Studio core (`playback::{PlaybackModel, stage}`) and the
independent `crates/runtime` execution engine. It does not consume Studio
state. Studio's Play view hosts the same panel.

Atelier, the GPUI drawing and image-generation app, is a separate repository
(`../storyfella-canvas`) that uses `storyfella-studio-core` by path. Its own
[development guide](../../storyfella-canvas/docs/development.md) covers running and
packaging it.

The native-inference runtime is **not** owned by this workspace: it is the
shared `agentkit-ai` crate at
`../../zerune_labs/agentkit/crates/agentkit-ai`. Native profiles, provisioning,
HTTP clients, provider resolution and lifecycle controls are exposed through
its `native-inference` feature. That checkout must be present; no Clipforge
crate or installed Clipforge application is required.

`config/` contains plan fixtures and the `config/plans` root marker.
`xtask/` is the developer task runner described below.

The payoff is a cheap domain loop: `storyfella-studio-core`'s dependency tree
has no GPUI in it, so

```sh
cargo test -p storyfella-studio-core                  # domain tests, no toolkit
cargo run -p sf-cli -- --help
```

skip the GUI stack entirely. The workspace-wide `cargo check --workspace
--all-targets` / `cargo test --workspace` cover everything, including the
desktop crate's tests.

## Task runner (`cargo xtask`)

The recurring `cargo` invocations are wrapped by a
small task runner (`xtask/`, exposed through the `.cargo/config.toml` alias as
`cargo xtask`). It runs them from the workspace root, with every path resolved
absolutely so it behaves the same from any directory:

```bash
cargo xtask build                 # cargo build -p storyfella-studio (debug)
cargo xtask run [-r]              # build, then launch Studio
cargo xtask run -l                # …teeing its output to .svs-e2e/logs/
cargo xtask plan config/plans/smoke.yaml --mode check
cargo xtask check                 # cargo check --workspace --all-targets
cargo xtask clippy [--deny]        # cargo clippy …; --deny errors on warnings
cargo xtask fmt [--check]         # cargo fmt --all
cargo xtask test                  # cargo test --workspace
cargo xtask doctor                # prerequisite tools + provisioned paths
cargo xtask clean                 # cargo clean
```

Debug is the default; `-r`/`--release` and `-d`/`--debug` override it, and
conflict rather than silently picking one. `run` and `plan` propagate the app's
exit code, so `cargo xtask plan … --mode check` gates a script.

`run` and `plan` launch `target/<profile>/storyfella-studio` from the workspace
root so the app's project-root path resolution finds `config/plans`.
`-l`/`--log` tees the app's stdout/stderr to `.svs-e2e/logs/app-<unix>.log` for
the GUI live-test workflow. Studio has no packaged bundle; run the binary.

`cargo xtask <command> --help` documents the flags.

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

## Run the desktop application

```bash
cargo run -p storyfella-studio
```

`cargo xtask run` builds and launches the same binary.

### Compile-only checks

```bash
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets
```

`cargo xtask check` and `cargo xtask clippy` are the same commands; the latter
takes `--deny` to fail on warnings, which is how CI runs it. Clippy is a
component of the pinned toolchain, so rustup installs it with Rust.

## Tests

### Rust

```bash
cargo test --workspace
```

The workspace tests cover the CLI, language, runtime, Studio and project
persistence. The managed writing-provider fixture runs a local fake worker;
no external model is needed. `cargo xtask test` runs the same command.

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
before any step changes state.
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

## CLI tools

`sf-cli story` checks, compiles and traces stories. `sf-cli mcp` exposes the
project authoring tools to an MCP client, including a bridge to an open Studio
Writer. Run `cargo run -p sf-cli -- --help` for the commands and see
[agentic production](agentic-vn-production.md) for MCP setup. Atelier owns
project artwork operations; Clipforge supplies general image inference tools.

## Configuration

Studio stores application preferences in `studio.toml` and reads configured
text and vision providers from the shared `clipforge.toml`. Select providers in
Studio Settings and manage provider configuration and credentials in Clipforge.
Studio uses its text provider for the writing workflow; the guide driver can
also use a vision provider for image checks.

`SVS_USER_CONFIG` and `SVS_AI_CONFIG` override the application and shared
provider configuration paths. `SVS_HOME` selects the resource root for local
plans. Portable story projects keep their own source, database and artwork.
See [the shared project format](shared-project.md).

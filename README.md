# Storyfella

Storyfella is the workspace for story authoring and playback. The existing
Dioxus application is **Storyfella Studio**; the `svs` CLI keeps its current
commands and behavior. Writer and Canvas share persistent scene/beat associations
and authoring tools with the catalog-backed LSP. Language, compiler and story
runtime are implemented; Studio preview and the standalone visual Dioxus Player
share deterministic execution, exact artwork bindings and controls. See
[workspace boundaries](docs/architecture.md#storyfella-workspace-boundaries).

```text
storyfella/
├── apps/studio/       # storyfella-studio (Dioxus)
├── apps/cli/          # svs-cli: svs, model_setup, vlm_probe
├── apps/player/       # storyfella-player (visual Dioxus desktop host)
├── crates/studio-core/ # storyfella-studio-core (no GUI)
├── crates/language/    # shared lexer, parser, AST and analysis
├── crates/compiler/    # versioned story plans, linking and flow validation
├── crates/lsp/         # stdio editor services
├── crates/runtime/     # deterministic execution and verified saved state
├── crates/playback-ui/ # shared visual presentation and controls
└── xtask/            # developer tasks
```

Clipforge and FineWineTTS remain separate sibling repositories. Storyfella
consumes `../clipforge/crates/inference`; the directory rename leaves that
relative dependency unchanged.

Check or compile `.sf` stories without loading models or opening a project DB:

```sh
cargo run -p svs-cli --bin svs -- story check examples/harbor.sf
cargo run -p svs-cli --bin svs -- story compile examples/harbor.sf
```

See [the language contract](docs/language.md) and [compiler contract](docs/compiler.md).
Asset linking and unsupported directions remain explicit readiness requirements;
use `--require-ready` to make either command fail when they are unmet.

Studio edits [authoritative `.sf` files](docs/story-files.md) beside each project
database. Projects → Open accepts `.sf` or `.db`; Writer offers Save copy and
Reload for resolving external edits.

Try basic playback with procedural fixture images, without inference services:

```sh
cargo run -p storyfella-studio-core --example playback_fixture
cargo run -p storyfella-player -- .svs-e2e/inputs/playback/lantern.sf
cargo xtask plan config/plans/playback.yaml --mode guide --log
```

The fixture stays under `.svs-e2e/`. [Playback documentation](docs/playback.md)
defines open inputs, fixed snapshots, keyboard controls and save/restore.
Voice, expressive media and portable packaging/export remain later work.

Storyfella Studio is a cross-platform Dioxus desktop application for planning, generating, arranging and previewing visual stories. The workflow includes authoritative source editing, the artifact canvas (characters, environments, objects, scenes with typed operations, variants, revisions), resident local Qwen-Image-2.1 generation, and sectioned runtime settings. The typed operation/planner layer is shared by the `svs` CLI and the canvas; source order drives scene/beat associations and basic playback.

The validated target is Linux with an NVIDIA GPU (CUDA). Image generation runs on a CUDA build of `stable-diffusion.cpp`'s `sd-server` serving Qwen-Image-2.1. The LLM endpoint is an external prerequisite and is never installed or bundled by this project.

This is intentionally a desktop-only application. Web, mobile, and WASM targets are not supported. There is no Python in the project: the old FastAPI vision runtime and its SAM/segmentation stack were removed with the Qwen migration.

## Architecture

- **Rust** owns the Dioxus UI, application state, the artifact registry, project persistence boundary, model-download boundary, directory resolution, process lifecycle, the LLM client, and the native generation client. Rust never links an ML framework.
- **`stable-diffusion.cpp`** (`sd-server`) is a separate native process owned by Rust that keeps the Qwen-Image-2.1 diffusion model resident. Rust talks to it directly over the server's async `sdcpp` HTTP API; it is not ComfyUI and there is no Python adapter in between.
- The **LLM endpoint** is accessed directly from Rust through its OpenAI-compatible `/v1/chat/completions` endpoint. The typed planner vocabulary (operations + pipelines) lives in `crates/studio-core/src/registry/`; `crates/studio-core/src/llm/` is the client only.

## Product surface

- **Projects** creates a fresh story and reopens paired `.sf`/`.db` projects.
- **Writer** preserves exact drafts, reconciles valid scene/beat declarations with
  Canvas identities, and supplies precise completion, navigation, diagnostics,
  resolved rename and conservative formatting.
- **Preview / Player** share still artwork, dialogue/thought/narration, choices,
  manual advancement, pause, restart, scene jump and verified save/restore.
- **Canvas** is the artifact workspace: a gallery grid at rest (variants stack
  behind their base, in-flight work dimmed and badged) and an image-dominant
  detail pane with an inspector rail, plus a slash-command composer
  (`/create`, `/variant`, `/regenerate`, `/modify`, `/describe` — the same
  typed vocabulary as the `svs` CLI). Undo/redo restores source and registry together.
- **Image generation** starts the resident native Qwen-Image-2.1 `sd-server` on
  demand, waits for the model to load, and leaves it resident for subsequent
  requests. `regenerate` and `modify` (edits described in plain language) run
  through the same pipelines as the CLI.
- **Variants** are first-class artifacts (`c:mia-outfit`), created with the
  `/variant` command or the detail-pane action; axes (outfit, weather, …)
  are tags for organization.
- **Settings** separates service status, general behavior, LLM endpoint
  configuration, image generation, and storage. A Status section probes the
  LLM endpoint and the native generation backend on demand, reports readiness
  and model residency, and can start or stop the resident runtime. The
  Intelligence section discovers the endpoint's model list into a searchable
  planner picker (`SearchSelect` — filter, arrows, Enter; selection applies
  immediately and `config/app.toml` holds the default).

The shell deliberately uses a small reusable component vocabulary, low-contrast boundaries, and progressive disclosure instead of a dense card dashboard. Styling is primarily Tailwind CSS 4 utility classes in RSX, with a small shared CSS layer for global browser behavior and form controls.

### Current creative workflow

1. Create artifacts with the composer: `/create character "Mia, a lighthouse keeper" --name mia`, `/create environment "a storm-tossed harbor" --name harbor`, `/create object "an antique brass lantern" --name lantern`.
2. Iterate versions: `/variant c:mia "in yellow rain gear" --axis outfit` — variants appear under the base artifact and can be selected like any artifact.
3. Generate images (Qwen-Image-2.1 provisioned): `/regenerate c:mia "make it warmer"` — the result is a new revision of the active image.
4. Edit an image: `/modify c:mia "give her a short bob haircut"` — say what to change in plain language. The generation model is an instruction editor, so it keeps what you did not mention (face, pose, clothing, background); there is no region to select.
5. Keep an edit: `/describe c:mia "Mia, a lighthouse keeper, short bob"` — edits produce revisions but never change what the artifact *is*, so the next `/modify` re-anchors to the description and an earlier change washes out. `describe` promotes a change to canonical; `/variant` turns one into its own artifact.
6. Browse revisions in the detail pane; click any completed revision to show it; undo/redo in the header.

Because `modify` re-states the artifact's description as the list of things to hold constant, **descriptions are worth writing carefully** — they are the edit's contract. An edit that contradicts the description ("long dark hair" + "dye it auburn") is reconciled by the configured LLM planner; without one, a deterministic fallback keeps the description verbatim and the contradiction can show up in the image.

The configured defaults favor Qwen's single quality profile — 25 steps, CFG 6.0, `euler`, and a 1024×1536 character canvas in this checkout. Qwen-Image-2.1 has no few-step turbo path, so iteration is slower than the retired Krea Turbo loop; the settings are editable per kind in Settings → Image generation.

## Project layout

```text
Cargo.toml            Virtual workspace manifest ([workspace] members only)
.cargo/config.toml    checkout-local build output and tool temporary directories
crates/studio-core/          storyfella-studio-core — the domain layer (no GUI dependency):
  src/registry/         Artifact registry, typed ops + pipelines, slash parser,
                        CreativeBackend
  src/persistence/      SQLite project store (versioned migrations)
  src/story.rs          Exact .sf source, disk snapshots and save conflict checks
  src/authoring.rs      Source associations and shared asset catalog
  src/playback.rs       Snapshot compilation and pinned resource checks
  src/llm/              OpenAI-compatible LLM client
  src/runtime/          CreativeRuntime — adapter over the shared
                        clipforge-inference crate (path dependency)
                        (sd-server lifecycle, sdcpp client, model store live
                        in ../clipforge/crates/inference)
  src/errors.rs         Error-code registry
  src/config.rs         Configuration loading
  src/paths.rs          Project-root path resolution
apps/studio/         storyfella-studio — the Dioxus application:
  src/app/              Dioxus root
  src/ui/               Desktop UI components (canvas, writer, projects, settings)
  src/state/            AppState: registry, selection, undo/redo
  src/commands.rs       Shared action language (slash parse → execute)
  src/driver/           YAML plan driver (Guide/Check modes)
  Dioxus.toml, tailwind.css, assets/, package.json
apps/cli/             svs-cli — src/bin/{svs,model_setup,vlm_probe}.rs
apps/player/          storyfella-player — standalone visual Dioxus desktop host
crates/language/      Shared front end and authoring services
crates/compiler/      Versioned plans, linking and flow validation
crates/lsp/           Catalog-backed stdio editor tooling
crates/runtime/       Independent deterministic story execution
crates/playback-ui/   Shared Dioxus presentation and controls
xtask/                cargo xtask — the developer task runner (build, run, bundle,
                      check, test); not part of any shipped artifact
config/               app.toml + plans/*.yaml (data, at the workspace root)
assets/generated/     Dev runtime data written by generations; project databases
                      store absolute asset paths, so this stays at the root
docs/ models/ cache/ .svs-e2e/
```

`crates/studio-core` never depends on the GUI; `apps/studio` and `apps/cli` both depend on `storyfella-studio-core`, and the desktop crate names `storyfella_studio_core::…` explicitly rather than re-exporting it.

Self-resolved paths anchor to a **project root** instead of the working directory (see `crates/studio-core/src/paths.rs`): `$SVS_HOME` wins, otherwise the nearest ancestor of the working directory (then of the executable) holding `config/app.toml`. Relative paths in `config/app.toml` resolve against that root, and unset values fall back to platform-appropriate application directories via the Rust `directories` crate. A user-level `app.toml` in that platform config directory (`paths::user_config_path()`) is layered over the checked-in file for machine-local secrets and overrides.

## Documentation

In-depth material lives in [`docs/`](docs/README.md): a system [architecture overview](docs/architecture.md) (including the Rust → `sd-server` `sdcpp` contract), the [developer guide](docs/development.md), the [runtime lifecycle](docs/runtime-lifecycle.md), and the [data model and persistence format](docs/data-model.md). Proof-of-concept notes are indexed in [`docs/poc/`](docs/poc/README.md).

## Prerequisites

- Rust stable
- The current stable Dioxus CLI (`dx`)
- Node.js/npm (for the pinned Tailwind CSS 4 toolchain)
- An LLM endpoint (llama.cpp `llama-server` or LM Studio), running separately when LLM features are exercised
- A CUDA toolchain and an NVIDIA GPU to build and run `sd-server`
- Linux desktop builds: the system packages required by Dioxus/WebKitGTK

## Set up the generation backend

`sd-server` is built by the sibling `clipforge` workspace, which owns the
vendored `stable-diffusion.cpp` checkout (git-ignored, pinned to the `3f8527a`
revision that added Qwen-Image-2.1 support). From the `clipforge` checkout,
build it and install it into the tree this repo's `providers.toml` names:

```bash
cd ../clipforge
cargo xtask sd-server --model-dir ../storyfella/models
```

Provision the Qwen-Image-2.1 weights (diffusion, text encoder, VAE) from
Hugging Face:

```bash
cargo run -p svs-cli --bin model_setup
```

## Run the desktop application

Install the frontend toolchain once (from `apps/studio`, which holds `package.json`):

```bash
cd apps/studio
npm install
```

The `tailwind.css` file in `apps/studio` selects Tailwind CSS 4 and lets `dx` automatically compile `assets/tailwind.css`. Most component styling is expressed as Tailwind utility classes in RSX. To build or watch CSS independently, use `npm run css:build` or `npm run css:watch`.

```bash
cd apps/studio && dx serve --platform desktop
```

The workspace also carries a task runner for the common commands — `cargo xtask
run` (build and launch), `cargo xtask bundle` (release AppImage into `dist/`),
`cargo xtask test`, and more; see [`docs/development.md`](docs/development.md).

For a compile-only check:

```bash
cargo check --workspace --all-targets
```

This checkout keeps build artifacts in gitignored `target/` through `.cargo/config.toml`. Cargo, tests and inherited subprocesses use ignored `.cargo/tmp/` for temporary files. Keep task scratch files and desktop-plan projects under `.svs-e2e/`. Use a checkout path without `:`, which macOS and Linux treat as a separator in dynamic-library path lists.

Configuration lives in `config/app.toml`. The default LLM endpoint URL is `http://localhost:1234/v1`; update `model` to the identifier loaded in the endpoint. `api_key` is optional.

## How generation runs

Rust spawns the native `sd-server` on demand and talks to it directly over the
`sdcpp` API: `POST /sdcpp/v1/img_gen` submits an async job, `GET
/sdcpp/v1/jobs/{id}` polls it, and `GET /sdcpp/v1/capabilities` reports the
resident model. Rust decodes the completed job's image and stores it under
`assets/generated/`. There is one generation profile — Qwen-Image-2.1 — so
there is no profile switching. See
[`docs/runtime-lifecycle.md`](docs/runtime-lifecycle.md) for the launch line
and readiness rules.

Only one model fits on a 24 GiB card: generation peaks at ≈23.4 GiB, so the
LLM planner must not be resident during a generation.

## Visual LLM probe

The isolated proof of concept exercises vision-capable models through the LLM endpoint's OpenAI-compatible chat-completions API. It uses deterministic synthetic fixtures, structured JSON output, per-request latency and token capture, and machine-readable results. See [`docs/poc/vlm/README.md`](docs/poc/vlm/README.md) for commands and scoring limitations.

## Native generation

The product-oriented native path serves Qwen-Image-2.1 (int8 convrot
safetensors) through `stable-diffusion.cpp`, with explicit request-time LoRAs
and asynchronous jobs — no ComfyUI, no Python adapter. See
[`docs/architecture.md`](docs/architecture.md) and
[`docs/runtime-lifecycle.md`](docs/runtime-lifecycle.md) for the `sdcpp`
contract and the launch line; the earlier Krea 2 native validation is recorded
(historically) in
[`docs/poc/native-generation/README.md`](docs/poc/native-generation/README.md).

## Visual evaluation (`svs eval`)

The local LLM endpoint's vision model doubles as an automated image reviewer —
"eyes" for evaluating generations, edits, and regressions without a
human at the screen. `svs eval` attaches images, asks questions, prints the
model's verdict, and can gate a script's exit code on the answer:

```sh
# Judge an instruction edit: is identity preserved?
svs eval -i base.png -i edited.png \
  -p "Answer with ONLY YES or NO: is image 2 the same person as image 1, \
      with only the described change applied?"

# Gate a test script on the verdict (exit 1 when the answer lacks "yes")
svs eval -i image.png -p "Are there visible artifacts?" --expect-not "yes"

# Multiple questions, one round each (all images attached to every round)
svs eval -i a.png -i b.png -p "Same person in both?" -p "Which is sharper?"
```

The default model is the configured planner model, which must have vision
support. Images are downscaled to a 1024px maximum before sending (the endpoint
rejects oversized payloads), and the default token budget is 2048 because the
model reasons before answering. The `--expect`/`--expect-not` checks are
case-insensitive substrings — craft prompts that elicit a strict YES/NO to gate
reliably.

## Roadmap

The ordered backlog with statuses lives in `docs/ROADMAP.md` §12
(this is the tracker). Upcoming work in brief:

1. Connect free-form Canvas messages to LLM proposals (the vocabulary
   exists; `svs stack propose` is the contract test bed) and add
   `c:` autocomplete.
2. Add the interactive-first job scheduler and richer timeline/media composition.
   Authored scene/beat order and basic preview/Player execution are implemented.
3. Add versioned project persistence improvements, generated-asset indexing, and autosave polish.
4. Wire model-download progress/cancellation, LoRA discovery, and runtime installation into Settings.
5. Complete warm-latency and peak-memory gates for Qwen-Image-2.1 on the target hardware.
6. Add evaluator metrics, API integration tests, and macOS/Linux platform CI coverage.

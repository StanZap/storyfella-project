You are working on **Smart Visual Sequencer**, a desktop-only Dioxus 0.7 application for planning, generating, and arranging visual stories. Rust owns the UI, state, models, persistence, process lifecycle, the native image-generation client, and the LLM client. Image generation runs through a native `stable-diffusion.cpp` process serving **Qwen-Image-2.1**; Rust talks to it directly over its `sdcpp` HTTP API. There is no Python in this project.

Current shape: the artifact registry + typed operation layer (`crates/core/src/registry/`), the SQLite project store, the native generation runtime (`crates/core/src/runtime/`, adapting the shared `clipforge-inference` crate; its image deployment is resolved from `providers.toml`), and the canvas are implemented and driven by the `svs` CLI and the desktop app (see `# Current status` below). Read `docs/ROADMAP.md` §12 (the feature/status tracker) before planning changes.

# Layout

A Cargo workspace: four packages.

| Path | Package | Owns |
| :--- | :--- | :--- |
| `crates/core` | `svs-core` | the domain layer — artifact registry and typed ops (`registry/`), `persistence/`, `llm/`, `runtime/` (the `CreativeRuntime` adapter over the shared `clipforge-inference` crate), `errors.rs`, `config.rs`, `assets/`, `paths.rs`. **No GUI dependency** |
| `apps/desktop` | `smart-visual-sequencer` | the Dioxus app — `ui/`, `state/`, `commands.rs`, `driver/`, `app/`, plus `Dioxus.toml`, `tailwind.css`, `assets/`, `package.json` |
| `apps/cli` | `svs-cli` | `svs`, `model_setup`, `vlm_probe`, the VLM fixture example and its fixtures |
| `xtask` | `xtask` | `cargo xtask` — the developer task runner (build/run/bundle/check/test); not part of any shipped artifact |

`crates/core` never depends on the GUI. `apps/desktop` and `apps/cli` both depend on
`svs-core`, and the desktop crate names `svs_core::…` explicitly rather than re-exporting
it, so the boundary stays visible at every call site.

### The shared inference runtime (`clipforge-inference`)

The native-inference runtime is **not** owned by this workspace — it is the
sibling `clipforge` project's crate, package **`clipforge-inference`**, pulled in
as a **path dependency** (`clipforge-inference = { path = "../clipforge/crates/inference" }`
in the root `[workspace.dependencies]`, used here as `clipforge-inference.workspace = true`).
It owns the served profile (`sd::Profile` / `sd::qwen_image_2_1()`), the resident
`sd-server` process (`sd::Runtime`), the `sdcpp` HTTP client (`sd::Client`,
`sd::GenerateRequest`, `sd::LoraSelection`), the LoRA top-level symlinking
(`sd::prepare_lora_links`), model provisioning (`ModelStore`), and the sampling /
layout defaults (`sd::DEFAULT_STEPS`, `sd::DEFAULT_CFG_SCALE`,
`sd::DEFAULT_SAMPLE_METHOD`, `sd::DEFAULT_MODEL_DIR`, `sd::DEFAULT_EXECUTABLE`,
`sd::DEFAULT_LORA_DIR`, `sd::DEFAULT_BASE_URL`).

What storyfella keeps is the **adapter**: `crates/core/src/runtime/`
(`CreativeRuntime` binds the shared `sd::Runtime` + `sd::Client` to `AppConfig`,
waits on jobs via `Client::wait`, imports generated assets, and maps the shared
errors into `CodedError`), the storyfella-owned `config/` + `config.rs`, and the
`registry/pipeline.rs` `GenerateResponse` that the pipeline consumes.

**Build implication:** the sibling checkout must be present at the relative path
`../clipforge/crates/inference` — storyfella does not build without it. The
crate is **not** a member of this workspace, so `cargo`/`dx` resolve it from the
sibling tree.

**The app's image deployment is a provider.** The `sd-server` executable, model
tree, LoRA root, endpoint, and weight overrides come from `<root>/providers.toml`
(anchored by `paths::providers_path()`, so never working-directory-relative),
loaded with `clipforge_inference::Providers::load_or_built_in(…)` and resolved
with `image_deployment(Capability::T2i)` into an `ImageDeployment` kept on
`AppConfig::image`. `CreativeRuntime`, the health probe, `model_setup`, and the
Settings screen all read that deployment; the LoRA discovery directory is the
provider's `lora_dir`. A provider may change *where* a model comes from, not
*which* model it is: `architecture` must be a served profile.

## Where paths come from

`crates/core/src/paths.rs` anchors every path the app resolves for itself — the config
file, the plan directory, and the configured `paths.*` directories — to one **project
root**: `$SVS_HOME` if set, otherwise the nearest ancestor of the working directory (then
of the executable) holding `config/app.toml`. Relative values in `config/app.toml`
resolve against that root, so the GUI, the CLI and the plans work from any directory.
Never reintroduce a working-directory-relative path or a `parents[N]` walk.

# Project rules

- **Desktop only.** Never add web, mobile, WASM, routing, or fullstack features. The `apps/desktop/Cargo.toml` features are limited to `default = ["desktop"]` / `dioxus/desktop`. Ignore any Dioxus docs about routers, server functions, or hydration.
- **No stale/dead code; no backward-compatibility shims.** We are pre-1.0 and not in production — formats, models, and APIs change freely. When a change replaces an older shape, remove the old one **in the same change**: no deprecated aliases, no "keep for old files" parse/import paths, no unused fields, no `#[allow(dead_code)]` for hypothetical future use, no leaving a superseded implementation around "for reference". A one-time import of a legacy format is sanctioned only while real files exist that need it (and must be removed once they're migrated or abandoned — SQLite schema migrations are the way to evolve the current format in place). Deferred vocabulary is allowed only when a roadmap item explicitly names it (e.g. slice-1 pipeline steps marked "vocabulary, not executable"); mark it with a doc comment naming the deferred item, never with `allow(dead_code)`. Concrete implication: when the canvas (roadmap item 4) replaces the storyboard model, delete `StoryboardFrame`/`project_json` rather than keeping them alongside.
- **Reusable components before repetition.** Isolate repeated patterns into small shared components/modules and reuse them everywhere. The UI vocabulary in `apps/desktop/src/ui/components.rs` (`SettingRow`, `EmptyVisual`, `StatusDot`, …) and `apps/desktop/src/ui/icons.rs` is the model; apply the same instinct outside the UI (registry ops, pipeline steps, persistence helpers, config). When a new use case needs a variant, extend the shared component instead of copy-pasting a near-identical one, and keep similar use cases looking and behaving the same. A new control for a case an existing control already covers is a code smell.
- **No perl for code edits.** Never use `perl -i` (or similar opaque one-liner rewrites) to modify source files — they are hard to review and have corrupted code before. Make edits with the file editing tools (`edit_file`/`write_file`), which produce reviewable diffs; `cargo fmt` is the only bulk formatter allowed.
- **Rust talks to the generation backend over HTTP.** The native `sd-server` client lives in the shared `clipforge-inference` crate (`clipforge_inference::sd::Client`, with `GenerateRequest` and the job types) and is consumed through `CreativeRuntime`; do not re-implement it here. It speaks the server's `sdcpp` API (`POST /sdcpp/v1/img_gen`, `GET /sdcpp/v1/jobs/{id}`, `GET /sdcpp/v1/capabilities`). Keep the request/response types in sync with `stable-diffusion.cpp/examples/server/api.md` (the checked-out revision is authoritative). There is no Python in this project — do not reintroduce one.
- **No ML frameworks in Rust.** Rust must never depend on or know about PyTorch, Transformers, or any other ML framework. Inference is the native `stable-diffusion.cpp` process; Rust is only its HTTP client.
- **The LLM endpoint is external** and never bundled or started by the app; `crates/core/src/llm/` is the client only. The planner vocabulary (typed operations + pipelines) lives in `crates/core/src/registry/`; LLM steps are soft dependencies that degrade to manual input, never hard-fail.
- **UI styling** is Tailwind CSS 4 utility classes in RSX (`apps/desktop/tailwind.css` is generated output; regenerate with `npm run css:build` from `apps/desktop`). Prefer the small shared component vocabulary in `apps/desktop/src/ui/components.rs` (`SettingRow`, `EmptyVisual`, `StatusDot`, …) and icons from `apps/desktop/src/ui/icons.rs`.
- **State mutations** go through `AppState` methods in `apps/desktop/src/state/mod.rs`, which maintain the registry invariants documented in `docs/data-model.md`. Never mutate `Project` fields directly from UI code.
- **The repository path contains a literal `:`.** Build artifacts are redirected to `/tmp/smart-visual-sequencer-target` by `.cargo/config.toml`; never remove that file.
- **Build with `dx build --platform desktop` for runnable binaries, not plain `cargo build`.** Run `dx` from `apps/desktop`, where `Cargo.toml` and `Dioxus.toml` live. The `asset!` pipeline (Tailwind CSS + custom protocols) is wired by the dx CLI: a plain `cargo build` binary starts fine but renders **unstyled** (no CSS — the stylesheet is never embedded/served). `cargo check`/`cargo test` are workspace-wide (`--workspace`); only launching a built app needs the dx artifact (`/tmp/smart-visual-sequencer-target/dx/smart-visual-sequencer/debug/linux/app/smart-visual-sequencer` after `dx build`). `cargo xtask build`/`run`/`serve`/`bundle` (and `check`/`test`/`fmt`/`clippy`) wrap these exact commands from the workspace root — prefer them over retyping the invocations.
- **Validation logs and scratch files stay in the repo** under `.svs-e2e/` (gitignored). When live-testing the GUI, write its stdout/logs to `.svs-e2e/logs/` — never to `/tmp` or the repo root. When killing a stale app instance, use a bracket-quoted pattern so `pgrep`/`pkill` do not match their own command line (e.g. `pkill -f "smart-visual-sequence[r]"`).
- **The vision evaluator — the agent's eyes.** The agent cannot see images; when judging generations, edits, or regressions, ask the LLM endpoint's vision model through `svs eval -i <image>… -p "<question>"` (default model `google/gemma-4-e4b`, served by llama.cpp or LM Studio). Images are auto-downscaled to 1024px; default max-tokens 2048 (the model reasons before answering, and 512 was exhausted by reasoning). `--expect <substring>` / `--expect-not <substring>` gate the exit code for scripted checks — craft prompts that elicit strict YES/NO. Comparison workflow: attach the base first, then the candidate(s); the model refers to them as image 1, image 2, …. Common checks: identity preservation across an edit or variant, artifacts/distortion in a generation. Evaluations used to drive a decision should be recorded in `.svs-e2e/logs/`.
- Consult the documentation below before making changes; many decisions and known gaps are recorded there.

# Documentation

- `docs/ROADMAP.md` — product design and **the planned-feature status tracker (§12)**: every feature to ship, with its status. Read §12 first to orient; update its statuses when work lands.
- `docs/api-slice-1.md` — implementation record of the API slice: settled decisions, module map, `svs` CLI reference, session guide (which ops need the generation backend).
- `docs/architecture.md` — system boundaries, component map, the Rust → `sd-server` `sdcpp` contract, prompt-to-revision data flow.
- `docs/development.md` — setup, build/run/test commands, `config/app.toml` reference, troubleshooting.
- `docs/runtime-lifecycle.md` — process supervision, readiness, model provisioning, residency.
- `docs/data-model.md` — domain types, state invariants, persistence format, known gaps.

# Current status

The API slice (`docs/ROADMAP.md` §12 items 1–3), the canvas (item 4), the
generation quality controls (item 5), the LoRA registry (item 6), and the
error-code registry (item 7) are implemented and validated:

- `crates/core/src/registry/` — artifact registry (kinds, variants, scenes/beats + layers, revisions, drafts, LoRA entries), typed slice-1 operations (`create`, `variant`, `regenerate`, `compose`, `draft`, `modify`) + LoRA ops (`attach-lora`, `lora-scale`, `lora-remove`), pipeline builder (closed step vocabulary — `LoadImage`, `Checkpoint`, `Generate`, `Llm`; typed handles, static validation at `build()`, linear fail-fast stacks, checkpoints), live backend (`CreativeBackend`), slash-command parser (`slash.rs`) for the canvas composer.
- `c:<name>` references are primary (case-insensitive exact match, ambiguity rejected; UUID/8-hex fallbacks).
- `crates/core/src/persistence/` — SQLite project store (`ProjectDb`): §10 schema (v3 dropped the legacy `project_json` column; v8 dropped the `masks` table with the mask edit path), versioned migrations (`schema_meta`), WAL, lossless round-trip (including the `lora_registry` table); the CLI persists to `.svs-project.db`. No legacy formats are parsed — the current format evolves via migrations.
- `apps/desktop/src/ui/` — screen tabs across the top (Story, Canvas, Projects, Plans, Settings), a status bar below. The **Canvas** is the artifact workspace in two states: `gallery.rs` is the grid at rest (variants stack behind their base, in-flight work stays in place dimmed and badged), and opening one gives an image-dominant detail pane with `inspector.rs` as its rail. The **Writer** (`writer.rs`) edits the story text with the artifacts it names resolved beside it; an unresolved reference is *pending*, not an error. `cmdbar.rs` is the tokenized command bar — highlighting, completion and signature help over `crates/core/src/registry/completion.rs` — and `palette.rs` (Ctrl+K) is a third view of the same vocabulary, so every inspector control is also a command. `shortcuts.rs` is the one keyboard table that tooltips, the status bar and the handlers all read; `keys.rs` is the single key listener. The workspace saves via autosave (after every change / every minute / off) or Cmd/Ctrl+S; `AppState.project_path` tracks the open database, `AppState.story_source` the story text. The legacy storyboard model (`Project`/`StoryboardFrame`/`Timeline`) was deleted with the canvas, and the kind-grouped sidebar with the grid.
- `svs` CLI — `op` (incl. `--lora <file>@<mult>`, `attach-lora`/`lora-scale`/`lora-remove`), `stack run`/`propose`, `runtime serve --force` (Qwen-Image-2.1 profile only), `log`, `project`; `--out` golden runs; `--approve auto|interactive`; `eval` (vision evaluation through the LLM endpoint — see the project rule on the vision evaluator). There is no `segment` subcommand.
- **Generation quality controls** (`docs/ROADMAP.md` §12 item 5): per-kind default sizes + steps are configurable (`config/app.toml` `[generation.sizes]`/`steps`) and editable in Settings → Image generation (persisted line-preserving); a live `Signal<GenerationDefaults>` (Workspace context) drives the canvas ops. The LLM sets size/steps/adapters only when the user explicitly asks (visual ops carry `size`/`steps`/`loras`; the `resize` op persists a new default size and regenerates at it). Free-form planning runs a tool-calling loop (`list_artifacts`/`list_loras`) with a strict schema on the final stack, falling back to the static planner when `tools` are unsupported. `variant --name` supports explicit naming.
- **`modify` is an instruction edit, and every instruction is grounded.** The artifact's active revision is the reference image and the grounded plain-language instruction is the prompt — no mask, no segmentation, no composite. A terse instruction under-specifies the render and collapses into base-model priors — measured, `"make him smile"` returned an *anime illustration of a different person*, twice, at two aspect ratios. `ground_instruction` (ops.rs) restates the medium and the artifact's traits as invariants; when an LLM endpoint is configured, `llm_ground` refines it by dropping the attributes the edit overrides (restating both "long wavy blonde hair" and "a bob cut" makes the model split the difference). The LLM is a soft dependency here: it never blocks, it only sharpens the deterministic fallback. **The planner model matters**: this subtraction is model-sensitive — `gemma-4-e4b` kept "long dark hair" while the edit asked for auburn and in one phrasing invented a colour that appeared nowhere in the input; `bonsai-27b` updated the trait and handled the haircut and outfit cases too. `config/app.toml` defaults to the larger planner. **Grounding is not modify-only**: a variant description and an explicit regenerate prompt are the same terse-delta shape and drift the same way — an ungrounded variant of a woman in her forties returned a different person of a different sex. Both ground too (a variant against its *base*); a regenerate with no prompt does not, since it replays an already-grounded prompt. **Grounding anchors on the artifact description, not the current revision**, so chained edits re-anchor to the description and earlier edits wash out (create dark → recolour auburn → haircut came back dark). That is deliberate — an artifact has one canonical look — and `describe` is the op that promotes an edit to canonical. `/variant` forks a look into its own artifact instead; variants cannot nest.
- **The native backend (Qwen-Image-2.1)**: `sd-server` is launched with `--diffusion-model`/`--llm`/`--vae` pointing at the profile's three safetensors (diffusion, text encoder, VAE) — or at the local file a `providers.toml` `weights` override names — plus `--model-args qwen_image_2_1_prefix_cache_type=q8_0` (an explicit quantized prefix cache keeps the working set small; flash attention is on via `--diffusion-fa`). It has **no few-step turbo path** — the default is CFG 6.0 at 25 steps with the `euler` sampler. A reference edit sends the image as the first `ref_images[]` entry (base64). **VAE tiling must be explicit**: the client always sends `vae_tiling_params` with a 256px tile and 0.5 overlap, because this build ignores tiling when only `enabled` is set and the untiled decode OOMs at a 1024x1536 canvas. **VRAM budget on a 24GiB card**: measured peak ≈23.4 GiB (weights resident ≈15.9 GB) — the LLM/planner cannot be resident during a generation; planner and generator must be swapped, not co-resident. Measured on RTX 3090 (int8, 25 steps): t2i 1024² ≈52 s sampling (≈1.9 s/it with CFG), reference edit/cutout 1024×1536 ≈140 s. **Alpha/RGBA cutouts come from the model's prompt-driven alpha channel** (verified against upstream's own examples), not from any segmentation + matting: the prompt asks for transparency and the alpha survives only in PNG/WebP. Cutout quality is subject-dependent — reliable for logo/object subjects, but a fixed blonde portrait produced the same failure under four wordings (opaque hair, transparent face). The authoritative request/response fields live in `stable-diffusion.cpp/examples/server/api.md` at the checked-out revision — read it before adding a field.

**Verifying a generation input actually reaches the model**: anything the server does not name in its JSON parser is dropped without a warning. When a field "has no effect", grep `stable-diffusion.cpp/examples/server/api.md` and its parser for the exact key before concluding the model ignores it.

- **LoRA serving**: sd.cpp registers only the *top-level* files of its lora directory. `prepare_lora_links` symlinks subdirectory adapters to the top level as `<subdir>-<basename>` before the server starts, and `LoraBindings::registered` maps request paths to those names (absolute paths are rejected by the server). Repeated `--lora` flags accumulate (slash + CLI). Multi-LoRA is validated end-to-end (several adapters applied together).
- **LoRA registry** (§12 item 6): project-level entries (SQLite `lora_registry`, round-tripped) with per-artifact bindings (`/attach-lora`, `/lora-scale`, `/lora-remove` — slash, CLI, LLM vocabulary), config use-case bindings (`[lora]`: per-kind defaults, shared multiplier), discovery from `lora_dir`, and auto-injection into every generation (artifact-bound → kind default; missing-on-disk adapters warn and skip).
- **Error codes + reporting** (§12 item 7): `crates/core/src/errors.rs` is the central registry — every `thiserror` type implements `CodedError` mapping to a stable `SVS-<DOMAIN>-<NNN>` code (code, short message, remediation hint); `CodedError::report` builds the `ReportedError` payload the UI shows (a global toast, Workspace-owned `Signal<Option<ReportedError>>` context, with a Details modal: full text + context + hint + copy via `document::eval`). `ReportedError` carries a `context` list (operation, generation prompt, model, size, steps, seed, job id, work dir) attached at the failure site (`PipelineError::Backend` carries it up from `CreativeBackend::generate`) — surfaces emit the single coded log line with `report.log()`. The CLI's op paths report the same way (svs.rs inits a tracing subscriber).
- **The image backend is provider-driven**: `providers.toml` at the project root (`paths::providers_path()`) declares the `sdcpp` provider — its `architecture` (must be a served profile), `capabilities`, `base_url`, `model_dir`/`lora_dir`/`executable`, and per-artifact `weights` overrides — under a `[defaults]` capability→id map. `crates/core/src/config.rs` loads it through `clipforge_inference::Providers::load_or_built_in(…)` → `image_deployment(Capability::T2i)`, mapping `ProviderError` to `SVS-CFG-108` and keeping the result on `AppConfig::image` (there is no `[generation]` endpoint/profile/executable and no `paths.model_dir` any more). The LoRA discovery dir is the provider's `lora_dir`; `model_setup` provisions through the same deployment (skipping artifacts a `weights` override supplies); `runtime serve` and `CreativeRuntime` both use `config.image.runtime()` and `config.image.host`/`port`.
- **The automation driver** (`apps/desktop/src/driver/` + `apps/desktop/src/commands.rs`): the intent seam — `apps/desktop/src/commands.rs` owns the app's action language (project create/open/save, slash-command parse→execute via `ExecutionUi`); the composer, the operation-log rerun, and the driver all call the same entry points. `apps/desktop/src/driver/` runs YAML plans from `config/plans/` (plan = steps in app vocabulary: `create_project`, `compose:` — types visibly into the canvas composer and submits like a user — `command: "/create …"`, `expect_artifact`, …; no selectors) in **Guide** mode (narrated by the floating assistant card, Plans rail → "Plans" screen) or **Check** mode (fast, per-step report, exit code; `smart-visual-sequencer --plan <file> --mode check`, final registry persisted before exit). The driver observes the same signals the UI uses (activity/error), never the DOM; checkpoints are auto-rejected.
- 211 Rust lib + bin tests pass (131 in `svs-core`, 77 in the desktop crate, plus the CLI bins' 3). The only generation profile is `qwen-image-2.1`; its t2i / instruction-edit / RGBA-cutout behaviour was measured on a CUDA `sd-server` (RTX 3090) in the 2026-09-30 spike — see `.svs-e2e/logs/2026-09-30-quality-evals.md`. LLM `draft`, `stack propose`, LoRA injection, and the size/step controls still await live validation.
- Next per §12: Studio integration (item 8) — **deferred** by user
  reprioritization (generation quality + LoRA registry landed first; the
  tracker is authoritative).

# Validation

Always verify changes that touch Rust code:

```sh
cargo check --workspace --all-targets
cargo test --workspace
```

That is `cargo xtask check` (+ `test`), which wraps the same commands from the workspace root. Do not modify `Cargo.toml` features or add dependencies without a strong reason — dependencies live in `[workspace.dependencies]` at the root and are added there once, then referenced from a member with `name.workspace = true`; the `desktop` feature lives in `apps/desktop`.

## Scratch files stay in the project, never in `/tmp`

Every temporary artifact of an experiment — probe scripts, generated images,
downloaded test inputs, server logs — goes under `.svs-e2e/`, which is
gitignored. Do not use `/tmp`, an agent scratchpad, or any other path outside
the checkout: those are wiped between sessions, and generation experiments are
expensive to reproduce (minutes of model loading per run). An image that cost
40s to make and the exact script that made it must both survive to the next
session, or the next investigation restarts from zero — which is how this
project spent days re-deriving the same quality findings.

| Path | Holds |
| :--- | :--- |
| `.svs-e2e/inputs/` | Fixed test inputs (downloaded photos, hand-made test images) — the stable base an A/B is measured against |
| `.svs-e2e/scripts/` | Probe scripts. Resolve paths from `Path(__file__).resolve().parents[2]`, never hardcode an absolute path |
| `.svs-e2e/quality/<experiment>/` | Generated outputs, one directory per experiment |
| `.svs-e2e/logs/` | sd-server logs, named for the configuration that produced them |

Prefer a downloaded photo from `.svs-e2e/inputs/` over generating a base image
when the experiment is about *editing*: it removes a 20–40s generation from
every iteration and keeps the base bit-identical across runs, so a measured
difference is attributable to the change under test.

Write the measurement into the script, not the eyeball. Every probe should
print a number (change inside vs. outside a region, drift against the base)
so results are comparable across sessions; record the conclusions in
`.svs-e2e/logs/<date>-quality-evals.md`.

# Dioxus 0.7 essentials

Dioxus 0.7 changed every API. Only use the current documentation at <https://dioxuslabs.com/learn/0.7>. `cx`, `Scope`, and `use_state` are gone. Provide concise code examples with detailed descriptions.

## Dependency

```toml
[dependencies]
dioxus = { version = "0.7.1" }

[features]
default = ["desktop"]
desktop = ["dioxus/desktop"]
```

## Launching

Create a main function that sets up the Dioxus runtime and mounts your root component:

```rust
use dioxus::prelude::*;

fn main() {
	dioxus::launch(App);
}

#[component]
fn App() -> Element {
	rsx! { "Hello, Dioxus!" }
}
```

Serve with `dx serve --platform desktop`.

## UI with RSX

```rust
rsx! {
	div {
		class: "container", // Attribute
		color: "red", // Inline styles
		width: if condition { "100%" }, // Conditional attributes
		"Hello, Dioxus!"
	}
	// Prefer loops over iterators
	for i in 0..5 {
		div { "{i}" } // use elements or components directly in loops
	}
	if condition {
		div { "Condition is true!" } // use elements or components directly in conditionals
	}

	{children} // Expressions are wrapped in brace
	{(0..5).map(|i| rsx! { span { "Item {i}" } })} // Iterators must be wrapped in braces
}
```

## Assets

The asset macro links to local files. All links start with `/` and are relative to the root of your project:

```rust
rsx! {
	img {
		src: asset!("/assets/image.png"),
		alt: "An image",
	}
}
```

`document::Stylesheet` injects a stylesheet into the `<head>`:

```rust
rsx! {
	document::Stylesheet {
		href: asset!("/assets/styles.css"),
	}
}
```

Generated images are not static assets; they are served through the custom `generated` asset handler in `apps/desktop/src/ui/mod.rs`.

## Components

- Components are functions annotated with `#[component]`.
- The function name must start with a capital letter or contain an underscore.
- A component re-renders only under two conditions: its props change (as determined by `PartialEq`), or internal reactive state it depends on is updated.

```rust
#[component]
fn Input(mut value: Signal<String>) -> Element {
	rsx! {
		input {
            value,
			oninput: move |e| {
				*value.write() = e.value();
			},
			onkeydown: move |e| {
				if e.key() == Key::Enter {
					value.write().clear();
				}
			},
		}
	}
}
```

Props are function arguments:

- Props must be owned values, not references. Use `String` and `Vec<T>` instead of `&str` or `&[T]`.
- Props must implement `PartialEq` and `Clone`.
- To make a prop reactive and copy, wrap the type in `ReadOnlySignal`. Reactive state like memos and resources that read `ReadOnlySignal` props re-run when the prop changes.

## State

A signal wraps a value and automatically tracks where it's read and written. Changing a signal's value reruns the code that relies on it.

### Local state

`use_signal` creates state local to a component. Call the signal like a function (e.g. `my_signal()`) to clone the value, `.read()` for a reference, `.write()` for a mutable reference.

`use_memo` creates a memoized value that recalculates when its dependencies change:

```rust
#[component]
fn Counter() -> Element {
	let mut count = use_signal(|| 0);
	let mut doubled = use_memo(move || count() * 2); // re-runs when count changes

	rsx! {
		h1 { "Count: {count}" } // re-renders when count changes
		h2 { "Doubled: {doubled}" }
		button {
			onclick: move |_| *count.write() += 1,
			"Increment"
		}
		button {
			onclick: move |_| count.with_mut(|count| *count += 1),
			"Increment with with_mut"
		}
	}
}
```

### Context API

A parent provides state with `use_context_provider`; any child consumes it with `use_context`:

```rust
#[component]
fn App() -> Element {
	let mut theme = use_signal(|| "light".to_string());
	use_context_provider(|| theme); // Provide a type to children
	rsx! { Child {} }
}

#[component]
fn Child() -> Element {
	let theme = use_context::<Signal<String>>(); // Consume the same type
	rsx! {
		div {
			"Current theme: {theme}"
		}
	}
}
```

## Async

For state that depends on an async operation, use `use_resource`. It takes an `async` closure and re-runs it whenever signals it reads are updated. Reading the `Resource` returns `None` while loading and `Some(value)` once loaded:

```rust
let mut dog = use_resource(move || async move {
	// api request
});

match dog() {
	Some(dog_info) => rsx! { Dog { dog_info } },
	None => rsx! { "Loading..." },
}
```

For fire-and-forget work (such as starting a generation and updating app state from a spawned task), use `spawn(async move { ... })` as in `apps/desktop/src/ui/editor.rs`.

## Dioxus 0.7 usage notes (learned while building this app)

- `use_effect(move || { ... })` runs after the first render and re-runs whenever a signal it reads changes. The closure is `FnMut`, so it cannot move captured non-`Copy` values out — clone them inside (`runtime.clone()`) or move the originals in once and never use them again.
- `Signal::set(value)` consumes a copy of the signal, so the binding must be declared `mut` at the call site (`let mut prompt = use_signal(...); prompt.set(...)`). Prefer `*prompt.write() = value` when the binding is not `mut`.
- Signals are `Copy`. Pass them to child components as props (`Signal<T>`) and to plain helper functions that mutate them — the helper declares `mut signal: Signal<T>` parameters.
- `EventHandler<T>` props receive closures at the call site (`onclick: move |event| ...`). Handlers are `Clone`, so one handler can be shared between several components by binding it once.
- `use_hook` constructs one-time, non-reactive values (for example `CreativeRuntime::new(&config)` in `apps/desktop/src/ui/mod.rs`).
- In `spawn`ed tasks, never hold a signal `.read()`/`.write()` guard across an `await`; read or clone first, await, then write (see `start_generation` in `apps/desktop/src/ui/editor.rs`).

## Routing (reference only — this desktop-only app does not use Dioxus routing)

All possible routes are defined in a single Rust `enum` that derives `Routable`. Each variant represents a route and is annotated with `#[route("/path")]`. Dynamic Segments can capture parts of the URL path as parameters by using `:name` in the route string. These become fields in the enum variant.

The `Router<Route> {}` component is the entry point that manages rendering the correct component for the current URL.

You can use the `#[layout(NavBar)]` to create a layout shared between pages and place an `Outlet<Route> {}` inside your layout component. The child routes will be rendered in the outlet.

```rust
#[derive(Routable, Clone, PartialEq)]
enum Route {
	#[layout(NavBar)] // This will use NavBar as the layout for all routes
		#[route("/")]
		Home {},
		#[route("/blog/:id")] // Dynamic segment
		BlogPost { id: i32 },
}

#[component]
fn NavBar() -> Element {
	rsx! {
		a { href: "/", "Home" }
		Outlet<Route> {} // Renders Home or BlogPost
	}
}

#[component]
fn App() -> Element {
	rsx! { Router::<Route> {} }
}
```

```toml
dioxus = { version = "0.7.1", features = ["router"] }
```

## Fullstack (reference only — web/server targets are not used in this project)

Fullstack enables server rendering and ipc calls. It uses Cargo features (`server` and a client feature like `web`) to split the code into a server and client binaries.

```toml
dioxus = { version = "0.7.1", features = ["fullstack"] }
```

### Server Functions

Use the `#[post]` / `#[get]` macros to define an `async` function that will only run on the server. On the server, this macro generates an API endpoint. On the client, it generates a function that makes an HTTP request to that endpoint.

```rust
#[post("/api/double/:path/&query")]
async fn double_server(number: i32, path: String, query: i32) -> Result<i32, ServerFnError> {
	tokio::time::sleep(std::time::Duration::from_secs(1)).await;
	Ok(number * 2)
}
```

### Hydration

Hydration is the process of making a server-rendered HTML page interactive on the client. The server sends the initial HTML, and then the client-side runs, attaches event listeners, and takes control of future rendering.

Errors: the initial UI rendered by the component on the client must be identical to the UI rendered on the server.

- Use the `use_server_future` hook instead of `use_resource`. It runs the future on the server, serializes the result, and sends it to the client, ensuring the client has the data immediately for its first render.
- Any code that relies on browser-specific APIs (like accessing `localStorage`) must be run *after* hydration. Place this code inside a `use_effect` hook.

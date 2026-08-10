You are working on **Smart Visual Sequencer**, a desktop-only Dioxus 0.7 application for planning, generating, and arranging visual stories. Rust owns the UI, state, models, persistence, process lifecycle, and LM Studio client. Python owns all ML framework code behind a FastAPI HTTP boundary. Image generation runs through a native `stable-diffusion.cpp` (Krea 2) process; it is not ComfyUI.

Current shape: the artifact registry + typed operation layer (`src/registry/`), the SQLite project store, and the canvas are implemented and driven by the `svs` CLI and the desktop app (see `# Current status` below). Read `docs/ROADMAP.md` §12 (the feature/status tracker) before planning changes.

# Project rules

- **Desktop only.** Never add web, mobile, WASM, routing, or fullstack features. The `Cargo.toml` features are limited to `default = ["desktop"]` / `dioxus/desktop`. Ignore any Dioxus docs about routers, server functions, or hydration.
- **No stale/dead code; no backward-compatibility shims.** We are pre-1.0 and not in production — formats, models, and APIs change freely. When a change replaces an older shape, remove the old one **in the same change**: no deprecated aliases, no "keep for old files" parse/import paths, no unused fields, no `#[allow(dead_code)]` for hypothetical future use, no leaving a superseded implementation around "for reference". A one-time import of a legacy format is sanctioned only while real files exist that need it (and must be removed once they're migrated or abandoned — SQLite schema migrations are the way to evolve the current format in place). Deferred vocabulary is allowed only when a roadmap item explicitly names it (e.g. slice-1 pipeline steps marked "vocabulary, not executable"); mark it with a doc comment naming the deferred item, never with `allow(dead_code)`. Concrete implication: when the canvas (roadmap item 4) replaces the storyboard model, delete `StoryboardFrame`/`project_json` rather than keeping them alongside.
- **Reusable components before repetition.** Isolate repeated patterns into small shared components/modules and reuse them everywhere. The UI vocabulary in `src/ui/components.rs` (`SettingRow`, `EmptyVisual`, `StatusDot`, …) and `src/ui/icons.rs` is the model; apply the same instinct outside the UI (registry ops, pipeline steps, persistence helpers, config). When a new use case needs a variant, extend the shared component instead of copy-pasting a near-identical one, and keep similar use cases looking and behaving the same. A new control for a case an existing control already covers is a code smell.
- **No perl for code edits.** Never use `perl -i` (or similar opaque one-liner rewrites) to modify source files — they are hard to review and have corrupted code before. Make edits with the file editing tools (`edit_file`/`write_file`), which produce reviewable diffs; `cargo fmt` is the only bulk formatter allowed.
- **Rust/Python boundary.** Rust talks to Python only over HTTP (`VisionClient` in `src/vision/mod.rs` ↔ contracts in `python/models/schemas.py`). Keep the two sides' request/response types in sync.
- **No ML frameworks in Rust.** Rust must never depend on or know about PyTorch, Transformers, or SAM. Those details live in `python/` only.
- **LM Studio is external** and never bundled or started by the app; `src/llm/` is the client only. The planner vocabulary (typed operations + pipelines) lives in `src/registry/`; LLM steps are soft dependencies that degrade to manual input, never hard-fail.
- **UI styling** is Tailwind CSS 4 utility classes in RSX (`assets/tailwind.css` is generated output; regenerate with `npm run css:build`). Prefer the small shared component vocabulary in `src/ui/components.rs` (`SettingRow`, `EmptyVisual`, `StatusDot`, …) and icons from `src/ui/icons.rs`.
- **State mutations** go through `AppState` methods in `src/state/mod.rs`, which maintain the storyboard/timeline invariants documented in `docs/data-model.md`. Never mutate `Project` fields directly from UI code.
- **The repository path contains a literal `:`.** Build artifacts are redirected to `/tmp/smart-visual-sequencer-target` by `.cargo/config.toml`; never remove that file.
- **Build with `dx build --platform desktop` for runnable binaries, not plain `cargo build`.** The `asset!` pipeline (Tailwind CSS + custom protocols) is wired by the dx CLI: a plain `cargo build` binary starts fine but renders **unstyled** (no CSS — the stylesheet is never embedded/served). `cargo check`/`cargo test` stay plain cargo; only launching a built app needs the dx artifact (`/tmp/smart-visual-sequencer-target/dx/smart-visual-sequencer/debug/linux/app/smart-visual-sequencer` after `dx build`).
- **Validation logs and scratch files stay in the repo** under `.svs-e2e/` (gitignored). When live-testing the GUI, write its stdout/logs to `.svs-e2e/logs/` — never to `/tmp` or the repo root. When killing a stale app instance, use a bracket-quoted pattern so `pgrep`/`pkill` do not match their own command line (e.g. `pkill -f "smart-visual-sequence[r]"`).
- **The vision evaluator — the agent's eyes.** The agent cannot see images; when judging generations, edits, or regressions, ask the local LM Studio vision model through `svs eval -i <image>… -p "<question>"` (default model `google/gemma-4-e4b`, must be running in LM Studio). Images are auto-downscaled to 1024px; default max-tokens 2048 (the model reasons before answering, and 512 was exhausted by reasoning). `--expect <substring>` / `--expect-not <substring>` gate the exit code for scripted checks — craft prompts that elicit strict YES/NO. Comparison workflow: attach the base first, then the candidate(s); the model refers to them as image 1, image 2, …. Common checks: localization of a mask edit, identity preservation across variants, artifacts/distortion in a generation. Evaluations used to drive a decision should be recorded in `.svs-e2e/logs/`.
- Consult the documentation below before making changes; many decisions and known gaps are recorded there.

# Documentation

- `docs/ROADMAP.md` — product design and **the planned-feature status tracker (§12)**: every feature to ship, with its status. Read §12 first to orient; update its statuses when work lands.
- `docs/api-slice-1.md` — implementation record of the API slice: settled decisions, module map, `svs` CLI reference, session guide (which ops need the generation backend).
- `docs/architecture.md` — system boundaries, component map, prompt-to-revision data flow.
- `docs/development.md` — setup, build/run/test commands, `config/app.toml` reference, troubleshooting.
- `docs/http-api.md` — the Python runtime HTTP contract (endpoints, schemas, job lifecycle).
- `docs/runtime-lifecycle.md` — process supervision, readiness, model provisioning, residency.
- `docs/data-model.md` — domain types, state invariants, persistence format, known gaps.

# Current status

The API slice (`docs/ROADMAP.md` §12 items 1–3), the canvas (item 4), the
generation quality controls (item 5), the LoRA registry (item 6), and the
error-code registry (item 7) are implemented and validated:

- `src/registry/` — artifact registry (kinds, variants, scenes/beats + layers, revisions + masks, drafts, LoRA entries), typed slice-1 operations (`create`, `variant`, `regenerate`, `compose`, `draft`, `modify`) + LoRA ops (`attach-lora`, `lora-scale`, `lora-remove`), pipeline builder (closed step vocabulary, typed handles, static validation at `build()`, linear fail-fast stacks, checkpoints), composite mask fallback, live backend (`CreativeBackend`), slash-command parser (`slash.rs`) for the canvas composer.
- `c:<name>` references are primary (case-insensitive exact match, ambiguity rejected; UUID/8-hex fallbacks).
- `src/persistence/` — SQLite project store (`ProjectDb`): §10 schema (v3 drops the legacy `project_json` column), versioned migrations (`schema_meta`), WAL, lossless round-trip (including the `lora_registry` table); the CLI persists to `.svs-project.db`. No legacy formats are parsed — the current format evolves via migrations.
- `src/ui/` — the Projects screen lists, creates, and opens `.svs-project.db` files; the **Canvas** is the artifact workspace (sidebar grouped by kind, detail pane with image/variants/revisions/children/LoRAs/operation log, slash composer, undo/redo via registry snapshots); the workspace saves via autosave (after every change / every minute / off) or Cmd/Ctrl+S; `AppState.project_path` tracks the open database. The legacy storyboard model (`Project`/`StoryboardFrame`/`Timeline`) was deleted with the canvas.
- `svs` CLI — `op` (incl. `--lora <file>@<mult>`, `attach-lora`/`lora-scale`/`lora-remove`), `stack run`/`propose`, `runtime serve --force`, `log`, `project`; `--out` golden runs; `--approve auto|interactive`; `eval` (vision evaluation through LM Studio — see the project rule on the vision evaluator).
- **Generation quality controls** (`docs/ROADMAP.md` §12 item 5): per-kind default sizes + steps are configurable (`config/app.toml` `[generation.sizes]`/`steps`) and editable in Settings → Image generation (persisted line-preserving); a live `Signal<GenerationDefaults>` (Workspace context) drives the canvas ops. The LLM sets size/steps/adapters only when the user explicitly asks (visual ops carry `size`/`steps`/`loras`; the `resize` op persists a new default size and regenerates at it). Free-form planning runs a tool-calling loop (`list_artifacts`/`list_loras`) with a strict schema on the final stack, falling back to the static planner when `tools` are unsupported. `variant --name` supports explicit naming.
- **Reference strength (img2img blend)**: the sd.cpp server defaults `control_strength` to 0.9 — near-full renoise that distorts turbo-model outputs. Every reference-based op sets a tuned value (`variant`/`regenerate` 0.45, `resize` 0.35, `modify` inpaint 0.95) carried through `GenerateRequest.reference_strength` → the server's `control_strength`. **Probed caveat**: the current sd.cpp build's `krea2_ostris_edit` preset ignores `control_strength` (0.3/0.5/0.9 produce identical output); the config `[lora].variant` binding applies Krea's identity-preserving edit adapter to variant/regenerate only (validated 5/5 identity); the auto-injected inpaint adapter (INPAINTKREA) is capped at 0.5 — at 1.0 it collapses a reference edit into a near no-op. **Masked-latent inpaint (modify)**: `mask_images` alone IS ignored (probed: identical output), but the generic per-step denoise-mask blend (`denoised = denoised*mask + init_latent*(1-mask)`) engages for ANY model when `init_image` is ALSO sent — so `modify` now sends reference + mask + init image, and the edit is localized at the latent level (validated 2026-08-09: same face/body/background, only the masked hair changes; haircut and recolor both pass the evaluator). The composite step stays as the boundary smoother (its feather removes the latent-level seam; erosion keeps it inside the replacement) — evaluated as cleaner than the raw masked-latent output. The inpaint prompt is composed from the artifact description + the edit, so the re-render keeps the scene's lighting and colors (a bare prompt renders the masked region near-black); keep appearance traits the edit will override OUT of the artifact description — a prompt that demands both "long dark hair" and "bob cut" splits the difference (left long / right bob). **Reference-image preset**: the sd-server must run with `--ref-image-args "preset=krea2_edit"` — the identity-edit LoRA is trained with 768px VLM grounding (`krea2_ostris_edit`'s AREA mode passes up to 1536px, which the model card's own README warns causes split/duplicate compositions; validated: the `krea2_edit` preset wins on likeness and naturalness). `ref_image_args` is CLI-only — the server JSON API never exposes it, so the flag goes on the sd-server launch line. **VAE tiling**: `vae_tiling_params` must carry an explicit `tile_size`/`overlap` — this sd.cpp build ignores tiling when only `enabled` is set, and the untiled buffers (11.2GB decode / 7.6GB encode at 1024x1536) OOM the 24GiB card with one LM Studio model resident. With explicit 256px tiles the full portrait canvas fits even with the vision model loaded (validated 2026-08-09: create 1024x1536@4 steps ≈ 19s, modify with ref+mask+2 LoRAs ≈ 39s; unloaded-LM generations run ~21–28s).
- **LoRA serving**: sd.cpp registers only the *top-level* files of its lora directory. `prepare_lora_links` symlinks subdirectory adapters to the top level as `<subdir>-<basename>` before the server starts, and `LoraBindings::registered` maps request paths to those names (absolute paths are rejected by the server). Repeated `--lora` flags accumulate (slash + CLI). Multi-LoRA validated end-to-end (facial + INPAINTKREA applied together).
- **Segmentation (modify)**: text prompts route to **SAM 3** (`Sam3Segmenter` — native text prompting): primary `facebook/sam3.1` (HF-gated, and ships only `.pt` multiplex weights — needs a safetensors conversion when access lands), fallback `obsxrver/sam3` (ungated transformers-format port, validated end-to-end), then DINO grounding + SAM 2.1 (`GroundingDinoGrounder` + `Sam2Segmenter`, weights cached). Geometry prompts stay on SAM 2. The composite fallback resizes mismatched canvases (nearest for masks, lanczos for the image) and locally-produced finals are imported into the asset store via `GenerationBackend::import_asset`.
- **LoRA registry** (§12 item 6): project-level entries (SQLite `lora_registry`, round-tripped) with per-artifact bindings (`/attach-lora`, `/lora-scale`, `/lora-remove` — slash, CLI, LLM vocabulary), config use-case bindings (`[lora]`: inpaint adapter for mask edits, per-kind defaults, shared multiplier), discovery from `lora_dir`, and auto-injection into every generation (artifact-bound → kind default → inpaint adapter on `modify`; missing-on-disk adapters warn and skip).
- **Error codes + reporting** (§12 item 7): `src/errors.rs` is the central registry — every `thiserror` type implements `CodedError` mapping to a stable `SVS-<DOMAIN>-<NNN>` code (code, short message, remediation hint); `CodedError::report` builds the `ReportedError` payload the UI shows (a global toast, Workspace-owned `Signal<Option<ReportedError>>` context, with a Details modal: full text + context + hint + copy via `document::eval`). `ReportedError` carries a `context` list (operation, generation prompt, model, size, steps, seed, job id, work dir) attached at the failure site (`PipelineError::Backend` carries it up from `CreativeBackend::generate`) — surfaces emit the single coded log line with `report.log()`. The CLI's op paths report the same way (svs.rs inits a tracing subscriber).
- `mask_path`/`init_image_path` in the Rust ↔ Python `GenerateRequest` contract (masked-latent inpaint: reference + mask + init image keep the unmasked pixels fixed; the composite step is the boundary smoother/fallback).
- **The automation driver** (`src/driver/` + `src/commands.rs`): the intent seam — `src/commands.rs` owns the app's action language (project create/open/save, slash-command parse→execute via `ExecutionUi`); the composer, the operation-log rerun, and the driver all call the same entry points. `src/driver/` runs YAML plans from `config/plans/` (plan = steps in app vocabulary: `create_project`, `compose:` — types visibly into the canvas composer and submits like a user — `command: "/create …"`, `expect_artifact`, …; no selectors) in **Guide** mode (narrated by the floating assistant card, Plans rail → "Plans" screen) or **Check** mode (fast, per-step report, exit code; `smart-visual-sequencer --plan <file> --mode check`, final registry persisted before exit). The driver observes the same signals the UI uses (activity/error), never the DOM; checkpoints are auto-rejected.
- 140 Rust lib + bin tests pass. The default generation profile is
  `krea-2-turbo-q4`; q4 generation was validated end-to-end through the
  canvas on Linux/RTX 3090 (CUDA sd-server build; the q4 diffusion GGUF is
  placed on disk — `model_setup` is not run for locally-provided files).
  q2 generation was validated on macOS. `modify`'s mask path, LLM `draft`,
  `stack propose`, LoRA injection, and the new size/step controls still
  await live validation.
- Next per §12: Studio integration (item 8) — **deferred** by user
  reprioritization (generation quality + LoRA registry landed first; the
  tracker is authoritative).

# Validation

Always verify changes that touch Rust or Python code:

```sh
cargo check --features desktop
cargo test --features desktop
PYTHONPATH=python python/.venv/bin/python -m unittest discover -s python/tests
```

The Python tests skip gracefully when optional torch extras are absent; `OK (skipped=N)` is expected on a base environment. Do not modify `Cargo.toml` features or add dependencies without a strong reason.

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

Generated images are not static assets; they are served through the custom `generated` asset handler in `src/ui/mod.rs`.

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

For fire-and-forget work (such as starting a generation and updating app state from a spawned task), use `spawn(async move { ... })` as in `src/ui/editor.rs`.

## Dioxus 0.7 usage notes (learned while building this app)

- `use_effect(move || { ... })` runs after the first render and re-runs whenever a signal it reads changes. The closure is `FnMut`, so it cannot move captured non-`Copy` values out — clone them inside (`runtime.clone()`) or move the originals in once and never use them again.
- `Signal::set(value)` consumes a copy of the signal, so the binding must be declared `mut` at the call site (`let mut prompt = use_signal(...); prompt.set(...)`). Prefer `*prompt.write() = value` when the binding is not `mut`.
- Signals are `Copy`. Pass them to child components as props (`Signal<T>`) and to plain helper functions that mutate them — the helper declares `mut signal: Signal<T>` parameters.
- `EventHandler<T>` props receive closures at the call site (`onclick: move |event| ...`). Handlers are `Clone`, so one handler can be shared between several components by binding it once.
- `use_hook` constructs one-time, non-reactive values (for example `CreativeRuntime::new(&config)` in `src/ui/mod.rs`).
- In `spawn`ed tasks, never hold a signal `.read()`/`.write()` guard across an `await`; read or clone first, await, then write (see `start_generation` in `src/ui/editor.rs`).

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

You are working on **Smart Visual Sequencer**, a desktop-only Dioxus 0.7 application for planning, generating, and arranging visual stories. Rust owns the UI, state, models, persistence, process lifecycle, and LM Studio client. Python owns all ML framework code behind a FastAPI HTTP boundary. Image generation runs through a native `stable-diffusion.cpp` (Krea 2) process; it is not ComfyUI.

Current shape: the artifact registry + typed operation layer (`src/registry/`) is implemented and driven by the `svs` CLI (see `# Current status` below); the creation canvas and SQLite are roadmap work. Read `docs/ROADMAP.md` §12 (the feature/status tracker) before planning changes.

# Project rules

- **Desktop only.** Never add web, mobile, WASM, routing, or fullstack features. The `Cargo.toml` features are limited to `default = ["desktop"]` / `dioxus/desktop`. Ignore any Dioxus docs about routers, server functions, or hydration.
- **No stale/dead code; no backward-compatibility shims.** We are pre-1.0 and not in production — formats, models, and APIs change freely. When a change replaces an older shape, remove the old one **in the same change**: no deprecated aliases, no "keep for old files" parse/import paths, no unused fields, no `#[allow(dead_code)]` for hypothetical future use, no leaving a superseded implementation around "for reference". The only sanctioned migration is a documented one-time import of a legacy format into the current one (like `svs import` and the GUI's TOML/JSON import) — and that disappears once nothing needs it. Deferred vocabulary is allowed only when a roadmap item explicitly names it (e.g. slice-1 pipeline steps marked "vocabulary, not executable"); mark it with a doc comment naming the deferred item, never with `allow(dead_code)`. Concrete implication: when the canvas (roadmap item 4) replaces the storyboard model, delete `StoryboardFrame`/`ProjectStore`/`project_json` rather than keeping them alongside.
- **Reusable components before repetition.** Isolate repeated patterns into small shared components/modules and reuse them everywhere. The UI vocabulary in `src/ui/components.rs` (`SettingRow`, `EmptyVisual`, `StatusDot`, …) and `src/ui/icons.rs` is the model; apply the same instinct outside the UI (registry ops, pipeline steps, persistence helpers, config). When a new use case needs a variant, extend the shared component instead of copy-pasting a near-identical one, and keep similar use cases looking and behaving the same. A new control for a case an existing control already covers is a code smell.
- **Rust/Python boundary.** Rust talks to Python only over HTTP (`VisionClient` in `src/vision/mod.rs` ↔ contracts in `python/models/schemas.py`). Keep the two sides' request/response types in sync.
- **No ML frameworks in Rust.** Rust must never depend on or know about PyTorch, Transformers, or SAM. Those details live in `python/` only.
- **LM Studio is external** and never bundled or started by the app; `src/llm/` is the client only. The planner vocabulary (typed operations + pipelines) lives in `src/registry/`; LLM steps are soft dependencies that degrade to manual input, never hard-fail.
- **UI styling** is Tailwind CSS 4 utility classes in RSX (`assets/tailwind.css` is generated output; regenerate with `npm run css:build`). Prefer the small shared component vocabulary in `src/ui/components.rs` (`SettingRow`, `EmptyVisual`, `StatusDot`, …) and icons from `src/ui/icons.rs`.
- **State mutations** go through `AppState` methods in `src/state/mod.rs`, which maintain the storyboard/timeline invariants documented in `docs/data-model.md`. Never mutate `Project` fields directly from UI code.
- **The repository path contains a literal `:`.** Build artifacts are redirected to `/tmp/smart-visual-sequencer-target` by `.cargo/config.toml`; never remove that file.
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

The API slice (`docs/ROADMAP.md` §12 items 1–3) is implemented and CLI-validated:

- `src/registry/` — artifact registry (kinds, variants, scenes/beats + layers, revisions + masks, drafts), typed slice-1 operations (`create`, `variant`, `regenerate`, `compose`, `draft`, `modify`), pipeline builder (closed step vocabulary, typed handles, static validation at `build()`, linear fail-fast stacks, checkpoints), composite mask fallback, live backend (`CreativeBackend`).
- `c:<name>` references are primary (case-insensitive exact match, ambiguity rejected; UUID/8-hex fallbacks).
- `src/persistence/` — SQLite project store (`ProjectDb`): §10 schema (v2 adds `project_json` for the legacy storyboard), versioned migrations (`schema_meta`), WAL, lossless round-trip; the CLI persists to `.svs-project.db`; `svs import <legacy.json>` migrates the old JSON stopgap.
- `src/ui/` — the Projects screen lists, creates, opens, and imports `.svs-project.db` files (legacy TOML/JSON included); the workspace saves via autosave (after every change / every minute / off) or Cmd/Ctrl+S; `AppState.project_path` tracks the open database.
- `svs` CLI — `op`, `stack run`/`propose`, `runtime serve --force`, `log`, `project`, `import`; `--out` golden runs; `--approve auto|interactive`.
- `mask_path` in the Rust ↔ Python `GenerateRequest` contract (best-effort native passthrough; composite fallback is primary).
- 75 Rust lib + 4 CLI/bin tests pass; q2 generation validated on macOS. `modify`'s mask path, LLM `draft`, and `stack propose` await live validation (Linux/CUDA).
- Next per §12: the canvas (item 4). The tracker in §12 is authoritative — update it as features ship.

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

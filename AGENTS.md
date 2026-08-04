You are working on **Smart Visual Sequencer**, a desktop-only Dioxus 0.7 application for planning, generating, and arranging visual stories. Rust owns the UI, state, models, persistence, process lifecycle, and LM Studio client. Python owns all ML framework code behind a FastAPI HTTP boundary. Image generation runs through a native `stable-diffusion.cpp` (Krea 2) process; it is not ComfyUI.

# Project rules

- **Desktop only.** Never add web, mobile, WASM, routing, or fullstack features. The `Cargo.toml` features are limited to `default = ["desktop"]` / `dioxus/desktop`. Ignore any Dioxus docs about routers, server functions, or hydration.
- **Rust/Python boundary.** Rust talks to Python only over HTTP (`VisionClient` in `src/vision/mod.rs` ↔ contracts in `python/models/schemas.py`). Keep the two sides' request/response types in sync.
- **No ML frameworks in Rust.** Rust must never depend on or know about PyTorch, Transformers, or SAM. Those details live in `python/` only.
- **LM Studio is external** and never bundled or started by the app; `src/llm/` is the only touchpoint. The planner business logic is not implemented yet.
- **UI styling** is Tailwind CSS 4 utility classes in RSX (`assets/tailwind.css` is generated output; regenerate with `npm run css:build`). Prefer the small shared component vocabulary in `src/ui/components.rs` (`SettingRow`, `EmptyVisual`, `StatusDot`, …) and icons from `src/ui/icons.rs`.
- **State mutations** go through `AppState` methods in `src/state/mod.rs`, which maintain the storyboard/timeline invariants documented in `docs/data-model.md`. Never mutate `Project` fields directly from UI code.
- **The repository path contains a literal `:`.** Build artifacts are redirected to `/tmp/smart-visual-sequencer-target` by `.cargo/config.toml`; never remove that file.
- Consult the documentation below before making changes; many decisions and known gaps are recorded there.

# Documentation

- `docs/architecture.md` — system boundaries, component map, prompt-to-revision data flow.
- `docs/development.md` — setup, build/run/test commands, `config/app.toml` reference, troubleshooting.
- `docs/http-api.md` — the Python runtime HTTP contract (endpoints, schemas, job lifecycle).
- `docs/runtime-lifecycle.md` — process supervision, readiness, model provisioning, residency.
- `docs/data-model.md` — domain types, state invariants, persistence format, known gaps.

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

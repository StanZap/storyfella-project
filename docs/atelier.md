# Atelier and the shared AI boundary

Atelier is `apps/atelier`, package `storyfella-atelier`, binary `atelier`.
It is a separate GPUI desktop app in the Storyfella workspace; Studio and Player
remain Dioxus. The move preserves the current app, including working-tree
changes, the Metal renderer patch, `.atelier` projects, provider profiles and
credentials. It does not convert either interface to the other framework.

## Run and data

From the workspace root:

```sh
cargo xtask atelier
cargo xtask atelier -- --data-dir .svs-e2e/atelier-dev
cargo test -p storyfella-atelier
```

`cargo xtask atelier` builds and runs only the GPUI app with Cargo. Studio's
`cargo xtask run` continues to use Dioxus; `svs` remains the separate CLI.
Use `-r` for release, `-l` to tee build/app output to
`.svs-e2e/logs/atelier-<unix>.log`, and `--` to forward Atelier arguments.

On macOS, `sh apps/atelier/package-macos.sh` creates
`dist/atelier/Atelier.app`. The bundle identifier remains
`com.storyfella.atelier`. The default data directory remains
`~/Library/Application Support/Atelier` on macOS and `~/.local/share/Atelier`
on other desktop platforms. The macOS Keychain service remains `Atelier`.
No existing projects or keys are moved by the repository change.

The relocated workspace passed `cargo check --workspace --all-targets`,
`cargo test --workspace`, `cargo fmt --all -- --check` and Atelier's Clippy
check on macOS. Atelier's suite passed 33 tests; eight physical Metal tests
remain opt-in. The macOS development bundle builds and passes strict signature
verification. Linux CI dependencies were updated; Linux runtime validation
has not been repeated for this relocation.

The root Cargo patch points at `vendor/gpui-pre-apple`; its source, patch record
and licenses travel with the app. FineWineTTS retains its own renderer patch
because its review application also uses GPUI.

## What is connected now

Atelier owns its sketchbook document, drawing renderer, storage workers and
generation interface. It calls Replicate, fal.ai, Alibaba and GPT Image directly
through its hosted adapters, or submits a workflow to an existing ComfyUI server.
Generated images become references, not editable image layers. The UI uses
800 px previews and references are normalized to at most 1600 px per edge.

Studio consumes the sibling `clipforge-inference` crate for native image
inference, managed llama-server lifecycle and shared chat/vision infrastructure.
Atelier does not consume it yet. Relocation does not expose Atelier's hosted
providers in Studio or add text/vision assistance to Atelier.

## Planned shared AI interface

Clipforge is the intended home for AI execution shared by Studio and Atelier:
local managed servers, external services, capability discovery and provider
adapters. Storyfella owns creative intent, artifact/revision history, scene
bindings, image files and document persistence. UI framework types stay out of
the shared engine.

Start with a Rust library contract used by both real callers. An HTTP or MCP
surface can wrap that same implementation when a separate process needs it;
neither desktop app should require an extra daemon merely to share Rust code.
The interface should expose:

- Capability routing for text, vision, image creation and image editing.
- Explicit support for reference counts, masks, output sizes, alpha and seeds.
  Unsupported operations must be reported before submission rather than silently
  dropping inputs. Model-specific options remain typed adapter details.
- Shared job progress, result metadata and errors, including whether cancellation
  reaches the provider or merely stops local waiting. Retry policy must avoid
  submitting another paid generation without knowing the original job's state.
- Credential resolution without secrets in projects, logs or request metadata.
- Local process ownership, readiness and GPU residency coordination. Hosted
  services have no local Start/Stop lifecycle, and must not evict a local model
  solely because a request uses a remote image service.

This interface is planned. The current `clipforge-inference` provider registry
already routes text/vision and native images; hosted image adapters and Atelier's
consumer integration still need implementation. The first concrete reuse should
move Atelier's hosted image execution into that shared layer and adapt its
workers, preserving provider-specific semantics and existing local mock tests.

## Planned art-workspace milestones

1. Preserve full-resolution image originals separately from display previews.
2. Open a selected image in a large zoomable workspace with editing instructions,
   before/after comparison, version selection and image export.
3. Add image-backed canvas layers so a generated image can be painted over.
4. Add masked edits and multiple references for providers that support them.
5. Reuse the shared engine and GUI-free behavior in Dioxus Studio; integrate
   approved artwork with Studio's existing artifact/revision model rather than
   creating another project store.

Only the repository relocation is implemented here. The workflow milestones
remain open, and Atelier's prototype document is distinct from Studio's registry.

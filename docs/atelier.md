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
generation interface. Replicate, fal.ai, Alibaba and GPT Image requests use
`clipforge_inference::images::Client`. A thin app adapter resolves the selected
create/edit model and the existing environment/Keychain credential, owns a
Tokio runtime on the GPUI worker, then converts original result bytes and
metadata into a document reference. ComfyUI workflows remain app-local.
Generated images become references, not editable image layers. The UI uses
800 px previews and references are normalized to at most 1600 px per edge.

Studio consumes the sibling `clipforge-inference` crate for native image
inference, managed llama-server lifecycle and shared chat/vision infrastructure.
Atelier now consumes that same crate for hosted images. Studio's image runtime
still uses its native deployment adapter; hosted-provider selection is not
yet exposed in Studio, and Atelier has no text/vision assistant.

## Shared AI interface

Clipforge owns AI execution shared by Studio and Atelier: managed native
servers, OpenAI-compatible text/vision, provider routing and hosted image
adapters. Storyfella owns creative intent, artifact/revision history, image
files and document persistence. UI framework types stay out of the engine.

`clipforge_inference::images::Client` exposes one async `generate(Request)`
contract for managed sd-server and the four hosted image APIs. A source image
selects instruction editing; the result contains original bytes, provider,
model and an optional submitted job ID. No paid request is automatically retried.
Polling/download failures retain a known job ID, and cancellation/timeout
errors report whether a provider accepted cancellation. Native cancellation
stops waiting; the served sd-server API has no verified per-job cancel endpoint.

The registry accepts image kinds `sdcpp`, `fal`, `replicate`, `alibaba` and
`gptimage`. Resolve a client for `t2i` or `edit`; model routes that differ
between create/edit should be separate provider entries. Hosted credentials
can be passed as redacted `ApiKey` values or resolved from registry
`api_key_env`. Atelier keeps the existing `Atelier` Keychain namespace and
session cache and does not serialize keys into projects or settings.

Official API origins and model paths are validated before authenticated
submission. Queue URLs must share the API origin. Image downloads are bounded,
redirects are disabled and API keys are never sent to image CDNs. Hosted
requests do not start, stop or evict native servers. The caller owns its runtime
and output storage; no extra daemon is required.

This first contract supports one source image and the existing adapter payloads.
It does not expose masks, multiple references, arbitrary provider parameters
or native LoRA/sampling overrides (those remain on the lower-level `sd` API).
Replicate maps the app's three aspect ratios; exact output dimensions are
provider-dependent. Model presets moved from Atelier into Clipforge without
changing the existing IDs or making paid calls to verify them.

Validation after integration: Storyfella's workspace check/tests and strict
Atelier/xtask Clippy pass; Clipforge's `cargo xtask ci` passes, including MCP
smoke tests. Atelier has 28 passing tests and eight opt-in Metal tests; the
shared inference crate has 65 passing tests. Build-cache exhaustion on macOS
required clearing only Storyfella's regenerable Cargo incremental cache.

The public hosted client is tested against loopback mock providers for both
creation and editing, authentication, job polling, inline/CDN images and
cancellation. Registry checks reject unsupported capabilities and foreign API
origins. Atelier retains profile migration, settings, credential and ComfyUI
tests. The Clipforge CLI/MCP image tools still use the native `sd` path; future
HTTP/MCP exposure can wrap the shared client rather than duplicate adapters.

## Planned art-workspace milestones

1. Preserve full-resolution image originals separately from display previews.
2. Open a selected image in a large zoomable workspace with editing instructions,
   before/after comparison, version selection and image export.
3. Add image-backed canvas layers so a generated image can be painted over.
4. Add masked edits and multiple references for providers that support them.
5. Reuse the shared engine and GUI-free behavior in Dioxus Studio; integrate
   approved artwork with Studio's existing artifact/revision model rather than
   creating another project store.

The relocation and shared hosted image execution are implemented. The workflow milestones
remain open, and Atelier's prototype document is distinct from Studio's registry.

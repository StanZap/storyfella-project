# Runtime lifecycle

Rust drives two things: one native process and one external service:

| Process | Rust owner | Port | Started when |
| --- | --- | --- | --- |
| `sd-server` (stable-diffusion.cpp) | the **shared lifecycle owner** — `clipforge_inference::control` over the machine's shared state directory — which drives `sd::Runtime` | the image provider's `base_url` (`127.0.0.1:7861` by default in the shared `clipforge.toml`) | First generation request, or **Start** in Settings |
| LLM endpoint | external, user-run (or a managed `llama-server`) | user-configured (shared `clipforge.toml`) | external, or on demand |

The desktop app does **not** start `sd-server` at launch. The shell reports it as
idle until a generation request or **Start** triggers `CreativeRuntime::ensure_ready`.
This keeps a quiet app quiet and delays the multi-GiB model load until it is needed.

The process, client, model store, and profile are all owned by the sibling
`clipforge` project's crate, package **`clipforge-inference`**, pulled in as a
**path dependency** (`clipforge-inference = { path = "../clipforge/crates/inference" }`
in the root `[workspace.dependencies]`); the sibling checkout must be present at
that relative path for storyfella to build, and the crate is not a workspace
member. `crates/studio-core/src/runtime/` holds only the adapter: `CreativeRuntime`
(`creative_runtime.rs`), the health probes (`health.rs`), and `mod.rs`.

## One shared owner

The native server is owned by the machine, not by one app. `clipforge_inference::control`
is the lifecycle owner every front end drives — Storyfella Studio, Atelier, and the
Clipforge desktop/CLI:

- `control::start` launches the server **detached** (it outlives the caller) and
  records its pid in a **marker** under the shared state directory
  (`clipforge_inference::config::default_state_dir()`, `$SVS_STATE_DIR` override;
  on Linux `~/.local/share/clipforge/servers/<port>.marker`). A server another
  front end already started is reused, not duplicated.
- `control::stop` stops the server the marker records; a server started outside
  the owner (a hand-run binary) has no marker and is left alone.
- `control::load` ensures the *configured model* is the one resident: it is a
  no-op when the right model is already loaded, and it stops and restarts a
  server **the owner started** that currently holds a different one. A foreign
  server is reported, never killed.

Because the server is detached and shared, **the app does not stop it on exit**:
it stays resident for the next front end, exactly like a CLI `svs runtime start`.
It is stopped from Settings → Services → **Stop**, or with `svs runtime stop`
(`--force` also stops a foreign server).

`crates/studio-core/src/paths.rs` resolves the shared state directory
(`paths::state_dir`) alongside the shared `clipforge.toml`
(`paths::clipforge_config_path`); a hermetic `--plan` run points both — and the
app config — at scratch files under `.svs-e2e/` so it never touches the
developer's real state.

## `CreativeRuntime` facade

`CreativeRuntime` (`crates/studio-core/src/runtime/creative_runtime.rs`) is the single entry point
used by the UI. It is a **client of the shared owner**: it holds

- an `sd::Client` for the native `sd-server` (from the shared `clipforge-inference` crate),
- the image deployment (executable, address, weights, residency),
- the shared state directory, and
- the generated-asset directory (`<asset_dir>/generated`).

Its lifecycle methods delegate to `control` with that state directory; it never
owns a process itself.

### Readiness — `ensure_ready`

```text
ensure_ready()
    │
    ├─ control::load(deployment, state_dir)
    │    ├─ resident model matches?  ──▶ reuse it (no restart)
    │    ├─ a server the owner started holds a different model?
    │    │      ──▶ stop it, then start ours
    │    ├─ nothing answering ──▶ start ours (detached)
    │    └─ a foreign server holds the port ──▶ ProfileMismatch (reported)
    │
    └─ sd::Runtime::ensure_ready(<host>, <port>)
         (the image provider's `base_url` in the shared clipforge.toml)
         polls /sdcpp/v1/capabilities every 500 ms until the model matches,
         or a 600 s deadline → ReadinessTimeout
```

The model check is two-phase: capabilities answer "is the HTTP server up" and
"is the right model actually resident". Loading ≈17.3 GiB of weights takes
minutes, so the readiness cap stays generous (600 s). The Rust side never treats
process creation as readiness. A `ProfileMismatch` names the loaded model
(compared by file name or stem) and the fix: **Start** replaces a server the
shared owner recorded, or `svs runtime serve --force` / `svs runtime stop
--force` stops a foreign one.

A server is found by its **marker** in the shared state dir
(`servers/<port>.marker`: pid, port, executable); `ServerMarker::is_live`
re-checks that the pid is alive *and* really that binary, so a recycled pid is
never killed. `control::status` reports `Up { owned }` (answering; `owned` is the
marker pid when the shared owner started it) or `Down`; the CLI and Settings'
residency check read it. The health check probes `/sdcpp/v1/capabilities`
directly and reports a reachable server whose resident model is not ours as
`Degraded`, with the foreign model named.

### Job waiting — `wait_for_job`

After `POST /sdcpp/v1/img_gen` returns a `queued`/`generating` job, the facade
polls `GET /sdcpp/v1/jobs/{id}` every 150 ms until the job reaches a terminal
state or the 20-minute deadline expires. `failed` and `cancelled` map to
`CreativeRuntimeError::Job` with the backend's message.

### Asset import

On completion the facade base64-decodes `result.images[0].b64_json`, writes it
as `<asset_dir>/generated/<uuid>.png`, and returns the **canonical absolute
path**, which the registry stores on the revision and the UI serves back through
the `generated` asset handler (`/generated/<file>`). `import_asset` copies an
already-produced file into that directory the same way (a path already inside it
is returned unchanged).

## `Runtime` (sd.cpp, shared crate)

`sd::Runtime` (in the shared `clipforge-inference` crate —
`../clipforge/crates/inference/src/sd/runtime.rs`) starts the process; the
lifecycle owner builds one from the deployment's `ImageDeployment::runtime()` and
keeps it detached (`keep_alive`). It is defensive:

1. Refuses to start twice (`AlreadyRunning`).
2. Verifies the `sd-server` executable exists.
3. Verifies the artifacts the provider's architecture loads exist — under
   `<model_dir>/<model_root>/` by default, or at the local file the provider
   declares under `weights` (diffusion safetensors, text encoder, VAE).
4. Creates the LoRA directory and runs `prepare_lora_links` (sd.cpp registers
   only the *top-level* files of its lora directory, so subdirectory adapters are
   symlinked to the top level as `<subdir>-<basename>` before the server's
   startup scan).
5. Spawns with explicit arguments:

```text
sd-server \
  --diffusion-model <model_dir>/qwen-image-2.1/qwen_image_2.1_int8_convrot.safetensors \
  --llm             <model_dir>/qwen-image-2.1/qwen3vl_8b_int8_convrot.safetensors \
  --vae             <model_dir>/qwen-image-2.1/qwen_image_2.1_vae_bf16.safetensors \
  --model-args qwen_image_2_1_prefix_cache_type=q8_0 \
  --diffusion-fa \
  --lora-model-dir <lora_dir> \
  --listen-ip <host> --listen-port <port>
```

`<model_dir>`, `<lora_dir>`, and the listen address are the image provider's
`model_dir`, `lora_dir`, and `base_url` from the shared
[`clipforge.toml`](../config/clipforge.example.toml).
A provider that declares a `weights.diffusion` override has the server load that local file as
`--diffusion-model` instead of the profile's own tree.

`--model-args qwen_image_2_1_prefix_cache_type=q8_0` pins an explicit quantized
prefix cache (the `auto` default would pick f32); `--diffusion-fa` enables
diffusion flash attention. The lifecycle owner starts the server detached
(`keep_alive`), so its logs go to the shared state dir and it outlives the
caller; `stdin` is nulled. `is_running()`/`try_wait()` detect a crashed server on
the next readiness check so it is restarted.

### Residency and VRAM

The diffusion model, encoder, and VAE stay loaded across prompts; normal
generation never unloads them. There is a single generation profile —
`qwen-image-2.1` — so there is no profile switching.

Residency is a **launch-time** choice (`sd::Residency`), set per `sdcpp`
provider in `clipforge.toml` under `residency`, or in Settings → Services →
Residency. There is no unload endpoint, so changing it needs a restart — the
Settings control persists the shared provider's choice, and restarts a running
server through the shared owner so the new mode takes effect now.

| mode | flags | peak (t2i 1024² / edit 1024×1536) | sampling cost |
| --- | --- | ---: | --- |
| `resident` (default) | — | ≈22.7 / ≈23.3 GiB | baseline |
| `lowvram` | `--offload-to-cpu` | ≈9.2 / ≈15.5 GiB | +7% / +0.5% |

Measured on an RTX 3090, 25 steps, CFG 6.0 (`.svs-e2e/logs/2026-10-01-offload-cost.md`).
`lowvram` is nearly free at these canvases, so it is what lets a second model
share the card. `--max-vram 8` was also measured and is **not** a general
answer: it leaves t2i untouched but makes a reference edit ~2.9× slower
(99.7 s → 288.5 s) as the graph is re-cut, so it is not offered as a mode.
Qwen-Image-2.1 peaks at ≈23.4 GiB `resident`, so the LLM planner/VLM **cannot
be co-resident that way**: either move to `lowvram`, or free VRAM first.

## `ModelStore` — provisioning

`ModelStore` (in the shared `clipforge-inference` crate —
`../clipforge/crates/inference/src/model_store.rs`) downloads and verifies the
Qwen-Image-2.1 artifacts from Hugging Face:

- Resumable: an existing `<file>.part` continues with a `Range` request.
- Verified: after download, size and SHA-256 are checked before the `.part` is
  renamed into place (a profile whose checksum has not been recorded yet
  verifies by size only).
- Progress: a `FnMut(DownloadProgress)` callback reports filename and byte
  counts.

storyfella drives it through the `model_setup` binary
(`apps/cli/src/bin/model_setup.rs`), which loads the providers registry and
provisions each artifact of the image deployment's `model_root()` whose
`needs_download()` is true (a `weights` override is reported and skipped):

```bash
cargo run -p svs-cli --bin model_setup
```

The app drives the same call from Settings → Services → **Model files**
(`storyfella_studio_core::runtime::provision_image_model`, over the same `ModelStore`), listing
each artifact with its size, presence, and a byte-counted progress bar — so a
first run does not need a terminal.

> The manifests' Hugging Face `remote_path`s were verified against the live
> `Comfy-Org/Qwen-Image-2.1` repository on 2026-10-01: all three remote paths,
> sizes, and SHA-256 digests match.

Expected layout below the image provider's `model_dir` (OS-local Storyfella model
storage by default; a user-selected existing tree can override it):

```text
models/
  qwen-image-2.1/
    qwen_image_2.1_int8_convrot.safetensors   # or the provider's weights.diffusion override
    qwen3vl_8b_int8_convrot.safetensors
    qwen_image_2.1_vae_bf16.safetensors
  loras/
```

LoRA paths in generation requests are the registered top-level names within the
LoRA directory; absolute paths and parent traversal are rejected (see
[`architecture.md`](architecture.md)).

## Health status checks

The Settings **Models** and **Services** pages probe every service the application depends on
(`crates/studio-core/src/runtime/health.rs`, triggered by `Settings` in `apps/studio/src/ui/settings.rs`). The
checks are read-only: they never start or stop a process and never download
anything. Each probe runs with a short timeout and reports one of four states:

| State | Meaning | Dot |
| --- | --- | --- |
| `Online` | Ready and serving (or ready to serve when needed) | emerald |
| `Degraded` | Reachable, but with a gap that prevents full function | amber |
| `Idle` | Deliberately not running; the app starts it on demand | zinc |
| `Offline` | Unreachable or unhealthy when it should be serving | rose |

| Service | Probe | Reported as |
| --- | --- | --- |
| LLM endpoint (text or vision) | `GET /v1/models` | `Online` when reachable **and** the configured model is loaded; `Degraded` when reachable but the model is not; `Idle` when the provider is a managed `llamacpp` server that is not running (it starts on demand); `Offline` when an external endpoint is unreachable |
| Image generation (Qwen-Image-2.1) | `sd-server` binary, profile artifacts on disk, `control::status`, `GET /sdcpp/v1/capabilities` | `Offline` when the binary is missing or the resident server is unreachable; `Degraded` when the model artifacts are missing (prepare them in Settings) or the resident server holds a different model; `Online` when the resident backend reports the profile's model; `Idle` when nothing is answering |

The checks run in parallel (`tokio::join!`); the **Models** and **Services** pages
each show the last-checked time and a single **Check now** that re-probes every
service.

### On-demand start and stop

The **Services** page controls the local processes (`apps/studio/src/ui/settings.rs`,
`operate_service` for the image engine, `on_chat_operate` for chat):

- **Image generation** — **Start** runs `control::load` through `CreativeRuntime`
  (ensures the configured model is resident, replacing a server the shared owner
  started); **Stop** runs `control::stop`. The shared owner holds the process, so
  the card does not offer a separate restart or take-over: changing **Residency**
  restarts through the owner, and a foreign server is reported with the CLI
  remedy.
- **Chat servers** — each managed `llamacpp` provider gets `Start`/`Stop`/`Restart`
  (`CreativeBackend::start_chat_server` / `stop_chat_server` /
  `restart_chat_server`), so its `llama-server` can be booted from the app and its
  `/v1/models` list fetched. An `openai` provider has no controls: it is
  external. The same servers are also started on demand — the planner via
  `ensure_planner_ready` before an LLM call, vision via a `VisionSession` for an
  eval — and `stop_chat_servers` stops every managed chat server before a
  generation, so no chat model shares the card with the image engine.

While an operation runs, its row shows a spinner and the other buttons are
disabled. When it finishes — successfully or not — a fresh health check runs
immediately; failures appear in an error banner above the list. The controls
use the same `CreativeRuntime` methods as generation, so manual starts and
automatic starts behave identically.

## Known gaps

- The shared server is intentionally left resident when the app exits, so the
  next front end reuses it. It is stopped from Settings → Services → **Stop** or
  with `svs runtime stop`; a hand-run server outside the shared owner needs
  `svs runtime stop --force`.
- The shared owner coordinates one machine; two `model_dir`s that resolve the
  same `base_url` would still collide, but the shared `clipforge.toml` is what
  every front end reads, so this needs an explicit per-provider override.

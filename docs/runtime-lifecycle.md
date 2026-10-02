# Runtime lifecycle

Rust supervises one child process and one external service:

| Process | Rust owner | Port | Started when |
| --- | --- | --- | --- |
| `sd-server` (stable-diffusion.cpp) | `sd::Runtime` in the shared `clipforge-inference` crate (`../clipforge/crates/inference/src/sd/runtime.rs`) | the image provider's `base_url` (`127.0.0.1:7861` in the checked-in `providers.toml`) | First generation request, or manually from Settings |
| LLM endpoint | external, user-run | user-configured (`config/app.toml`) | external |

The desktop app does **not** start `sd-server` at launch. The shell reports it as
idle until a generation request triggers `CreativeRuntime::ensure_ready`. This
keeps a quiet app quiet and delays the multi-GiB model load until it is needed.

The process, client, model store, and profile are all owned by the sibling
`clipforge` project's crate, package **`clipforge-inference`**, pulled in as a
**path dependency** (`clipforge-inference = { path = "../clipforge/crates/inference" }`
in the root `[workspace.dependencies]`); the sibling checkout must be present at
that relative path for storyfella to build, and the crate is not a workspace
member. `crates/core/src/runtime/` holds only the adapter: `CreativeRuntime`
(`creative_runtime.rs`), the health probes (`health.rs`), and `mod.rs`.

## `CreativeRuntime` facade

`CreativeRuntime` (`crates/core/src/runtime/creative_runtime.rs`) is the single entry point
used by the UI. It owns:

- an `sd::Client` for the native `sd-server` (from the shared `clipforge-inference` crate),
- a shared `sd::Runtime` (the resident process supervisor),
- the generated-asset directory (`<asset_dir>/generated`).

### Readiness — `ensure_profile_ready`

```text
ensure_profile_ready()
    │
    ├─ GET /sdcpp/v1/capabilities answers, and the resident model matches?  ──▶ Ok
    │    (a mismatched server this process owns is stopped and restarted; a
    │     mismatched server owned by another session → ProfileMismatch)
    │
    ├─ sd::Runtime::ensure_ready(<host>, <port>)
    │    (the image provider's `base_url` in providers.toml)
    │
    └─ Poll /sdcpp/v1/capabilities every 500 ms until the model matches,
       or a 600 s deadline → ReadinessTimeout
```

The model check is two-phase: capabilities answer "is the HTTP server up" and
"is the right model actually resident". Loading ≈17.3 GiB of weights takes
minutes, so the readiness cap stays generous (600 s). The Rust side never treats
process creation as readiness. A `ProfileMismatch` names the loaded model
(compared by file name or stem) and the fix (`svs runtime serve --force` or
`pkill -f "sd-server --diffusion"`).

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
`../clipforge/crates/inference/src/sd/runtime.rs`) starts the process. It is
defensive:

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
`model_dir`, `lora_dir`, and `base_url` from [`providers.toml`](../providers.toml).
A provider that declares a `weights.diffusion` override — the checked-in file
points at the uncensored int8 DiT — has the server load that local file as
`--diffusion-model` instead of the profile's own tree.

`--model-args qwen_image_2_1_prefix_cache_type=q8_0` pins an explicit quantized
prefix cache (the `auto` default would pick f32); `--diffusion-fa` enables
diffusion flash attention. Child `stdin` is nulled and `stdout`/`stderr` are
inherited, so the server's logs appear in the app's terminal; `kill_on_drop(true)`
plus an explicit `stop()` owns the child's lifetime. `is_running()` uses
`try_wait()` and clears the slot if the child exited, so a crashed server is
detected on the next readiness check and restarted.

### Residency and VRAM

The diffusion model, encoder, and VAE stay loaded across prompts; normal
generation never unloads them. There is a single generation profile —
`qwen-image-2.1` — so there is no profile switching.

Residency is a **launch-time** choice (`sd::Residency`), set per `sdcpp`
provider in `providers.toml` under `residency`, or in Settings → Image
generation → Residency. There is no unload endpoint, so changing it restarts
the server; the Settings control persists the choice and restarts a running
server into the new mode.

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

The app drives the same call from Settings → Image generation → **Model files**
(`svs_core::runtime::provision_image_model`, over the same `ModelStore`), listing
each artifact with its size, presence, and a byte-counted progress bar — so a
first run does not need a terminal.

> The manifests' Hugging Face `remote_path`s were verified against the live
> `Comfy-Org/Qwen-Image-2.1` repository on 2026-10-01: all three remote paths,
> sizes, and SHA-256 digests match.

Expected layout below the image provider's `model_dir` (the checked-in
`providers.toml` roots it at `models/`):

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

The Settings **Status** section probes every service the application depends on
(`crates/core/src/runtime/health.rs`, triggered by `Settings` in `apps/desktop/src/ui/settings.rs`). The
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
| LLM endpoint | `GET /v1/models` | `Online` when reachable **and** the configured model is loaded; `Degraded` when reachable but the model is not; `Offline` when unreachable |
| Image generation (Qwen-Image-2.1) | `sd-server` binary, profile artifacts on disk, process state, `GET /sdcpp/v1/capabilities` | `Offline` when the binary is missing or the resident server is unreachable; `Degraded` when the model artifacts are missing (`run model_setup`); `Online` when the resident backend reports the profile's model; `Idle` when the process is on-demand and not started |

The checks run in parallel (`tokio::join!`); the Status section shows the
last-checked time and can be re-run with **Check now** or the **Test
connection** button in Intelligence.

### On-demand start and stop

The Status section controls the local runtime (`apps/desktop/src/ui/settings.rs`,
`operate_service`):

- **Image generation** — `Start` runs `ensure_profile_ready` (starts `sd-server`
  and waits for the model to be resident); `Stop` kills the native server.
- **LLM endpoint** has no controls: it is external.

While an operation runs, its row shows a spinner and the other buttons are
disabled. When it finishes — successfully or not — a fresh health check runs
immediately; failures appear in an error banner above the list. The controls
use the same `CreativeRuntime` methods as generation, so manual starts and
automatic starts behave identically.

## Known gaps

- Application shutdown stops the child with a bounded blocking kill, but only
  from the app's own exit points; a `SIGKILL`ed app still leaves the server to
  be adopted or replaced (see TODO §7).

# Runtime lifecycle

Rust supervises one child process and one external service:

| Process | Rust owner | Port | Started when |
| --- | --- | --- | --- |
| `sd-server` (stable-diffusion.cpp) | `sd::Runtime` in the shared `clipforge-inference` crate (`../clipforge/crates/inference/src/sd/runtime.rs`) | `127.0.0.1:7861` | First generation request, or manually from Settings |
| LM Studio | external, user-run | user-configured (`config/app.toml`) | external |

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
    ├─ sd::Runtime::ensure_ready("127.0.0.1", 7861)
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
3. Verifies all three artifacts for the Qwen-Image-2.1 profile exist under
   `<model_dir>/qwen-image-2.1/` (diffusion safetensors, text encoder, VAE).
4. Creates the LoRA directory and runs `prepare_lora_links` (sd.cpp registers
   only the *top-level* files of its lora directory, so subdirectory adapters are
   symlinked to the top level as `<subdir>-<basename>` before the server's
   startup scan).
5. Spawns with explicit arguments:

```text
sd-server \
  --diffusion-model <model_dir>/qwen-image-2.1/qwen-image-2.1-UC-int8_convrot.safetensors \
  --llm             <model_dir>/qwen-image-2.1/qwen3vl_8b_int8_convrot.safetensors \
  --vae             <model_dir>/qwen-image-2.1/qwen_image_2.1_vae_bf16.safetensors \
  --model-args qwen_image_2_1_prefix_cache_type=q8_0 \
  --diffusion-fa \
  --lora-model-dir <model_dir>/loras \
  --listen-ip 127.0.0.1 --listen-port 7861
```

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
`qwen-image-2.1` — so there is no profile switching. Qwen-Image-2.1 peaks at
≈23.4 GiB on a 24 GiB card (weights resident ≈15.9 GB), so the LM Studio
planner/VLM **cannot be resident during a generation**: free VRAM, then generate.

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
(`apps/cli/src/bin/model_setup.rs`), which resolves the profile with
`sd::qwen_image_2_1()` and provisions into `model_root_dir(model_dir)`:

```bash
cargo run -p svs-cli --bin model_setup            # into paths.model_dir
cargo run -p svs-cli --bin model_setup -- --model-dir <dir>
```

> The manifests' Hugging Face `remote_path`s are best-effort: the SHA-256 and
> size are pinned, but the exact repository path has not been re-confirmed
> against the live repo. Verify a download before relying on it.

Expected layout below the configured model directory:

```text
models/
  qwen-image-2.1/
    qwen-image-2.1-UC-int8_convrot.safetensors
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
| LM Studio | `GET /v1/models` | `Online` when reachable **and** the configured model is loaded; `Degraded` when reachable but the model is not; `Offline` when unreachable |
| Image generation (Qwen-Image-2.1) | `sd-server` binary, profile artifacts on disk, process state, `GET /sdcpp/v1/capabilities` | `Offline` when the binary is missing or the resident server is unreachable; `Degraded` when the model artifacts are missing (`run model_setup`); `Online` when the resident backend reports the profile's model; `Idle` when the process is on-demand and not started |

The checks run in parallel (`tokio::join!`); the Status section shows the
last-checked time and can be re-run with **Check now** or the **Test
connection** button in Intelligence.

### On-demand start and stop

The Status section controls the local runtime (`apps/desktop/src/ui/settings.rs`,
`operate_service`):

- **Image generation** — `Start` runs `ensure_profile_ready` (starts `sd-server`
  and waits for the model to be resident); `Stop` kills the native server.
- **LM Studio** has no controls: it is external.

While an operation runs, its row shows a spinner and the other buttons are
disabled. When it finishes — successfully or not — a fresh health check runs
immediately; failures appear in an error banner above the list. The controls
use the same `CreativeRuntime` methods as generation, so manual starts and
automatic starts behave identically.

## Known gaps

- Application shutdown does not yet gracefully stop the child (Rust relies on
  `kill_on_drop`); wiring graceful shutdown is planned with session management.
- Download progress is not surfaced in the Settings UI; `model_setup` is the
  only progress consumer.

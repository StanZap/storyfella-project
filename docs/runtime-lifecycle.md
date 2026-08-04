# Runtime lifecycle

Rust supervises two child processes and one external service:

| Process | Rust owner | Port | Started when |
| --- | --- | --- | --- |
| `python/main.py` (uvicorn) | `PythonRuntime` (`src/runtime/mod.rs`) | `127.0.0.1:8765` | First generation request |
| `sd-server` (stable-diffusion.cpp) | `GenerationRuntime` (`src/runtime/generation_runtime.rs`) | `127.0.0.1:7861` | First generation request |
| LM Studio | external, user-run | user-configured (`config/app.toml`) | external |

The desktop app does **not** start either child at launch. The shell reports
the vision runtime as idle until a generation request triggers
`CreativeRuntime::ensure_ready`. This keeps a quiet app quiet and delays
multi-GiB model loads until they are needed.

## `CreativeRuntime` facade

`CreativeRuntime` (`src/runtime/creative_runtime.rs`) is the single entry point
used by the UI. It owns:

- a `VisionClient` for the Python runtime,
- a shared `PythonRuntime`,
- a shared `GenerationRuntime`,
- the configured `KreaQuantization` profile,
- the generated-asset directory (`<asset_dir>/generated`).

### Readiness — `ensure_ready`

```text
ensure_ready()
    │
    ├─ Already healthy AND /generation/capabilities == "ready"?  ──▶ return Ok
    │
    ├─ generation.is_running()? no → GenerationRuntime::start(profile, 127.0.0.1, 7861)
    ├─ python.is_running()?      no → PythonRuntime::start("127.0.0.1", 8765)
    │
    └─ Poll /health + /generation/capabilities every 200 ms
         until both succeed, or 90 s deadline → ReadinessTimeout
```

The two-phase check matters: `/health` answers "is the HTTP server up", while
`/generation/capabilities` answers "is the resident model actually loaded".
Capabilities reports `status: "unavailable"` while sd.cpp is still loading its
weights, and `"ready"` once the model is resident. The Rust side never treats
process creation as readiness.

### Job waiting — `wait_for_job`

After `POST /generation/jobs` returns a `queued`/`generating` job, the facade
polls `GET /generation/jobs/{id}` every 150 ms until the job reaches a terminal
state or the 180-second deadline expires. `failed` and `cancelled` map to
`CreativeRuntimeError::Job` with the backend's message.

### Asset import — `import_asset`

Completed jobs reference files in `SVS_GENERATED_DIR` (the Python adapter's
scratch space). The facade copies the result into
`<asset_dir>/generated/<uuid>.png` and returns the **canonical absolute path**,
which the UI stores on the frame/revision and serves back through the
`generated` asset handler (`/generated/<file>`).

## `PythonRuntime`

`PythonRuntime` resolves the interpreter at `<python_runtime>/.venv/bin/python`
(or `Scripts/python.exe` on Windows), then spawns:

```text
python main.py --host 127.0.0.1 --port 8765
```

Details:

- Child `stdout`/`stderr` are inherited, so the runtime's logs appear in the
  app's terminal.
- `kill_on_drop(true)` plus an explicit `stop()` (SIGKILL + wait) owns the
  child's lifetime.
- `is_running()` uses `try_wait()` and clears the slot if the child exited, so
  a crashed runtime is detected on the next readiness check and restarted.
- Rust sets `SVS_SD_CPP_URL` from `generation.base_url` so the Python adapter
  finds sd.cpp without hardcoding the port.

## `GenerationRuntime` (sd.cpp)

`GenerationRuntime::start(profile, host, port)` is defensive:

1. Refuses to start twice (`AlreadyRunning`).
2. Verifies the `sd-server` executable exists.
3. Verifies all three artifacts for the profile exist under
   `<model_dir>/krea-2/` (diffusion GGUF, Qwen3-VL encoder GGUF, Wan VAE).
4. Creates the LoRA directory.
5. Spawns with explicit arguments:

```text
sd-server \
  --diffusion-model <model_dir>/krea-2/<diffusion.gguf> \
  --llm           <model_dir>/krea-2/Qwen3VL-4B-Instruct-Q4_K_M.gguf \
  --vae           <model_dir>/krea-2/wan_2.1_vae.safetensors \
  --lora-model-dir <model_dir>/loras \
  --lora-apply-mode at_runtime \
  --diffusion-fa \
  --listen-ip 127.0.0.1 --listen-port 7861
```

`--diffusion-fa` enables diffusion flash attention; CPU offloading and VAE
tiling are deliberately not enabled (see
[`docs/poc/native-generation/README.md`](poc/native-generation/README.md) for
the measured trade-off).

### Residency and profile switching

The diffusion model, encoder, and VAE stay loaded across prompts; normal
generation never unloads them. One server context owns one diffusion
checkpoint, so changing Q2 ↔ Q4 stops and restarts **only** the native server
(`switch_profile`). The Python adapter detects a mismatch via
`StableDiffusionCppClient::assert_profile` and refuses jobs rather than
generating with the wrong weights.

Profiles:

| Profile | Diffusion artifact | Approx. weights |
| --- | --- | --- |
| `krea-2-turbo-q2` | `krea2_turbo-q2_k.gguf` | 6.5 GiB on disk |
| `krea-2-turbo-q4` | `krea2_turbo-iq4_xs.gguf` | 8.9 GiB on disk |

Both share the Qwen3-VL 4B Q4_K_M encoder and `wan_2.1_vae.safetensors`.
Manifests are pinned with repository, revision, exact size, and SHA-256 in both
`src/runtime/generation_runtime.rs` (Rust) and `python/runtime/model_manifest.py`
(Python), so the two sides cannot silently diverge.

## `ModelStore` — provisioning

`ModelStore` (`src/runtime/model_store.rs`) downloads and verifies the profile
artifacts from Hugging Face:

- Resumable: an existing `<file>.part` continues with a `Range` request.
- Verified: after download, size and SHA-256 are checked before the `.part` is
  renamed into place. Verified shared artifacts (encoder, VAE) are reused
  across profiles.
- Progress: a `FnMut(DownloadProgress)` callback reports filename and byte
  counts; `model_setup` prints percent updates.

The `model_setup` binary drives it:

```bash
cargo run --bin model_setup -- --profile q2 --accept-krea-license
```

Expected layout below the configured model directory:

```text
models/
  krea-2/
    krea2_turbo-q2_k.gguf
    krea2_turbo-iq4_xs.gguf
    Qwen3VL-4B-Instruct-Q4_K_M.gguf
    wan_2.1_vae.safetensors
  loras/
```

LoRA paths in generation requests are relative to the LoRA directory; absolute
paths and parent traversal are rejected (see [`http-api.md`](http-api.md)).

## Environment variables

The Python runtime reads these (set by the Rust parent or for manual runs):

| Variable | Purpose | Default |
| --- | --- | --- |
| `SVS_SD_CPP_URL` | sd.cpp base URL | `http://127.0.0.1:7861` |
| `SVS_GENERATED_DIR` | Where completed jobs write PNGs | `<repo>/cache/generated` |
| `SVS_MASK_DIR` | Where `/segment` writes masks | `<repo>/cache/masks` |

## Known gaps

- The UI reports the runtime as idle until first use; there is no manual
  start/stop surface in the shell.
- Application shutdown does not yet gracefully stop children (Rust relies on
  `kill_on_drop`); wiring graceful shutdown is planned with session
  management.
- Download progress is not surfaced in the Settings UI; `model_setup` is the
  only progress consumer.

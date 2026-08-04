# Native Krea 2 generation

This proof of concept uses `stable-diffusion.cpp` as a small native inference runtime. It is not ComfyUI and does not require ComfyUI. The runtime provides Metal and CUDA backends, GGUF loading, request-time LoRA selection, and a persistent HTTP server.

## Profiles

| Profile | Diffusion weights | Shared text encoder | Approx. weight files |
| --- | --- | --- | ---: |
| `krea-2-turbo-q2` | `krea2_turbo-q2_k.gguf` | Qwen3-VL 4B Q4_K_M | 6.5 GiB |
| `krea-2-turbo-q4` | `krea2_turbo-iq4_xs.gguf` | Qwen3-VL 4B Q4_K_M | 8.9 GiB |

Both profiles use `wan_2.1_vae.safetensors`. The manifests in Rust and Python pin each source repository revision, exact size, and SHA-256 hash. "Q4" currently resolves to the available IQ4_XS diffusion artifact, not Q4_K. The encoder is the Q4_K_M file recommended by the official `stable-diffusion.cpp` Krea 2 guide.

The file totals fit comfortably below the 24 GiB target. That is not itself a peak-memory guarantee: activations, the compute graph, image dimensions, and backend behavior also consume memory. The checked-in configuration enables diffusion flash attention and deliberately avoids both CPU offloading and default VAE tiling, so the warm path favors latency and residency. VAE tiling remains an opt-in fallback when a larger workload demonstrates real memory pressure.

## Runtime lifecycle

Rust owns `sd-server` and starts it with one active profile. The diffusion model, quantized Qwen3-VL encoder, and VAE stay loaded across prompts. A normal generation never unloads them. Changing Q2 to Q4 or Q4 to Q2 explicitly stops and restarts only this native runtime because one server context owns one diffusion checkpoint.

The Python runtime is an HTTP adapter and result store. It exposes:

- synchronous `POST /generate` for simple callers;
- `POST /generation/jobs?priority=interactive|background` for non-blocking work;
- `GET /generation/jobs/{id}` for polling;
- `POST /generation/jobs/{id}/cancel` for cancellation while queued.

`stable-diffusion.cpp` serializes inference on its resident context. Queued work can be cancelled; an already-generating diffusion graph cannot currently be preempted safely. The `priority` field is retained for the application scheduler, which should hold speculative background work until no interactive request is waiting.

## Build the native runtime

On Apple Silicon:

```bash
git clone --recursive https://github.com/leejet/stable-diffusion.cpp.git
git -C stable-diffusion.cpp checkout db99efdd6d2a43c7937fd55b3359206c680a75b0
git -C stable-diffusion.cpp submodule update --init --recursive
cmake -S stable-diffusion.cpp -B stable-diffusion.cpp/build \
  -DSD_METAL=ON -DSD_SERVER_BUILD_FRONTEND=OFF -DCMAKE_BUILD_TYPE=Release
cmake --build stable-diffusion.cpp/build --target sd-server --config Release -j
```

On Linux/CUDA, replace `-DSD_METAL=ON` with `-DSD_CUDA=ON`.

Place the resulting binary at `models/runtime/stable-diffusion.cpp/bin/sd-server`, or set `generation.executable` in `config/app.toml`.

Probe a running profile twice to separate first-request and warm latency:

```bash
cd python
SVS_SD_CPP_URL=http://127.0.0.1:7861 .venv/bin/python -m scripts.krea_generation_probe \
  "A cinematic storyboard frame of a lighthouse in a storm, no text" \
  --profile krea-2-turbo-q2 --repeat 2
```

## Model layout

Review the Krea license, then let the Rust model store download, resume, and verify the selected profile:

```bash
cargo run --bin model_setup -- --profile q2 --accept-krea-license
cargo run --bin model_setup -- --profile q4 --accept-krea-license
```

`--model-dir` overrides the platform directory for development. Q2 and Q4 share the encoder and VAE, so verified shared artifacts are not downloaded twice.

Rust expects these files below its configured model directory:

```text
models/
  krea-2/
    krea2_turbo-q2_k.gguf
    krea2_turbo-iq4_xs.gguf
    Qwen3VL-4B-Instruct-Q4_K_M.gguf
    wan_2.1_vae.safetensors
  loras/
```

LoRA requests contain paths relative to `models/loras`; absolute paths and parent traversal are rejected. Native server APIs intentionally disable prompt-embedded LoRA parsing, so adapters are explicit request data. Quantized models use `at_runtime` LoRA application and remain resident.

Krea 2 uses its community license. Review the license, revenue threshold, distribution requirements, and content-policy obligations before shipping or distributing weights.

## Apple Silicon live result

The first native probe ran on an Apple M3 Max with 48 GB unified memory. This validates Metal compatibility and model residency, but it is not yet the required hard-cap test on a 24 GB host. The runtime's own model-memory report and end-to-end timings were:

| Profile | Resident model VRAM | Probe | First request | Warm request |
| --- | ---: | --- | ---: | ---: |
| Q2_K | 7,815 MB | 512², 4 steps | 40.6 s | 34.9 s |
| IQ4_XS | 10,298 MB | 512², 2 steps | 35.0 s | 26.9 s |
| IQ4_XS | 10,298 MB | 512², 4 steps | — | 34.8 s |

For Q2, warm text conditioning fell to 0.1 seconds and no model reload occurred. Default VAE tiling was rejected after measurement: it increased a warm Q2 run from 34.9 to 58.2 seconds at 512². The untiled Wan VAE still took about 18 seconds, making VAE acceleration and preview decoding the largest immediate latency opportunity. The reported VRAM figures cover resident parameters, not peak activation memory.

Representative four-step outputs:

![Krea 2 Q2 lighthouse](latest/krea2-q2-lighthouse-seed-43.png)

![Krea 2 Q4 lighthouse](latest/krea2-q4-lighthouse-seed-64.png)

## Remaining empirical gates

- Record cold start, first image, and warm image latency for Q2 and Q4 on both target platforms.
- Enforce the 24 GB gate on a 24 GB host and record peak Metal allocation and `nvidia-smi` process memory at 512, 768, and 1024 square.
- Verify representative LoRAs trained on Krea 2 Raw against the Turbo runtime.
- Add an interactive-first scheduler ahead of native submission and coalescing for superseded edit prompts.
- Add resumable, checksum-verified downloads and progress UI to Rust's `ModelStore`.

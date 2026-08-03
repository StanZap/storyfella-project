# Image-generation proof of concept

This spike validates local text-to-image inference behind the existing Python HTTP boundary. It deliberately does not connect generation to the timeline or application state.

## Baseline

The initial baseline is `Efficient-Large-Model/Sana_Sprint_0.6B_1024px_diffusers`: a small, Apache-2.0 SANA Sprint checkpoint intended for 1024px output in very few inference steps. The model uses a Gemma text encoder, whose separate license also applies. Confirm both licenses before distributing model weights.

The adapter chooses CUDA on Linux, MPS on Apple Silicon, and CPU only as a fallback. Dependencies live in the optional `image-generation` extra, and the pipeline is loaded on the first `/generate` request. `/health` remains fast and does not load model weights.

## Setup and probe

From the repository root:

```sh
uv sync --project python --extra image-generation
cd python
HF_HOME=../models/huggingface uv run python -m scripts.image_generation_probe \
  "A cinematic storyboard frame of a red lighthouse above a stormy sea, no text" \
  --device auto --steps 2 --seed 42
```

Generated images are written to `cache/generated` unless `SVS_GENERATED_DIR` is set. Model downloads go through Hugging Face during this POC only; production should have Rust provision and verify the model directory before starting Python.

## HTTP contract

- `GET /capabilities` reports Torch, CUDA, and MPS availability without loading a model.
- `POST /generate` accepts `prompt`, `width`, `height`, `steps`, `seed`, `model`, and `device`.
- The response includes the output path, resolved runtime metadata, duration, and a stable `completed` or `failed` status.

## Deliberate limitations

- Requests are serialized because a single pipeline owns most accelerator memory.
- There is no cancellation, progress stream, model integrity manifest, or warm-up policy yet.
- Quality evaluation is manual in this spike; the later evaluator should use deterministic fixtures and record both semantic and visual-quality metrics.
- SAM integration remains separate so segmentation and generation can evolve independently.

## First local result

On an Apple M3 Max with 48 GB unified memory, the baseline produced the checked-in `latest/lighthouse-seed-42.png` at 1024×1024 using MPS, BF16, two inference steps, and seed 42. The measured duration was 19.831 seconds including initial pipeline load. This is a feasibility measurement, not a benchmark; warm-run latency and a broader prompt suite still need measurement.

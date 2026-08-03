# Segmentation proof of concept

This spike validates promptable mask generation behind the Python `/segment` boundary. The initial backend is Meta SAM 2.1 Tiny through Hugging Face Transformers.

## Why SAM 2.1 first

SAM 2.1 is ungated, has an Apache-2.0 checkpoint, and provides point- and box-prompted image segmentation using the same class of mask contract needed by the sequencer. The Tiny checkpoint has 38.9 million parameters and is practical for an Apple Silicon proof of concept.

SAM 3.1 remains a future CUDA backend. Meta's official implementation currently lists CUDA 12.6+ as a prerequisite, requires authenticated checkpoint access, and does not document MPS support. Its headline 3.1 improvement is multiplexed multi-object video tracking, which is not required for the first static-storyboard workflow.

Primary references:

- [Meta SAM 2 repository and checkpoint table](https://github.com/facebookresearch/sam2)
- [Transformers SAM 2 documentation](https://huggingface.co/docs/transformers/model_doc/sam2)
- [Meta SAM 3.1 installation and access requirements](https://github.com/facebookresearch/sam3)
- [SAM 3.1 release notes](https://github.com/facebookresearch/sam3/blob/main/RELEASE_SAM3p1.md)

## Setup and probe

```sh
uv sync --project python --extra segmentation
cd python
HF_HOME=../models/huggingface uv run python -m scripts.segmentation_probe \
  ../docs/poc/image-generation/latest/lighthouse-seed-42.png \
  --box 430 70 600 620 --device auto
```

The adapter accepts either one or more boxes, or a set of positive/negative points describing one object. Text-only requests first pass through the separate Grounding DINO adapter; SAM 2.1 itself remains geometry-only.

Masks are written to `cache/masks` unless `SVS_MASK_DIR` is set. As with the image-generation POC, direct Hugging Face downloads are temporary development behavior; production model acquisition and verification remain Rust responsibilities.

## First local result

On an Apple M3 Max, SAM 2.1 Tiny isolated the generated lighthouse at 1024×1024 using MPS and float32. The selected mask scored 0.966, covered 36,445 pixels, and completed in 9.433 seconds including model load. See `latest/lighthouse-mask.png` and `latest/result.json`.

## Next measurements

- Warm inference latency and repeated-image embedding reuse.
- Point prompts, negative refinement points, and multiple boxes.
- Broader text-grounding fixtures, evaluated independently from mask quality.
- SAM 3.1 on Linux/CUDA after checkpoint access is approved.

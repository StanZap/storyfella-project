# Proofs of concept

These documents record what was learned while validating each ML backend
before it became part of the product. They are historical and empirical: they
describe measurements, decisions, and remaining gates, not the current product
surface. For how the pieces fit together today, start at
[`../README.md`](../README.md).

| POC | What it validated | Key result |
| --- | --- | --- |
| [`vlm/README.md`](vlm/README.md) | Vision-capable models through LM Studio's chat-completions API | Deterministic fixture scoring and per-request latency capture; the `vlm_probe` binary |
| [`image-generation/README.md`](image-generation/README.md) | SANA Sprint Diffusers pipeline behind the Python `/generate` contract | First checked-in generated image; device auto-selection |
| [`native-generation/README.md`](native-generation/README.md) | Resident Krea 2 Turbo via `stable-diffusion.cpp` | Q2/Q4 profiles, Metal/CUDA, first-request vs warm latency, remaining memory gates |
| [`segmentation/README.md`](segmentation/README.md) | SAM 2.1 Tiny behind the Python `/segment` contract | Point/box prompting on MPS; SAM decision rationale |
| [`grounded-segmentation/README.md`](grounded-segmentation/README.md) | Text grounding with Grounding DINO feeding SAM | Connected pipeline result and scoring |

The current product wiring lives in `src/` (Rust) and `python/` (FastAPI); see
[`architecture.md`](../architecture.md) for the boundary between them.

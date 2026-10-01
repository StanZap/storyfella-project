# Proofs of concept

These documents record what was learned while validating each ML backend
before it became part of the product. They are historical and empirical: they
describe measurements, decisions, and remaining gates, not the current product
surface. For how the pieces fit together today, start at
[`../README.md`](../README.md) and [`../architecture.md`](../architecture.md).

| POC | What it validated | Status |
| --- | --- | --- |
| [`vlm/README.md`](vlm/README.md) | Vision-capable models through the LLM endpoint's chat-completions API | Current (`vlm_probe` / `svs eval`) |
| [`image-generation/README.md`](image-generation/README.md) | SANA Sprint Diffusers pipeline behind the Python `/generate` contract | Removed (Qwen-Image-2.1 migration) |
| [`native-generation/README.md`](native-generation/README.md) | Resident generation via `stable-diffusion.cpp` | Superseded — now Qwen-Image-2.1 |
| [`segmentation/README.md`](segmentation/README.md) | SAM 2.1 Tiny behind the Python `/segment` contract | Removed (Qwen-Image-2.1 migration) |
| [`grounded-segmentation/README.md`](grounded-segmentation/README.md) | Text grounding with Grounding DINO feeding SAM | Removed (Qwen-Image-2.1 migration) |

The current product wiring is Rust only — `crates/core`, `apps/desktop`, and
`apps/cli` — plus the native `sd-server`; see
[`architecture.md`](../architecture.md) for the boundary.

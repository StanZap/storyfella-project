# Grounded segmentation proof of concept

This slice connects text grounding to mask refinement entirely behind `POST /segment`:

```text
text prompt → Grounding DINO Tiny → bounding boxes → SAM 2.1 Tiny → mask PNGs
```

Grounding DINO is a dedicated open-set detector rather than a general Visual LLM. The official Transformers documentation explicitly describes combining it with Segment Anything as Grounded SAM. Both the implementation and selected checkpoint are Apache-2.0 licensed.

Primary references:

- [Transformers Grounding DINO documentation](https://huggingface.co/docs/transformers/model_doc/grounding-dino)
- [Official Grounding DINO repository](https://github.com/IDEA-Research/GroundingDINO)

## Run

```sh
uv sync --project python --extra segmentation
cd python
HF_HOME=../models/huggingface uv run python -m scripts.segmentation_probe \
  ../docs/poc/image-generation/latest/lighthouse-seed-42.png \
  --prompt lighthouse --device auto
```

The response reports detector boxes and scores separately from final masks and mask scores. `duration_ms` covers grounding, both model loads, mask inference, and persistence.

## First local result

On an Apple M3 Max, the detector returned a tight lighthouse box with score 0.927. SAM refined it to a 35,849-pixel mask with score 0.964. A fresh process using cached weights completed the full chain in 7.934 seconds on MPS.

The earlier Gemma 4 experiment produced structurally valid JSON but a materially incorrect box, so general Visual-LLM coordinate output is not used as the production grounding path.

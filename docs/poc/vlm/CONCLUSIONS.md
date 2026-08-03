# Visual LLM POC conclusions

Run date: 2026-08-03. Full per-request payloads and measurements are in [`latest/results.json`](latest/results.json), with the generated table in [`latest/report.md`](latest/report.md).

## Recommendation

Use `google/gemma-4-e4b` as the initial development baseline for storyboard analysis and visual evaluation.

- It produced schema-valid output for all three fixtures.
- It passed all 12 semantic checks.
- Average observed latency was approximately 9.4 seconds, including local inference overhead.
- Its object labels, colors, positions, and aggregate counts were coherent across the fixture set.

Do not use `glm-ocr` as the general visual evaluator. It was fast (approximately 1.9 seconds average) and described the landscape well, but it returned nonsensical structured object data on basic geometry, including a count of 75 for a single shape and double-counting the squares. It remains a candidate for a separate OCR-specific adapter.

The tested `gemma-4-12b-it-uncensored` build was slower (approximately 32.9 seconds average) and spent the entire 1,400-token completion budget without emitting final JSON for one fixture. It does not offer an advantage over E4B in this initial workload.

## Limits of this result

- The fixtures are intentionally synthetic and establish contract correctness, counting, color, and spatial grounding—not photographic understanding.
- Latency includes model switching and possible LM Studio JIT loading; it is not a controlled throughput benchmark.
- The run did not measure peak memory or tokens per second because those metrics are not exposed by the OpenAI-compatible response used by the production client.
- The remaining downloaded vision models have not yet been compared.

## Next validation

Before integrating the evaluator into the UI, add a small licensed photographic fixture set covering people, foreground/background separation, occlusion, text, and composition. Re-run E4B alongside one strong alternate model, then freeze the first version of the storyboard-analysis schema.

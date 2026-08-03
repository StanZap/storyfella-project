# Visual LLM comparison

Generated at Unix time `1785784415` against `http://localhost:1234/v1`. Scores are deterministic semantic checks over schema-valid responses, not perceptual-quality judgments.

| Model | Fixture | Status | Score | Latency | Tokens |
|---|---|---:|---:|---:|---:|
| `google/gemma-4-e4b` | colors-and-shapes | `Passed` | 100% (4/4) | 5940 ms | 164+399 |
| `google/gemma-4-e4b` | simple-landscape | `Passed` | 100% (4/4) | 14796 ms | 164+1037 |
| `google/gemma-4-e4b` | counting-grid | `Passed` | 100% (4/4) | 7439 ms | 164+502 |
| `gemma-4-12b-it-uncensored` | colors-and-shapes | `Passed` | 100% (4/4) | 19333 ms | 164+557 |
| `gemma-4-12b-it-uncensored` | simple-landscape | `Passed` | 100% (4/4) | 31098 ms | 164+1014 |
| `gemma-4-12b-it-uncensored` | counting-grid | `InvalidJson` | 0% (0/4) | 48179 ms | 164+1400 |
| `glm-ocr` | colors-and-shapes | `Passed` | 0% (0/4) | 2583 ms | 379+115 |
| `glm-ocr` | simple-landscape | `Passed` | 100% (4/4) | 1565 ms | 379+284 |
| `glm-ocr` | counting-grid | `Passed` | 75% (3/4) | 1537 ms | 379+276 |

## Failures

- `gemma-4-12b-it-uncensored` / `counting-grid`: EOF while parsing a value at line 1 column 0

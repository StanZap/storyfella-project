# Visual LLM proof of concept

This probe exercises image understanding through LM Studio's OpenAI-compatible API. It is deliberately separate from application business logic while reusing the production `LmStudioClient`.

Generate the deterministic fixtures:

```bash
cargo run --example generate_vlm_fixtures
```

List models visible through LM Studio:

```bash
cargo run --bin vlm_probe -- --list-models
```

Run one or more vision-capable models sequentially:

```bash
cargo run --bin vlm_probe -- \
  --model google/gemma-4-e4b \
  --model gemma-4-12b-it-uncensored
```

The probe writes `latest/results.json` for programmatic inspection and `latest/report.md` for a concise comparison. Its score checks fixture-specific semantic invariants such as object color, position, and aggregate count in a schema-valid response. It is a regression signal, not a substitute for human visual-quality evaluation.

LM Studio may JIT-load downloaded models. The first request can therefore take materially longer than later requests and is included in the reported latency.

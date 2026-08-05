# Project status — current state of changes

A living document tracking what is implemented, what changed recently, and
what is deferred. Source of truth for the design: `docs/artifact-canvas.md`;
slice details and CLI reference: `docs/api-slice-1.md`.

Updated: 2026-08-04.

## What is implemented and working

| Area | State |
| --- | --- |
| Artifact registry (`src/registry/`) | Unified id space (story/scene/beat/character/environment/object), variants with axes, beat compositions (backdrop/baked/dynamic layers), revisions with attached masks, drafts, operation log, snapshot/restore undo |
| `c:<name>` references | Memorable names are the primary ref form (case-insensitive, ambiguity rejected); UUIDs and 8-hex short ids still resolve. Auto-derived keys are slugs (`mia-a-lighthouse-keeper`); variants auto-name `{base}-{axis}` with `-2` dedup |
| Slice-1 operations | `create`, `variant`, `regenerate`, `compose`, `draft`, `modify` — typed, compiled, logged, checkpoint-gated |
| Pipeline builder (`src/registry/pipeline.rs`) | Closed step vocabulary, typed intermediates, static validation at `build()`, linear fail-fast stacks, checkpoints (mask confirm, text approval), LLM steps as soft dependencies |
| Composite fallback | The guaranteed mask-edit mechanism — pixels outside the mask are bit-identical (property-tested); `mask_path` carried in the contract as best-effort native passthrough |
| `svs` CLI (`src/bin/svs.rs`) | `op` (all six ops), `stack run`/`propose`, `runtime serve --force`, `log`, `project`; `--out` golden runs; `--approve auto\|interactive` |
| Runtime lifecycle | Profile-aware readiness: ops restart their own sd-server on model mismatch; `svs runtime serve --force --model <q2\|q4>` keeps a profile resident and clears stale servers; 20-minute job cap |
| Tests | 62 Rust (`cargo test --features desktop`) + 17 Python, all passing |

## Validated on hardware

- **macOS Apple Silicon:** `regenerate` works end to end at q2 @ 768×448
  (≈1–2 min/image) and q4 @ 1024×1024 (≈9 min/image — too slow, moved to
  Linux/CUDA). Both profiles are provisioned in `models/`.
- **LM Studio:** `config/app.toml` points at `google/gemma-4-e4b` (the id
  served on the dev machine).
- **Not yet exercised anywhere:** the full `modify` mask-edit path
  (segment → confirm → inpaint → composite), `draft` with the LLM proposal
  (only `--text` has been run), `stack propose`.

## Deferred / known gaps

- **SQLite persistence** — the CLI uses a stopgap JSON `ProjectFile`;
  the TOML `ProjectStore` is untouched.
- **GUI work** — the creation canvas, prompt bar, and Studio integration
  are untouched; `AppState` only holds the registry field.
- **Native mask support for Krea** — unvalidated (open question 6 in
  `artifact-canvas.md`); the composite fallback is primary.
- **`paint_strokes`** is vocabulary only, not executable in slice 1.
- **Captioning endpoint** is still a placeholder.
- **Storyfella DSL** not started (built in this repo when it lands).
- **Stale docs:** `AGENTS.md` still says "planner business logic is not
  implemented yet" — the vocabulary now lives in `src/registry/`.

## Recent commits

| Commit | Contents |
| --- | --- |
| `94a685c` | API-first slice: registry, operations, pipelines, `svs` CLI, `mask_path` contract, memorable-name refs, profile-aware runtime, `runtime serve` |
| `20d863b` | Docs: session guide (light/LLM/generation tiers), development/architecture/README updates |

## Where things live

- `docs/artifact-canvas.md` — product design (source of truth)
- `docs/api-slice-1.md` — slice decisions, module map, CLI reference, session guide
- `docs/development.md` — setup, commands, troubleshooting
- `docs/architecture.md` — system boundaries
- `git log --oneline` — the authoritative change history

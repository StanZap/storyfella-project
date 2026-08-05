# API slice 1 — artifact registry, operations, pipelines, `svs` CLI

Status: implemented. The API-first slice of `docs/ROADMAP.md` (§7
operations + pipelines, §12 steps 1–2). No SQLite, no GUI work — the
in-memory model and TOML `ProjectStore` are unchanged.

## Decisions settled (with the user, per §13)

1. **Execution model:** user-typed ops apply immediately and are logged;
   LLM proposals (`svs stack propose`) are gated by approval. Adopted as
   proposed.
2. **Approval granularity:** checkpoints. The mask confirm in the `modify`
   path is a `Checkpoint` step that blocks mid-stack; stack-level
   pre-approval still gates LLM-proposed stacks before execution.
3. **Ref format:** memorable names are the primary `c:` ref — `c:mia`
   (case-insensitive exact match; duplicate names are rejected as
   ambiguous). Full UUIDs, `c:` + UUID, and the 8-hex short id remain
   accepted as fallbacks. Artifacts created without `--name` get a slug
   derived from their description (`"Mia, a lighthouse keeper"` →
   `mia-a-lighthouse-keeper`); variants are auto-named `{base}-{axis}`
   (or `{base}-{slug}`) so every system-created key stays unique.
4. **Native mask support for Krea:** unvalidated (sd.cpp `submit()` sends
   no mask today), so the **composite fallback is primary** — the inpaint
   pipeline generates with the reference image, then blends so every pixel
   *outside* the confirmed mask is bit-identical to the original (feather
   ramps inside the mask only). `mask_path` is carried in the
   Rust ↔ Python `GenerateRequest` contract as best-effort native
   passthrough (`mask_images` in the sd.cpp body) so a validated backend
   can be flipped on later without a contract change.

## Modules

| Module | Contents |
| --- | --- |
| `src/registry/mod.rs` | Artifact model (kinds, variants, scenes, beats, layers, revisions, masks, drafts), one id space, `c:` ref resolution, parent/kind invariants, snapshot/restore undo, stopgap `ProjectFile` |
| `src/registry/ops.rs` | Typed operation set (slice 1: create, variant, regenerate, compose, draft, modify), closed JSON vocabulary (`op`-tagged), compiler (`compile`), executor (`execute`), operation log |
| `src/registry/pipeline.rs` | Linear fail-fast pipeline builder: closed `Step` vocabulary, typed handles (`ImageHandle`, `MaskHandle` → `SelectedMaskHandle`, `PromptHandle`, `PlanHandle`, …), static validation at `build()`, `GenerationBackend` trait, checkpoints, approval policies, `RunOptions` |
| `src/registry/backend.rs` | `CreativeBackend` — the live backend (`CreativeRuntime` + `LmStudioClient`); LLM steps are soft dependencies that degrade to manual input |
| `src/registry/image_ops.rs` | Pure image primitives: composite (masked blend), invert, feather, union — deterministic, property-tested |
| `src/bin/svs.rs` | The `svs` CLI (clap, second binary on `src/lib.rs`) |
| `src/persistence/mod.rs` | SQLite project store (`ProjectDb`): §10 schema, versioned migrations, WAL, snapshot save/load of the registry |
| `src/vision/mod.rs` ↔ `python/models/schemas.py` | `mask_path` added to `GenerateRequest` on both sides |

## Pipeline rules (as implemented)

- Steps are a closed Rust enum; the VLLM combines kinds, never invents
  them. LLM plan/draft steps degrade to manual input on failure — they
  never hard-fail a stack.
- `build()` validates: non-empty stacks, handle ordering, native mask ⇒
  reference image, sizes (multiples of 32, 256..=2048), steps 1..=50,
  ≤ 8 LoRAs with multipliers in -2..=2, model ∈ {krea-2-turbo-q2,
  krea-2-turbo-q4} (Krea 2 is the product model).
- Execution is linear and fail-fast; a rejected checkpoint ends the stack
  cleanly (revision cancelled, op logged `rejected`); a failed step keeps
  intermediates and fails the revision.
- Undo is state restore (`ArtifactRegistry::snapshot`/`restore`), never
  pipeline re-execution.
- The `modify` compiler: `LoadImage → Segment(mask prompt) → Checkpoint
  (confirm mask) → Generate(reference, inpaint prompt) → Composite`; the
  confirmed mask is stored on the new revision (`masks`), keeping its
  grounding prompt and score for follow-up edits.

## CLI

```sh
svs --project p.db project p.db                       # load or create (SQLite registry)
svs --project p.db import legacy.svs-project.json     # one-time migration from the JSON stopgap
svs --project p.db op create character "Mia, a lighthouse keeper" --name mia
svs --project p.db op create scene "The kitchen at dusk" --name kitchen
svs --project p.db op compose c:<scene> "Mia lights the lantern" --background c:<env> --layer c:<char>
svs --project p.db op variant c:<char> "in rain gear" --axis outfit
svs --project p.db op regenerate c:<char> "make it warmer" --seed 42 --steps 4 --size 768x448 --out golden/
svs --project p.db op draft c:<story> "write the opening"          # LLM + approval checkpoint
svs --project p.db op draft c:<story> "write the opening" --text "…"   # manual, no checkpoint
svs --project p.db op modify c:<char> "change her hair" --mask-prompt "her hair" --inpaint-prompt "a bob cut" --approve auto --out golden/
svs --project p.db stack run stack.json --approve auto             # the VLLM contract test bed
svs --project p.db stack propose "Add a rainy variant of the kitchen scene"   # LM Studio → JSON
svs --project p.db runtime serve --force --model krea-2-turbo-q4   # resident generation profile
svs --project p.db log [c:<ref>]
```

`--approve auto|interactive` (default interactive) resolves checkpoints;
`--out <dir>` drops every intermediate (image, mask, composite) into a
folder for human review — the manual golden-run tier. `stack run` persists
already-applied ops even when a later op fails (fail-fast, intermediates
kept).

## Session guide — what runs where

The slice splits into three tiers by what they need running, so a session
can stay light (or run on another machine) without the generation backend:

| Tier | Needs running | Ops |
| --- | --- | --- |
| Model-only | nothing — any machine, even a fresh checkout | `create`, `variant`, `compose`, `draft --text`, `log`, `project`, `stack run` of model-only stacks |
| LLM-assisted | LM Studio (see below) | `draft` (propose + approve), `modify` without explicit prompts (the LLM plans the mask/inpaint split), `stack propose` |
| Generation | provisioned Krea profile + resident sd-server | `regenerate`, `modify` |

LLM steps are soft dependencies: with LM Studio off, `draft`/`modify`
degrade to manual input at a checkpoint instead of failing the stack.

### Keeping a generation profile resident

An op starts its own sd-server when none is running, and that server dies
when the CLI exits (cold start per command). For a longer generation
session, keep a profile resident in one terminal:

```sh
svs --project p.json runtime serve --force --model krea-2-turbo-q4   # Ctrl-C to stop
```

- `--force` kills stale sd-server processes from interrupted sessions —
  they hold port 7861 and cannot be restarted by the runtime.
- Ops auto-restart **their own** server when the requested model differs
  from the resident one: `--model krea-2-turbo-q4` while q2 is loaded just
  works.
- A mismatched server that another session owns errors with a
  `ProfileMismatch` message naming the fix (`svs runtime serve --force` or
  `pkill -f "sd-server --diffusion"`).

### LM Studio

`lm_studio.model` in `config/app.toml` must match a model id LM Studio
serves (`curl http://localhost:1234/v1/models` lists them). The checked-in
config points at the gemma model used on the dev machine.

### Platform notes (measured)

- **macOS Apple Silicon:** q2 @ 768×448 @ 4 steps ≈ 1–2 min per image —
  the light loop (everything in the table, including `modify`).
  q4 @ 1024×1024 @ 8 steps is ≈ 9 min per image — not worth it; keep heavy
  work on CUDA.
- **Linux + NVIDIA CUDA:** q2 and q4 are both fast. The 20-minute job cap
  in `CreativeRuntime::wait_for_job` only matters on slow hardware.
- `model_setup` runs once per machine/profile
  (`--profile q2|q4 --accept-krea-license`); artifacts land in
  `paths.model_dir` (`models/` in this checkout).

## Divergences from the design doc (deferred, by directive)

- **Storyboard still on the legacy model.** The GUI persists via SQLite
  (schema v2 `project_json` on the `projects` row); the old beat/timeline
  model survives until the canvas (roadmap item 4) replaces it. `svs
  project <path>` stays one-shot, no session state.
- **No canvas work.** `src/ui/` gained real project open/save/import and
  autosave; the artifact registry is persisted but not yet surfaced in
  Studio (item 4).
- `paint_strokes` is vocabulary (a `Step` + builder method) but not
  executable in slice 1; `describe`/`critique` LLM steps degrade to manual
  input.

## First CI surface

- Builder validation tests (`src/registry/pipeline.rs`): empty stacks,
  bounds, ordering, mask-without-reference.
- Composite property tests (`src/registry/image_ops.rs`): pixels outside
  the mask are bit-identical with and without feathering.
- Execution tests with a fake backend: request bodies (prompt, reference,
  mask, LoRA, size, seed), revision state transitions, checkpoint
  accept/reject/degrade.
- Contract tests: `mask_path` round-trips on both sides of the wire
  (Rust `vision` tests, Python `test_contracts.py`,
  `test_native_generation.py`).

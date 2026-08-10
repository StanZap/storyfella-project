# API slice 1 — artifact registry, operations, pipelines, `svs` CLI

Status: implemented. The API-first slice of `docs/ROADMAP.md` (§7
operations + pipelines, §12 steps 1–3). SQLite persistence and the GUI
save/open flow landed with the foundation (step 3); the canvas (step 4)
surfaces the artifact registry in the GUI.

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
4. **Masks are no longer how `modify` works** (revised 2026-08-10). The
   generation model is an instruction editor that preserves what the
   instruction did not mention, so `modify` sends a reference image and a
   grounded instruction and the mask machinery is opt-in. `mask_path` is
   still carried in the Rust ↔ Python `GenerateRequest` contract, and the
   sd.cpp body field is **`mask_image`** — singular. It was `mask_images`
   (an array) for a long time; sd.cpp drops unrecognized keys silently, so
   the mask never arrived and the composite pasted a region of a fully
   re-rendered frame over the original. Anything the server does not name
   in its JSON parser is discarded without a warning — check the parser
   before concluding a generation input has no effect.
5. **Visual `create`/`variant` generate the first image.** Character,
   environment, and object artifacts get their first image as part of the
   operation (prompt = description, at the artifact's size); the variant's
   first image is an edit of the base — its active image as the reference
   (`generate(prompt: variant description, reference: base image)`),
   matching what a manual `/regenerate c:<base> "<variant desc>"` would
   have sent. A failed generation keeps the artifact with a failed
   revision; `/regenerate` is the retry. Text-only kinds (story, scene,
   beat) and `compose` stay model-only.

## Modules

| Module | Contents |
| --- | --- |
| `src/registry/mod.rs` | Artifact model (kinds, variants, scenes, beats, layers, revisions, masks, drafts), one id space, `c:` ref resolution, parent/kind invariants, snapshot/restore undo |
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
- The `modify` compiler emits one of two shapes. Default (an instruction
  edit): `LoadImage → Ground(instruction) → Generate(reference, grounded
  instruction)` — no mask, no composite, no checkpoint, and the inpaint
  adapter is not injected. With both `--mask-prompt` and `--inpaint-prompt`
  (opt-in): `LoadImage → Segment(mask prompt) → Checkpoint(confirm mask) →
  Generate(reference, inpaint prompt) → Composite`; the confirmed mask is
  stored on the new revision (`masks`) with its grounding prompt and score.
- `Ground` is a soft LLM step with a deterministic fallback, so unlike
  `Plan` it never blocks on a missing planner — it substitutes the template
  and continues.

## CLI

```sh
svs --project p.db project p.db                       # load or create (SQLite registry)
svs --project p.db op create character "Mia, a lighthouse keeper" --name mia
svs --project p.db op create scene "The kitchen at dusk" --name kitchen
svs --project p.db op create object "an antique brass lantern" --name lantern --size 768x768
svs --project p.db op compose c:<scene> "Mia lights the lantern" --background c:<env> --layer c:<char>
svs --project p.db op variant c:<char> "in rain gear" --axis outfit
svs --project p.db op regenerate c:<char> "make it warmer" --seed 42 --steps 4 --size 768x448 --out golden/
svs --project p.db op draft c:<story> "write the opening"          # LLM + approval checkpoint
svs --project p.db op draft c:<story> "write the opening" --text "…"   # manual, no checkpoint
svs --project p.db op modify c:<char> "give her a short bob haircut"   # instruction edit
svs --project p.db op describe c:<char> "Mia, a lighthouse keeper, short bob"  # make an edit canonical
svs --project p.db op modify c:<char> "change her hair" --mask-prompt "her hair" --inpaint-prompt "a bob cut" --approve auto --out golden/  # opt-in mask path
svs --project p.db stack run stack.json --approve auto             # the VLLM contract test bed
svs --project p.db stack propose "Add a rainy variant of the kitchen scene"   # LM Studio → JSON
svs --project p.db runtime serve --force --model krea-2-turbo-q4   # resident generation profile
svs --project p.db log [c:<ref>]
```

`create --size WxH` sets the artifact's default generation size (kind
defaults otherwise: character 512×768, object 768×768, environment/scene/beat
1024×576); `regenerate`/`modify` honor it, and `--size`/`--steps` on those
commands override it per run. Variants inherit the base artifact's size.
`create`/`variant` of visual kinds generate the first image at that size as
part of the op (so they need the generation backend — see the table below);
variants use the base's active image as the reference.

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
| Model-only | nothing — any machine, even a fresh checkout | `create` of text-only kinds (story/scene/beat), `compose`, `draft --text`, `log`, `project`, `stack run` of model-only stacks |
| LLM-assisted | LM Studio (see below) | `draft` (propose + approve), `modify` without explicit prompts (the LLM plans the mask/inpaint split), `stack propose`, free-form canvas messages (proposed stacks shown for approval) |
| Generation | provisioned Krea profile + resident sd-server | `create`/`variant` of visual kinds (first image), `regenerate`, `modify` |

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

- **The legacy storyboard model is gone.** The canvas (roadmap item 4)
  replaced `Project`/`StoryboardFrame`/`Timeline`; the GUI is the artifact
  registry (`AppState` in `src/state/`). Schema v3 drops `project_json`
  from the `projects` row — the sanctioned one-time loss, no legacy
  formats are parsed.
- **Canvas slice 1** (`src/ui/canvas.rs`): sidebar grouped by kind, detail
  pane (image, description, variants, revisions, children, operation log),
  slash-command composer (`/create`, `/variant`, `/regenerate`,
  `/modify`), undo/redo via registry snapshots. The mask checkpoint of
  `modify` is interactive: the op runs on a worker thread (its own tokio
  runtime) and the segmentation candidates are shown overlaid on the image
  in a modal; the choice is sent back over a channel. A `modify` whose LLM
  edit plan degrades (no LM Studio) asks for the mask/inpaint split in a
  second modal instead of rejecting. Stored masks appear
  under the active revision (served via the `mask` asset handler; the
  runtime points `SVS_MASK_DIR` at `<asset_dir>/masks`). Free-form composer
  messages (no leading `/`) are proposed as operation stacks by LM Studio
  (`CreativeBackend::propose_stack`, shared with `svs stack propose`) and
  shown in an approval modal with per-op toggles; approved stacks run in
  order through the same worker-thread machinery, so intra-op checkpoints
  stay interactive. The proposal prompt names the **currently viewed
  artifact** (`propose_stack` takes it) — requests without an explicit
  `c:` ref target the viewed artifact, so a change typed while viewing a
  variant edits the variant, never its base; the composer shows the
  binding as a `→ c:<ref>` chip. The operation log carries a **re-run**
  icon per applied row (`regenerate`/`modify`/`draft` only — re-executing
  the exact op with a fresh seed through the same worker path; creating
  ops would duplicate state and are excluded with a tooltip).
  Deferred per §6:
  artboard pan/zoom, `c:` autocomplete, syntax highlighting, and the
  compose/draft op surface (beats are item 5, Studio integration).
- `paint_strokes` is vocabulary (a `Step` + builder method) but not
  executable in slice 1; `describe`/`critique` LLM steps degrade to manual
  input.

## After slice 1: generation quality controls + LoRA registry

- **Configurable defaults** (`config/app.toml`, editable in Settings →
  Image generation, persisted line-preserving): `[generation] steps`,
  `[generation.sizes]` per kind (`WxH` strings), `[lora]` bindings
  (`inpaint`, `character`, `environment`, `object` file names within
  `generation.lora_dir` plus a shared `multiplier`; discovery is recursive
  (subfolders and symlinked model trees) with a refresh icon in Settings). The
  canvas ops read a
  live `Signal<GenerationDefaults>` (Workspace context); the CLI builds
  them from the loaded config. Kind resolution order everywhere: the
  artifact's explicit `--size` → the config override → the built-in kind
  default.
- **Sizes, steps, and adapters are user-requested intent.** The visual
  ops (`create`/`variant`/`regenerate`/`modify`) carry optional `size`,
  `steps`, and `loras` fields in the LLM JSON vocabulary, plus the
  dedicated `resize` op (sets the artifact's default size **and**
  regenerates at it). The planner prompt tells the model to include them
  ONLY when the user explicitly asks for a size, more steps, or a named
  adapter — never invented. All validated at compile (multiples of 32,
  256..=2048; steps 1..=50; multipliers -2..=2).
- **Tool-calling planner.** `CreativeBackend::propose_stack` runs a
  multi-round tool session (`list_artifacts`, `list_loras` — the model
  discovers the registry and the adapters itself) with the strict JSON
  schema riding every round so the final stack stays in the closed
  vocabulary (validated live against LM Studio). Servers that reject
  `tools` fall back to the static listing + strict schema.
- **`variant --name`** — explicit naming for variants (auto-naming
  `{base}-{axis}` remains the fallback).
- **LoRA registry** (§12 item 6): `ArtifactRegistry.loras` round-trips
  the `lora_registry` table; `/attach-lora <ref> <file|entry>`
  `[--multiplier]` binds (file must exist in `lora_dir`), `/lora-scale`
  and `/lora-remove` adjust/unbind by entry name — all three in the slash
  parser, the CLI, and the LLM closed vocabulary (the planner prompt lists
  the discovered adapters so the LLM can propose bindings). Auto-injection
  into every generation pipeline at compile: artifact-bound entries
  (walking the variant-of base chain) first, then the config kind binding,
  then the inpaint adapter on `modify`; first-wins by resolved path,
  missing-on-disk adapters are skipped with a warning. Per-run `--lora
  <file>@<mult>` CLI flags append after the auto-injected set (deduped).
- `svs` CLI: `op attach-lora`/`lora-scale`/`lora-remove`, `--lora` on
  `create`/`variant`/`regenerate`/`modify`, `stack run`/`propose` carry
  the new ops.

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

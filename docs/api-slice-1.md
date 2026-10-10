# API slice 1 — artifact registry, operations, pipelines, `svs` CLI

Status: implemented. The API-first slice of `docs/ROADMAP.md` (§7
operations + pipelines, §12 steps 1–3). SQLite persistence and the GUI
save/open flow landed with the foundation (step 3); the canvas (step 4)
surfaces the artifact registry in the GUI.

## Decisions settled (with the user, per §13)

1. **Execution model:** user-typed ops apply immediately and are logged;
   LLM proposals (`svs stack propose`) are gated by approval. Adopted as
   proposed.
2. **Approval granularity:** checkpoints. A `Checkpoint` step blocks
   mid-stack (draft-text approval); stack-level pre-approval still gates
   LLM-proposed stacks before execution. `modify` is checkpoint-free — an
   instruction edit segments nothing.
3. **Ref format:** memorable names are the primary `c:` ref — `c:mia`
   (case-insensitive exact match; duplicate names are rejected as
   ambiguous). Full UUIDs, `c:` + UUID, and the 8-hex short id remain
   accepted as fallbacks. Artifacts created without `--name` get a slug
   derived from their description (`"Mia, a lighthouse keeper"` →
   `mia-a-lighthouse-keeper`); variants are auto-named `{base}-{axis}`
   (or `{base}-{slug}`) so every system-created key stays unique.
4. **`modify` is an instruction edit** (revised 2026-08-10; made
   unconditional by the Qwen-Image-2.1 migration). The generation model takes
   a reference image plus a plain-language instruction and preserves what the
   instruction did not mention, so `modify` sends the artifact's active
   revision as the reference and a grounded instruction as the prompt — no
   mask, no segmentation, no composite. `GenerateRequest` has no mask field,
   and the pipeline has no mask/Segment/Composite steps. Anything the server
   does not name in its JSON parser is discarded without a warning — check the
   parser before concluding a generation input has no effect.
5. **Visual `create`/`variant` generate the first image.** Character,
   environment, and object artifacts get their first image as part of the
   operation (prompt = description, at the artifact's size); the variant's
   first image is an edit of the base — its active image as the reference,
   and the variant description **grounded against the base** before it is
   sent. The grounding is not optional: a variant description is a
   delta ("in heavy yellow oilskins") and sending it bare drifts off the
   base entirely — measured, a variant of a woman in her forties came back
   as a different person of a different sex. A failed generation keeps the
   artifact with a failed revision; `/regenerate` is the retry. Text-only
   kinds (story, scene, beat) and `compose` stay model-only.
6. **Variants do not nest.** A variant must derive from a base artifact, so
   the tree stays one level deep and every look is reproducible from the
   base alone; `RegistryError` rejects a variant of a variant. Combinations
   are described in full on a single variant ("in oilskins, grim") rather
   than layered.
7. **`describe` is the only way to change a description**, and the
   description is what every edit grounds against — so it is how an edit
   becomes canonical rather than one revision among many. See
   `docs/architecture.md` for why the anchor is the artifact and not the
   current revision.

## Modules

| Module | Contents |
| --- | --- |
| `crates/studio-core/src/registry/mod.rs` | Artifact model (kinds, variants, scenes, beats, layers, revisions, drafts), one id space, `c:` ref resolution, parent/kind invariants, snapshot/restore undo |
| `crates/studio-core/src/registry/ops.rs` | Typed operation set (slice 1: create, variant, regenerate, compose, draft, modify), closed JSON vocabulary (`op`-tagged), compiler (`compile`), executor (`execute`), operation log |
| `crates/studio-core/src/registry/pipeline.rs` | Linear fail-fast pipeline builder: closed `Step` vocabulary (`LoadImage`, `Checkpoint`, `Generate`, `Llm`), typed handles (`ImageHandle`, `PromptHandle`, `FeedbackHandle`), static validation at `build()`, `GenerationBackend` trait, `GenerateResponse`, checkpoints, approval policies, `RunOptions` |
| `crates/studio-core/src/registry/backend.rs` | `CreativeBackend` — the live backend (`CreativeRuntime` + `LlmClient`); LLM steps are soft dependencies that degrade to manual input |
| `clipforge-inference` (`../clipforge/crates/inference`) | The shared native-inference crate: `sd::Client` — the typed `sdcpp` client for the native `sd-server` — plus `sd::Runtime` (the resident process), `sd::qwen_image_2_1()` (the served profile), and `ModelStore` (provisioning). storyfella's `crates/studio-core/src/runtime/` adapts it as `CreativeRuntime` (see `architecture.md`) |
| `crates/studio-core/src/persistence/mod.rs` | SQLite project store (`ProjectDb`): §10 schema, versioned migrations, WAL, snapshot save/load of the registry |
| `apps/cli/src/bin/svs.rs` | The `svs` CLI (clap, a binary of the `svs-cli` package) |

## Pipeline rules (as implemented)

- Steps are a closed Rust enum; the VLLM combines kinds, never invents
  them. LLM plan/draft steps degrade to manual input on failure — they
  never hard-fail a stack.
- `build()` validates: non-empty stacks, handle ordering, sizes (multiples
  of 32, 256..=2048), steps 1..=50, ≤ 8 LoRAs with multipliers in -2..=2,
  and model `qwen-image-2.1` (the only generation model).
- Execution is linear and fail-fast; a rejected checkpoint ends the stack
  cleanly (revision cancelled, op logged `rejected`); a failed step keeps
  intermediates and fails the revision.
- Undo is state restore (`ArtifactRegistry::snapshot`/`restore`), never
  pipeline re-execution.
- The `modify` compiler emits one shape: `LoadImage → Generate(reference,
  grounded instruction)` — no mask, no composite, no checkpoint.
- `Ground` is a soft LLM step with a deterministic fallback, so unlike
  `Plan` it never blocks on a missing planner — it substitutes the template
  and continues. `variant` and `regenerate`-with-a-prompt emit it too; a
  `regenerate` with no prompt does not, because it replays a prompt that was
  already grounded when the revision was made.

## CLI

```sh
svs --project p.db project p.db                       # load or create (SQLite registry)
svs --project p.db op create character "Mia, a lighthouse keeper" --name mia
svs --project p.db op create scene "The kitchen at dusk" --name kitchen
svs --project p.db op create object "an antique brass lantern" --name lantern --size 768x768
svs --project p.db op compose c:<scene> "Mia lights the lantern" --background c:<env> --layer c:<char>
svs --project p.db op variant c:<char> "in rain gear" --axis outfit
svs --project p.db op regenerate c:<char> "make it warmer" --seed 42 --steps 25 --size 1024x1536 --out golden/
svs --project p.db op draft c:<story> "write the opening"          # LLM + approval checkpoint
svs --project p.db op draft c:<story> "write the opening" --text "…"   # manual, no checkpoint
svs --project p.db op modify c:<char> "give her a short bob haircut"   # instruction edit
svs --project p.db op describe c:<char> "Mia, a lighthouse keeper, short bob"  # make an edit canonical
svs --project p.db op revert c:<char>                                # re-activate the previous image
svs --project p.db stack run stack.json --approve auto             # the VLLM contract test bed
svs --project p.db stack propose "Add a rainy variant of the kitchen scene"   # LLM → JSON
svs --project p.db runtime serve --force                           # resident generation server
svs eval -i image.png -p "answer YES or NO: …"                     # LLM vision judge
svs --project p.db log [c:<ref>]
```

`create --size WxH` sets the artifact's default generation size (kind
defaults otherwise: character 512×768, object 768×768, environment/scene/beat
1024×576; the platform `studio.toml` is created with character at 1024×1536);
`regenerate`/`modify` honor it, and `--size`/`--steps` on those
commands override it per run. Variants inherit the base artifact's size.
`create`/`variant` of visual kinds generate the first image at that size as
part of the op (so they need the generation backend — see the table below);
variants use the base's active image as the reference.

`describe` rewrites an artifact's canonical description; `revert` re-activates
its previous completed image (the newest revision made before the current one).
Neither renders anything: both are registry mutations, so they need no
backend. `revert` never deletes — the revision reverted from stays in the
history and can be re-activated from the canvas.

`--approve auto|interactive` (default interactive) resolves checkpoints;
`--out <dir>` drops every intermediate image into a folder for human review —
the manual golden-run tier. `stack run` persists already-applied ops even
when a later op fails (fail-fast, intermediates kept).

## Session guide — what runs where

The slice splits into three tiers by what they need running, so a session
can stay light (or run on another machine) without the generation backend:

| Tier | Needs running | Ops |
| --- | --- | --- |
| Model-only | nothing — any machine, even a fresh checkout | `create` of text-only kinds (story/scene/beat), `compose`, `draft --text`, `log`, `project`, `stack run` of model-only stacks |
| LLM-assisted | LLM endpoint (see below) | `draft` (propose + approve), `modify` (the LLM grounds the instruction), `stack propose`, free-form canvas messages (proposed stacks shown for approval) |
| Generation | provisioned Qwen-Image-2.1 profile + resident `sd-server` | `create`/`variant` of visual kinds (first image), `regenerate`, `modify` |

LLM steps are soft dependencies: with the LLM endpoint off, `draft`/`modify`
degrade to deterministic/manual behaviour instead of failing the stack.

### Keeping the generation server resident

An op ensures the machine's shared `sd-server` through
`clipforge_inference::control`: it starts one detached when none is running, and
reuses a resident one otherwise. The server is owned by the shared owner, not by
the command, so it stays resident after the CLI exits and is reused by the next
op or by Studio/Atelier. Stop it explicitly:

```sh
svs runtime stop --force
```

For a foreground session whose server is owned by the command, keep the profile
resident in one terminal:

```sh
svs --project p.db runtime serve --force   # Ctrl-C to stop
```

- `--force` stops stale servers from interrupted sessions — they hold port
  7861 and cannot be restarted by the runtime. It matches the served
  executable and listen port (`servers::find_servers`), not a `pkill` pattern,
  so another model's server is left alone.
- Only the `qwen-image-2.1` profile is served.
- A mismatched server the shared owner started is replaced by **Start** in
  Settings or `svs runtime start`; a foreign server needs
  `svs runtime stop --force` (or `svs runtime serve --force`).

### LLM endpoints

Text (planning/grounding) and vision (`svs eval`) resolve from the shared
`clipforge.toml` as separate capabilities (`text`/`vision`), so the planner can be
a remote, text-only model — DeepSeek, say — while vision stays on a local
multimodal endpoint. Each provider's `model` must match a model id its endpoint
serves (`curl http://localhost:1234/v1/models` lists them); the default
`clipforge.toml` points `text` at the larger planner model and `vision` at the
vision model used on the dev machine. A machine-local secret or override goes in
that shared file (`~/.config/clipforge/clipforge.toml`, `$SVS_AI_CONFIG`
overrides).

### Platform notes (measured)

- **Linux + NVIDIA CUDA:** the validated target. Measured on an RTX 3090
  (24 GiB, int8 weights, 25 steps, CFG 6.0, euler): text-to-image 1024²
  ≈52 s sampling (≈62–68 s wall), reference edit/cutout 1024×1536 ≈140 s,
  peak ≈23.4 GiB. Qwen-Image-2.1 has no few-step turbo path.
- The 20-minute job cap in `CreativeRuntime::wait_for_job` only matters on
  slow hardware.
- `model_setup` runs once per machine (artifacts land in the image provider's
  OS-local models folder by default, or the `model_dir` override from `clipforge.toml`).

## Divergences from the design doc (deferred, by directive)

- **The legacy storyboard model is gone.** The canvas (roadmap item 4)
  replaced `Project`/`StoryboardFrame`/`Timeline`; the GUI is the artifact
  registry (`AppState` in `apps/studio/src/state/`). Schema v3 dropped
  `project_json` — the sanctioned one-time loss, no legacy
  formats are parsed.
- **Canvas slice 1** (`apps/studio/src/ui/canvas.rs`): gallery grid and detail
  pane (image, description, variants, revisions, children, operation log),
  slash-command composer (`/create`, `/variant`, `/regenerate`,
  `/modify`), undo/redo via registry snapshots. `modify` runs unattended (an
  instruction edit has no checkpoint). Free-form composer
  messages (no leading `/`) are proposed as operation stacks by the LLM
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
  compose/draft op surface (beats are Studio integration).
- `describe`/`critique` LLM steps degrade to manual input.

## After slice 1: generation quality controls + LoRA registry

- **Configurable defaults** (the platform `studio.toml`, editable in Settings →
  Models, persisted line-preserving): `[generation] steps`,
  `[generation] cfg_scale`/`sample_method`, `[generation.sizes]` per kind
  (`WxH` strings), and `[lora]` bindings (`character`, `environment`,
  `object` file names within the provider's `lora_dir` plus a shared
  `multiplier`; discovery is recursive (subfolders and symlinked model
  trees) with a refresh icon in Settings). The
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
  vocabulary (validated live against the LLM endpoint). Servers that reject
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
  (walking the variant-of base chain) first, then the config kind binding;
  first-wins by resolved path, missing-on-disk adapters are skipped with a
  warning. Per-run `--lora <file>@<mult>` CLI flags append after the
  auto-injected set (deduped).
- `svs` CLI: `op attach-lora`/`lora-scale`/`lora-remove`, `--lora` on
  `create`/`variant`/`regenerate`/`modify`, `stack run`/`propose` carry
  the new ops.

## First CI surface

- Builder validation tests (`crates/studio-core/src/registry/pipeline.rs`): empty stacks,
  bounds, ordering.
- Execution tests with a fake backend: request bodies (prompt, reference,
  LoRA, size, seed), revision state transitions, checkpoint
  accept/reject/degrade.
- Serialization tests (the shared `clipforge-inference` crate,
  `../clipforge/crates/inference/src/sd/`): the `sdcpp` wire body
  (sampling controls, reference, LoRA, VAE tiling) and job decoding.

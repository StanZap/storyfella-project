# Artifact canvas — product design notes

> **Status: design discussion notes.** Captures the ongoing product-design
> conversation around the artifact registry, the creation canvas, the typed
> operation set, and SQLite persistence. Nothing in this document is
> implemented yet; confirmed decisions and open questions are marked as such.

## 1. Vision recap

The product lets a user write a visual story and create its slides, iterating
on both until the result is right. The current app already covers prompt →
storyboard beat → Krea generation → revision history. The target shape adds:

- a shared **artifact registry** (characters, environments, objects, scenes,
  beats) with persistent visual variants;
- a **creation canvas** workspace where artifacts are created and edited
  through language (`/create …`, `/modify …`) plus direct manipulation;
- LLM-assisted story writing (plot → bible → scenes → beat narration) with
  propose/approve semantics;
- full per-beat generation history, mask-based regional edits, and a LoRA
  registry for character/environment consistency.
- the **Storyfella DSL** as the authored story surface: `.sf` files that
  reference registry artifacts, with the canvas as the visual-asset editor
  (see §9).

**Model focus:** Krea 2 (served by the native `stable-diffusion.cpp` process)
is the primary generation model. The stack stays as documented in
`architecture.md`; Krea 2's prompt-following and quality are the baseline and
LoRAs build on top of it.

## 2. Vocabulary

| Term | Meaning |
| --- | --- |
| Beat | One slide/panel of the visual story. A beat is a composition of layers referencing other artifacts. Today's `StoryboardFrame`. |
| Scene | A group of beats set in one place with a cast. New level above beats. |
| Artifact | Any first-class project entity: story, scene, beat, character, object, environment. One unified id space (`c:<ref>`). |
| Character | Reusable person artifact with named visual variants. |
| Object | Reusable prop/object artifact (same pattern as character). |
| Environment | Reusable place artifact (house, city, forest, …). |
| Backdrop | A *role inside a composition*, not an artifact kind: the layer slot a beat fills with an environment variant or a fresh generation. |
| Variant | A named visual variation of an artifact (character: outfit/age/body/hair/expression; environment: time-of-day/weather/season/mood). |
| Composition | A beat's description of how layers (backdrop, baked elements, dynamic characters) combine into one image. |
| Operation | A typed command from the operation set (Section 7), invoked via slash syntax or proposed by the LLM. |

## 3. Artifact model

```text
Project
├── Story(s)                     — one or more stories per project
│   ├── story documents          — premise, plot (prose, separate from artifact text)
│   └── Scene(s)                 — location + cast (character × variant) + beats
│       └── Beat(s)              — slides; each a composition of layers
│           └── layers           — references to character/environment/object artifacts
│               └── revisions    — persisted generation history per visual artifact
├── Character(s) (+ variants)    — bible level, reusable across stories
├── Environment(s) (+ variants)  — bible level; rooms are children, not variants
└── Object(s) (+ variants)       — bible level, reusable props
```

Rules agreed so far:

- **Unified registry, one id space.** Every artifact (including beats) is the
  same kind of thing to the system; `c:<ref>` references work uniformly.
- **Variants are artifacts** with `variant_of` pointing at their base. Axes
  (outfit, age, body, hair, expression, time-of-day, weather, season, mood)
  are tags for organization, not structure.
- **Rooms are children, not variants.** A house *contains* a kitchen; a
  kitchen's *variants* are states of that same space (rainy evening, morning).
- **A beat is a composition** — a list of layers referencing other artifacts
  with roles; it is not limited to a single environment + character.
- **Scene = location + cast + beats.** The timeline groups clips by scene.

## 4. Composition modes

A beat renders in one of three modes:

| Mode | Description | Change cost |
| --- | --- | --- |
| Baked | Everything in one Krea generation (today's flow). Max quality. | Regenerate or inpaint the region. |
| Layered | Environment base + character/object cutouts composited by the app. | Instant: swap variant/expression and recomposite. |
| Hybrid | Baked backdrop (environment + background characters baked in) with dynamic character layers composited on top. | Only dynamic layers regenerate. |

Consequences:

- The beat needs a **composition spec** separate from the rendered image:
  `background` (environment variant ref or "generate fresh"), `layers:
  [{artifact_ref, variant_ref, role, anchor}]`, and `mode`.
- **Re-sync is mode-dependent:** layered → recomposite; baked → regenerate or
  inpaint; hybrid → regenerate dynamic layers only.
- **Cutouts come from segmentation + matting**, not diffusion (no alpha
  channel). SAM 2.1 segmentation exists (`POST /segment`); matting quality
  (hair edges) is imperfect, so layered mode is the iteration mode while
  baked/hybrid dominate final quality. *(Matte quality acceptance: open.)*
- **Positioning is direct manipulation, not an operation.** Ops carry intent
  (what), gestures carry geometry (where, size, z-order).

## 5. Generation & consistency

- **LoRA registry.** Existing plumbing: `--lora-model-dir` on sd-server, up to
  eight `LoraSelection { path, multiplier }` per generation request in the
  Rust contract (`src/vision/mod.rs`) and the Python contract
  (`python/models/schemas.py`). Missing: a project-level registry (name, file,
  base model, tags), binding a LoRA to an artifact, and auto-injection into
  every generation referencing that artifact. *(Confirmed as desired; the
  registry is new work.)*
- **Character/environment consistency** is achieved by injecting canonical
  reference images and bound LoRAs into every generation that references the
  artifact. Trained character LoRAs are expected to be the main lever.
- **Masks.** Segmentation exists and produces mask PNGs. Masks attach to a
  *revision* (they are a view over one specific image). Missing: `mask_path`
  in `GenerateRequest` for mask-guided inpaint/outpaint edits (same W×H
  output). *(Whether the native backend takes the mask directly or the Python
  adapter composites the region must be validated: open.)*

### Regional edits — the canonical `modify` flow

The flagship use case is a visual `modify` on a character ("change her hair
style"). One command expands into a sequence of primitives:

1. `modify <char> <description>` — the change description is parsed (with LLM
   help) into a **mask prompt** ("hair") and an **inpaint prompt** ("bob
   cut").
2. `mask <char> <mask-prompt>` — `POST /segment` (SAM 2.1 + grounding)
   proposes candidate hair masks over the character's active image.
3. **User confirms a mask** — SAM returns several candidates; generating
   against the wrong mask wastes a full generation, so the mask is shown for
   confirmation before diffusion runs. This is the one interactive step.
4. `inpaint` — image + chosen mask + inpaint prompt go to the generator; the
   output keeps the original dimensions (same W×H).
5. The result is stored as a new revision; the mask is stored on the revision
   (`masks` table) so follow-up edits ("make it shorter") reuse or refine the
   same mask.

Two mechanisms can guarantee "only the mask region changes":

- **Native mask input**: pass `mask_path` to the backend if the model accepts
  masked inpainting directly (validation gate — open question 6).
- **Composite fallback (guaranteed)**: generate with the reference image and
  prompt, then blend so everything *outside* the mask keeps the original
  pixels, with feathering to hide seams.

The same flow applies to environments and objects; character hair is the
canonical example and the first to be built.

## 6. Creation canvas

The central workspace for creating and modifying artifacts:

- A large (effectively infinite) **artboard** with floating artifact cards,
  pan/zoom viewport.
- A **prompting area** below the artboard:
  - `/create <kind> <description>`, `/modify <ref> <description>`, … — the UI
    parses slash syntax into typed operations, with autocomplete and argument
    hints.
  - Free-form messages go to the LLM, which responds with a *proposed
    sequence of the same typed operations*.
  - `c:<ref>` mentions of existing artifacts, autocompleted from the registry.
  - Syntax highlighting for `/operations` and `c:refs` — implemented as a
    transparent textarea over a tokenized render (code-editor style); a
    dedicated chunk of UI work, deferred.
- **Artifact card anatomy:** written features/operations on the right, visual
  grid of variants/revisions on the left. Clicking an image expands it to fill
  the visual space with left/right nav actions through the variants.
- The canvas is the place where artifacts are born and edited; the operation
  log per artifact is its history, provenance for re-sync, and later the
  timeline for animations/transformations.

*(Relationship to the current Studio — creation canvas as the main workspace
vs. side-by-side with Studio for sequencing: open; lean is side-by-side.)*

## 7. Operation set (draft)

Typed operations defined in Rust (this is the planner business logic that is
currently unimplemented). Slash syntax is parsed by the UI; the LLM maps free
text onto the same set.

Every operation has a **kind**:

| Kind | Meaning |
| --- | --- |
| Primitive | Atomic, single effect; the atoms of the system |
| Compound | A named, saved sequence of primitives (data, not code) |
| Pure | Read-only (ask, summarize) — no mutation, no log entry |

**Slices** define when each operation ships. Slice 1 is the five-operation
core that drives prototyping; slices 2 and 3 extend it.

| Slice | Operation | Kind | Syntax | Purpose |
| --- | --- | --- | --- | --- |
| 1 | create | primitive | `/create <kind> <description>` | New artifact (story, scene, beat, character, object, environment) |
| 1 | variant | primitive | `/variant <ref> <description> [axis]` | New visual variant of an artifact |
| 1 | regenerate | primitive | `/regenerate <ref> [prompt]` | New revision of the active image (fresh seed, or edited prompt with current image as reference) |
| 1 | compose | primitive | `/compose <scene> <description> [layer refs…]` | New beat in a scene |
| 1 | draft | primitive | `/draft <ref> <request>` | LLM proposes story text; user approves |
| 2 | modify | primitive | `/modify <ref> <description>` | Change the artifact's defining text |
| 2 | write | primitive | `/write <ref> <text>` | Direct text (narration, premise, …) |
| 2 | rename | primitive | `/rename <ref> <name>` | Rename |
| 2 | delete | primitive | `/delete <ref>` | Remove (cascade rules TBD) |
| 2 | mask | primitive | `/mask <ref> <prompt-or-box>` | Segment the revision's image; store candidate masks |
| 2 | inpaint | primitive | `/inpaint <ref> <description> [mask]` | Mask-guided regional edit, same W×H output |
| 2 | outpaint | primitive | `/outpaint <ref> <description>` | Extend the image beyond its borders |
| 2 | layer | primitive | `/layer <beat> <ref> [role] [variant]` | Add/swap a layer (role: backdrop / baked / dynamic); geometry added by drag afterward |
| 2 | promote | primitive | `/promote <ref>` | Mark a revision as the artifact's canonical reference image |
| 2 | attach-lora | primitive | `/attach-lora <ref> <lora> [multiplier]` | Bind a registry LoRA; auto-injected into the artifact's generations |
| 2 | import | primitive | `/import <file>` | Bring an external image in as a revision |
| 2 | ask | pure | `/ask <question>` | LLM Q&A about the story; no mutation |
| 2 | variations | compound | `/variations <ref> <n>` | Generate an N-revision grid for comparison |
| 2 | re-sync | compound | `/re-sync <ref>` | Regenerate/recomposite every beat referencing the artifact |
| 3 | cutout | primitive | `/cutout <ref>` | Extract subject to a transparent PNG (matte) for layered mode |
| 3 | lora-scale | primitive | `/lora-scale <ref> <multiplier>` | Adjust a bound LoRA's weight |
| 3 | lora-remove | primitive | `/lora-remove <ref>` | Unbind a LoRA |
| 3 | style | primitive | `/style <ref> <description>` | Project/story-level style binding (style LoRA + prompt prefix) |
| 3 | duplicate | primitive | `/duplicate <ref>` | Branch an artifact (explore without losing the original) |
| 3 | tag | primitive | `/tag <ref> <tag>` | Organizational tags |
| 3 | note | primitive | `/note <ref> <text>` | Author annotation; never affects generation |
| 3 | move | primitive | `/move <beat> <scene>` | Relocate a beat between scenes |
| 3 | crop | primitive | `/crop <ref>` | Geometry-only crop |
| 3 | upscale | primitive | `/upscale <ref>` | Raise resolution |
| 3 | pose | primitive | `/pose <ref> <description>` | Re-pose variant without changing identity |
| 3 | expression | primitive | `/expression <ref> <description>` | Expression variant (dynamic layers) |
| 3 | transition | primitive | `/transition <beat> <effect>` | Fade/cut/dissolve out of a beat; future animation input |
| 3 | camera | primitive | `/camera <beat> <description>` | Camera note (zoom/angle/pan); metadata now, animation later |
| 3 | export | primitive | `/export <beat\|scene\|story>` | Render composed output (image now; presentation/video later) |
| 3 | summarize | pure | `/summarize <ref>` | Condense revision/operation history into a description |
| 3 | consistency-pass | compound | `/consistency-pass <character>` | Regenerate every beat of a character after a bible change |
| 3 | scene-fill | compound | `/scene-fill <scene>` | Draft beats for every story point, then compose + generate |
| 3 | re-style | compound | `/re-style <scene> <description>` | Apply a style across a scene's beats |
| 3 | mood-pass | compound | `/mood-pass <scene> <mood>` | Apply a mood variant across a scene's beats |

Semantics notes:

- `/modify` changes the artifact's defining description (character sheet,
  scene summary) on text artifacts; on visual artifacts it routes to the
  mask-guided regional edit pipeline (see §5): segment → confirm mask →
  inpaint. The flagship use case is a character hair-style change; `mask` and
  `inpaint` are expected to ride with slice 1 for this reason.
- `/layer` carries intent only; where a layer goes is a drag afterward.
- **Slice 1 core (the five):** create, variant, regenerate, compose, draft —
  enough to exercise the full pipeline: artifact creation, visual iteration,
  composition, and LLM-assisted story text.

**Execution model (pending confirmation):** user-typed ops apply immediately
and are undoable via the operation log; only LLM-proposed op sequences go
through approve-before-apply. Rationale: slash ops are explicit intent;
free-form messages are suggestions, and generation is slow/expensive.

### From operations to pipelines (execution layer)

Two layers, kept distinct:

- **Operations (intent)** — the typed set above: semantic, logged,
  approval-gated, user/LLM-facing. The "what".
- **Pipelines (execution)** — deterministic sequences of primitives built
  with a typed builder in Rust; the "how" behind each operation.

Each operation compiles to a pipeline:

```rust
let pipeline = GenerationPipeline::new()
    .model("krea-2-turbo-q2")
    .size(768, 448)
    .steps(4)
    .reference_image(&revision.asset_path)
    .mask(&hair_mask.path)
    .lora("lora/character.safetensors", 0.8)
    .build();           // static validation happens here

let job = pipeline.run(&runtime).await?;
```

Rules:

- Each operation compiles to a pipeline; pure operations (`ask`, `summarize`)
  are plain LLM calls, not pipelines.
- The builder validates statically at `build()` (a mask requires a reference
  image; sizes within contract bounds); runtime failures surface as job
  statuses, unchanged.
- The VLLM (LM Studio) never produces code — it emits operation stacks as
  structured JSON (function calling against a schema Rust defines); Rust
  deserializes, validates, and only *proposes* for approval before executing.
- Slice 1 keeps stacks linear (a sequence of steps); parallel execution
  (e.g., variations grids) can layer on later.
- The operation log records each executed stack with its arguments.

**Typed intermediates.** Steps are not a flat list of side effects; they
produce values later steps consume, and the builder returns typed handles:

```rust
let pipeline = Pipeline::new()
    .reference_image(&revision.asset_path)
    .segment("hair")        // produces a MaskHandle
    .inpaint("bob cut")     // consumes the MaskHandle
    .build();
```

Handles resolve at run time; step kinds are checked statically, so a caption
cannot be fed into a mask slot. Mask steps include `segment`, `paint_strokes`,
`invert`, `feather`, `union`.

**The VLLM is also a pipeline primitive.** Besides proposing whole stacks it
can be a step inside one: `describe(image) -> caption/prompt`, `plan(text) ->
stack`, `critique(image, prompt) -> feedback`. LM Studio is external and may
be offline, so LLM steps are soft dependencies: they degrade to manual input
and never hard-fail a stack.

**Closed vocabulary.** The VLLM combines step kinds, it never invents them.
Step kinds are a Rust enum; the JSON schema for stacks contains only
enumerated kinds with validated parameters.

**Failure semantics.** Stacks are linear and fail fast: a failed step marks
the stack failed, keeps any intermediates produced so far, and the artifact's
revision status reflects it. No retries or branching in slice 1.

**Undo is state restore, not re-execution.** Replaying a stack with VLLM or
painted-mask steps cannot reproduce the same result, so undo restores the
prior project state (revisions already provide snapshot semantics); the log
is for provenance, not replay.

**Testability.** The layering makes the pipeline layer testable end to end:

1. **Builder unit tests** (no IO): static validation (mask without a reference
   image is a build error), step ordering, parameter bounds, stack
   deserialization from JSON fixtures.
2. **Execution tests with a fake backend:** pipelines run against a stubbed
   `VisionClient`; assert request bodies (prompt, mask, LoRA, size) and
   revision state transitions. The VLLM step is injectable the same way.
3. **Contract tests (Python):** the existing fake-based pattern in
   `python/tests` keeps both sides of the wire in sync.
4. **Manual golden runs:** a scripted harness executes one pipeline against
   the real backend and drops outputs (image, mask, composite) into a folder
   for human review — "is this mask the hair?", "did only the shirt change?".

For this to hold, image primitives must be **pure functions**. The composite
fallback then gets a property test: pixels outside the mask are bit-identical
to the original — "only the masked region changes" is assertable, while mask
semantics remain a human judgment (the manual tier). Steps carry **seeds** so
a golden run can be replayed identically when a manual validation fails.

**CLI (`svs`).** The pipeline API gets a thin clap-based CLI so operations can
be driven and manually validated without the UI. It mirrors the operation
set: clap parses args → constructs the same typed operation → compiles to a
pipeline → executes — one code path shared with the UI and the VLLM.

- `svs project open <path>` — select the SQLite project
- `svs op create <kind> <description>`, `svs op variant <ref> <desc>`,
  `svs op regenerate <ref> [prompt] --seed --steps --size`,
  `svs op mask <ref> <prompt> --out <dir>`, `svs op inpaint <ref> <prompt>
  --mask <path> --out <dir>`
- `svs stack run <stack.json>` — execute a serialized op-stack; doubles as
  the VLLM contract test bed, since the model emits the same JSON. `svs stack
  propose <message>` drives LM Studio to produce a stack from free text.
- `--out <dir>` on any op drops intermediates/outputs (image, mask,
  composite) into a folder for human review — the manual validation tier.
- `svs log <ref>` — show an artifact's operation log.

The CLI is a second binary in the same crate sharing the library modules
(config, models, vision client, runtime, operations/pipelines) without the
Dioxus UI: the crate gains a `src/lib.rs` with the shared modules; `main.rs`
(UI) and `src/bin/svs.rs` (CLI) both build on it.

## 8. Story text pipeline

- A single **story document** per story: premise → plot → bible (characters,
  environments) → scenes → beat narration. Written in stages; the LLM assists
  at each stage via `/draft`, always propose/approve — no auto-apply. This
  story document *is* the `.sf` file (see §9); there is no separate
  `story_documents` table.
- **Recursive refinement via provenance:** beats/scenes store *references* to
  characters, variants, and locations by id. When the bible changes, affected
  beats are flagged ("uses Kitchen v2 — re-sync?") and re-sync is
  mode-dependent (Section 4). No automatic cascade.
- **Mixing text and visuals is bidirectional:** beat narration → image prompt
  with bible context auto-injected; generated image → narration text via
  captioning (captioning endpoint is currently a placeholder).

## 9. Storyfella DSL (built in this project)

Storyfella is a DSL and toolkit for interactive narrative experiences. It is
**built from scratch in this repository** — the earlier `../storyfella`
prototype is reference material only, not code to integrate. Modern Rust
stack: a workspace-member crate (`storyfella-dsl`) with its own lexer,
parser, compiler, CLI, and runtime; serde for wire schemas; clap for the
CLI. The sequencer becomes the visual-authoring surface for `.sf` projects.
Three surfaces share one artifact registry:

| Surface | Role |
| --- | --- |
| Story Writer | Edits `.sf` with syntax highlighting; asset references resolve to live chips (name, thumbnail) as you type; click a chip to jump to canvas |
| Canvas | Creates/edits the artifacts the `.sf` references |
| Studio | Sequence/timeline view, *derived* from `.sf` structure (scenes in order, choices as branches) |

Decisions:

- **Text-driven sync:** writing `scene kitchen:` in the writer auto-creates
  or links the kitchen scene artifact; the registry follows the text. The
  canvas is the visual-detail editor for things the text only names.
- **`.sf` is the story source of truth** — diffable, greppable, versionable.
  SQLite keeps only the asset side (artifacts, revisions, masks, operation
  log); the `story_documents` table is dropped from the schema.
- **References by name in the DSL** (`character: mia`), ids under the hood;
  the compiler catches collisions; the writer's autocomplete resolves names
  to ids as you type.
- **Validation extends to assets:** `character: mia` where `mia` does not
  exist is a compile error, with the canvas as the fix surface.
- **The writer reuses the tokenized-editor primitive** designed for the
  prompt bar (`/op` highlighting, `c:` chips) — one editor component, two
  uses.
- Per-beat visual references (variant, composition mode) in the DSL are the
  next design layer below this one.

## 10. Persistence: SQLite

Decision: SQLite replaces the TOML `ProjectStore` as the project format, with
a one-time import path from existing TOML files. Assets (PNGs) stay as files
on disk; the database stores relative paths (this also fixes the known
"absolute asset paths" gap). Schema versioning via a `schema_meta` table.

```sql
CREATE TABLE schema_meta (version INTEGER NOT NULL, applied_at TEXT NOT NULL);

CREATE TABLE projects (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

-- One id space for every artifact; c:<ref> references resolve here.
CREATE TABLE artifacts (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id),
  kind TEXT NOT NULL CHECK (kind IN
    ('story','scene','beat','character','environment','object')),
  name TEXT NOT NULL DEFAULT '',
  description TEXT NOT NULL DEFAULT '',
  variant_of_id TEXT REFERENCES artifacts(id),   -- NULL = base artifact
  variant_axis TEXT,      -- outfit | age | body | hair | expression |
                          -- time-of-day | weather | season | mood
  parent_id TEXT REFERENCES artifacts(id),       -- scene→story, beat→scene, room→building
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE revisions (
  id TEXT PRIMARY KEY,
  artifact_id TEXT NOT NULL REFERENCES artifacts(id),
  prompt TEXT NOT NULL,
  asset_path TEXT,               -- relative to project asset dir
  status TEXT NOT NULL,          -- queued | generating | completed | failed | cancelled
  seed INTEGER,
  model TEXT,
  created_at TEXT NOT NULL
);

-- Masks are a view over one specific revision image.
CREATE TABLE masks (
  id TEXT PRIMARY KEY,
  revision_id TEXT NOT NULL REFERENCES revisions(id),
  asset_path TEXT NOT NULL,
  source TEXT NOT NULL,          -- auto (segmentation) | painted
  prompt TEXT,                   -- grounding text or box JSON
  score REAL
);

-- Beat composition layers.
CREATE TABLE layers (
  id TEXT PRIMARY KEY,
  beat_id TEXT NOT NULL REFERENCES artifacts(id),
  position INTEGER NOT NULL,
  artifact_ref TEXT NOT NULL REFERENCES artifacts(id),
  variant_ref TEXT REFERENCES artifacts(id),
  role TEXT NOT NULL,            -- backdrop | baked | dynamic
  anchor TEXT                    -- position/size JSON
);

CREATE TABLE lora_registry (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id),
  name TEXT NOT NULL,
  path TEXT NOT NULL,            -- relative to configured lora_dir
  base_model TEXT,
  multiplier REAL NOT NULL DEFAULT 1.0,
  artifact_id TEXT REFERENCES artifacts(id)  -- optional binding
);

-- Every applied/proposed operation; the basis for undo and provenance.
CREATE TABLE operation_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id TEXT NOT NULL REFERENCES projects(id),
  artifact_id TEXT REFERENCES artifacts(id),
  op TEXT NOT NULL,              -- name from the typed operation set
  args TEXT NOT NULL,            -- JSON
  origin TEXT NOT NULL,          -- user | llm
  status TEXT NOT NULL,          -- proposed | applied | rejected | reverted
  created_at TEXT NOT NULL
);

-- Story prose: not stored in the database. `.sf` files (see §9) are the
-- story source of truth; SQLite stores the asset side only.
```

Implementation notes: prefer `rusqlite` (bundled, synchronous) with a single
writer connection behind a mutex and WAL mode for a Dioxus desktop app.
*(Crate choice not yet confirmed.)*

## 11. Open questions

1. **Canvas vs Studio:** creation canvas as the main workspace, or
   side-by-side with Studio for sequencing? *(Lean: side-by-side.)*
2. **Naming:** beat vs slide vs panel.
3. **Execution model:** user-typed ops instant + undo; LLM proposals gated by
   approve. *(Proposed, pending confirmation.)*
4. **Short-id format** for `c:<ref>` chips.
5. **Layered-mode matte quality** acceptance for final output.
6. **Krea mask input:** validate whether the native backend accepts a mask
   directly or the adapter composites the inpaint region.
7. **SQLite crate** (`rusqlite` vs `sqlx`) and TOML coexistence during
   migration.

## 12. Suggested implementation order

Active directive: **the API first — no SQLite and no GUI work for now.**
The in-memory model + TOML `ProjectStore` stay as-is while the API is built
and validated through tests and the CLI.

1. **The API:** artifact domain model (artifact kinds, variants, scenes,
   beats — the registry), typed operation set + pipeline builder (closed
   vocabulary, typed intermediates, static validation at `build()`, linear
   fail-fast stacks), `mask_path` in the generation contract, and the
   slice-1 op compilers (create, variant, regenerate, compose, draft, plus
   the mask-edit path).
2. **The CLI (`svs`):** ops, `stack run`/`propose`, `--out` golden runs —
   drives and validates the API without the UI.
3. **SQLite foundation (deferred):** schema + storage layer (§10), one-time
   TOML import.
4. **The canvas:** artboard viewport, artifact cards, prompt bar (slash
   parsing, autocomplete, `c:` chips).
5. **Studio integration:** scenes group the timeline; re-sync flows per
   mode.
6. **LoRA registry** UI + auto-injection into generations.
7. **Storyfella DSL (in-repo):** the language crate (lexer/parser/compiler,
   CLI), the writer with the tokenized editor, asset chips and autocomplete;
   DSL asset references + validation; round-trip to canvas.
8. **LSP-style tooling** as the DSL matures (go-to-definition, refactor,
   diagnostics).

## 13. Implementation handoff

Design status: this document is design notes; nothing in it is implemented.
The current codebase is the pre-design product (prompt → storyboard beat →
Krea revision flow). Work follows §12.

### Entry points (existing code)

- `src/models/mod.rs`, `src/models/persistence.rs` — current
  Project/StoryboardFrame model + TOML store; stays as-is for now (SQLite is
  deferred). The artifact registry (kinds, variants, scenes) is new domain
  model work layered onto this.
- `src/state/mod.rs` — AppState invariants (beat ↔ clip, revisions,
  selection).
- `src/vision/mod.rs` ↔ `python/models/schemas.py` — Rust/Python HTTP
  contracts; `mask_path` must be added to `GenerateRequest`.
- `src/runtime/creative_runtime.rs`, `src/runtime/generation_runtime.rs` —
  process lifecycle + job orchestration the pipeline builder wraps.
- `src/llm/` — LM Studio client; the operation registry (this design) is the
  planner business logic it will serve.

### Open decisions to settle first (with the user)

1. Execution model: user-typed ops instant + undo; LLM proposals gated by
   approval — proposed, unconfirmed.
2. Approval granularity: whole stack vs per-step checkpoints (the hair-mask
   confirm is a checkpoint).
3. Short-id format for `c:` references.
4. Native mask support for Krea (open question 6) — decides whether the
   composite fallback is primary.
5. Deferred with SQLite: crate choice (`rusqlite` vs `sqlx`).

### Pitfalls

- Krea 2 is the focus model (native, via stable-diffusion.cpp); do not treat
  "sd" as the product model.
- Undo is state restore, never pipeline re-execution.
- VLLM vocabulary is closed; LLM steps are soft dependencies.
- The builder is substrate: linear, fail-fast, no branching/DAG in slice 1.
- Storyfella is built from scratch in this repo; `../storyfella` is reference
  material only.

# Artifact canvas — product design & roadmap

> **Status: design discussion notes with the API slice implemented.** Captures
> the ongoing product-design conversation around the artifact registry, the
> creation canvas, the typed operation set, and SQLite persistence. **§12 is
> the feature/status tracker** — every planned feature with its status; update
> it when work lands. The API slice (§12 items 1–2) is implemented — see
> `docs/api-slice-1.md`; confirmed decisions and open questions are marked as
> such.

## 1. Vision recap

The product lets a user write a visual story and create its slides, iterating
on both until the result is right. The current app already covers prompt →
storyboard beat → Qwen-Image-2.1 generation → revision history. The target
shape adds:

- a shared **artifact registry** (characters, environments, objects, scenes,
  beats) with persistent visual variants;
- a **creation canvas** workspace where artifacts are created and edited
  through language (`/create …`, `/modify …`) plus direct manipulation;
- LLM-assisted story writing (plot → bible → scenes → beat narration) with
  propose/approve semantics;
- full per-beat generation history and a LoRA
  registry for character/environment consistency.
- the **Storyfella DSL** as the authored story surface: `.sf` files that
  reference registry artifacts, with the canvas as the visual-asset editor
  (see §9).

**Model focus:** Qwen-Image-2.1 (served by the native `stable-diffusion.cpp`
process) is the only generation model. The stack stays as documented in
`architecture.md`; Qwen-Image-2.1's prompt-following and quality are the
baseline and LoRAs build on top of it. Krea 2 and the older SAM/segmentation
edit path were removed in the Qwen migration.

## 2. Vocabulary

| Term | Meaning |
| --- | --- |
| Beat | One slide/panel of the visual story. A beat is a composition of layers referencing other artifacts. |
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
| Baked | Everything in one Qwen-Image-2.1 generation (today's flow). Max quality. | Regenerate the region. |
| Layered | Environment base + character/object cutouts composited by the app. | Instant: swap variant/expression and recomposite. |
| Hybrid | Baked backdrop (environment + background characters baked in) with dynamic character layers composited on top. | Only dynamic layers regenerate. |

Consequences:

- The beat needs a **composition spec** separate from the rendered image:
  `background` (environment variant ref or "generate fresh"), `layers:
  [{artifact_ref, variant_ref, role, anchor}]`, and `mode`.
- **Re-sync is mode-dependent:** layered → recomposite; baked → regenerate; hybrid → regenerate dynamic layers only.
- **Cutouts come from the model's prompt-driven alpha channel**, not from
  segmentation + matting: Qwen-Image-2.1 renders an alpha channel when the
  prompt asks for transparency, and it survives only in PNG/WebP (verified
  against upstream's own examples). Cutout quality is subject-dependent —
  reliable for logo/object subjects, weaker for some portraits — so layered
  mode is the iteration mode while baked/hybrid dominate final quality.
  *(Cutout quality acceptance: open.)*
- **Positioning is direct manipulation, not an operation.** Ops carry intent
  (what), gestures carry geometry (where, size, z-order).

## 5. Generation & consistency

- **LoRA registry.** Existing plumbing: `--lora-model-dir` on `sd-server`, up to
  eight `LoraSelection { path, multiplier }` per generation request
  (`clipforge_inference::sd::LoraSelection`, in the shared crate). The project-level
  registry (name, file,
  base model, tags), binding a LoRA to an artifact, and auto-injection into
  every generation referencing that artifact are implemented (§12 item 6).
- **Character/environment consistency** is achieved by injecting canonical
  reference images and bound LoRAs into every generation that references the
  artifact. Trained character LoRAs are expected to be the main lever.

### Regional edits — `modify` is an instruction edit

The flagship use case is a visual `modify` on a character ("change her hair
style"). It is a single operation, not a mask sequence:

1. `modify <char> <description>` — the change description is grounded (with LLM
   help) against the artifact's description, producing a plain-language
   instruction.
2. `generate` — the artifact's active revision is the reference image and the
   grounded instruction is the prompt; the model re-renders, preserving what the
   instruction did not mention.
3. The result is stored as a new revision; `/describe` promotes an edit to the
   artifact's canonical look, and `/variant` forks it into its own artifact.

`modify` is **not** localized by a user-selected mask: the instruction's
semantics decide what changes. The earlier mask/inpaint/composite flow
(segmentation, mask confirmation, inpaint) was removed with the Qwen migration —
there is no mask source, no segmentation step, and no composite. Localization is
a quality goal of prompt-following, not a region drawn by the user.

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
| 1 | revert | primitive | `/revert <ref>` | Re-activate the previous image; nothing is deleted, the history keeps both |
| 1 | compose | primitive | `/compose <scene> <description> [layer refs…]` | New beat in a scene |
| 1 | draft | primitive | `/draft <ref> <request>` | LLM proposes story text; user approves |
| 2 | modify | primitive | `/modify <ref> <description>` | Change the artifact's defining text |
| 2 | write | primitive | `/write <ref> <text>` | Direct text (narration, premise, …) |
| 2 | rename | primitive | `/rename <ref> <name>` | Rename |
| 2 | delete | primitive | `/delete <ref>` | Remove (cascade rules TBD) |
| 2 | outpaint | primitive | `/outpaint <ref> <description>` | Extend the image beyond its borders |
| 2 | layer | primitive | `/layer <beat> <ref> [role] [variant]` | Add/swap a layer (role: backdrop / baked / dynamic); geometry added by drag afterward |
| 2 | promote | primitive | `/promote <ref>` | Mark a revision as the artifact's canonical reference image |
| 2 | attach-lora | primitive | `/attach-lora <ref> <lora> [multiplier]` | Bind a registry LoRA; auto-injected into the artifact's generations |
| 2 | import | primitive | `/import <file>` | Bring an external image in as a revision |
| 2 | ask | pure | `/ask <question>` | LLM Q&A about the story; no mutation |
| 2 | variations | compound | `/variations <ref> <n>` | Generate an N-revision grid for comparison |
| 2 | re-sync | compound | `/re-sync <ref>` | Regenerate/recomposite every beat referencing the artifact |
| 3 | cutout | primitive | `/cutout <ref>` | Extract subject to a transparent PNG using the model's prompt-driven alpha channel, for layered mode |
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

- **Visual `create`/`variant` generate the first image** as part of the
  operation (prompt = description; the variant's first image is an edit of
  the base — its active image as reference, the size it inherits). A failed
  generation keeps the artifact with a failed revision; `/regenerate` is
  the retry. Text-only kinds (story, scene, beat) still apply instantly.
- `create`/`variant` accept `--name`; without it, names derive from the
  description (create) or the base (variant: `{base}-{axis}`/`{base}-{slug}`).
- `/modify` on a visual artifact is an **instruction edit** (see §5): the
  artifact's active revision is the reference image and the grounded
  description is the prompt. There is no mask/segment/inpaint path. The
  flagship use case is a character hair-style change.
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
    .model("qwen-image-2.1")
    .size(1024, 1536)
    .steps(25)
    .reference_image(&revision.asset_path)
    .lora("character.safetensors", 0.8)
    .build();           // static validation happens here

let job = pipeline.run(&runtime).await?;
```

Rules:

- Each operation compiles to a pipeline; pure operations (`ask`, `summarize`)
  are plain LLM calls, not pipelines.
- The builder validates statically at `build()` (sizes within contract bounds,
  step counts, LoRA count); runtime failures surface as job
  statuses, unchanged.
- The VLLM (the LLM endpoint) never produces code — it emits operation stacks as
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
    .generate("give her a bob cut", Some(image))
    .build();
```

Handles resolve at run time; step kinds are checked statically, so a caption
cannot be fed into an image slot. There is no mask handle — the closed step
vocabulary is `LoadImage`, `Checkpoint`, `Generate`, `Llm`.

**The VLLM is also a pipeline primitive.** Besides proposing whole stacks it
can be a step inside one: `describe(image) -> caption/prompt`, `plan(text) ->
stack`, `critique(image, prompt) -> feedback`. The LLM endpoint is external
and may be offline, so LLM steps are soft dependencies: they degrade to
manual input and never hard-fail a stack.

**Closed vocabulary.** The VLLM combines step kinds, it never invents them.
Step kinds are a Rust enum; the JSON schema for stacks contains only
enumerated kinds with validated parameters.

**Failure semantics.** Stacks are linear and fail fast: a failed step marks
the stack failed, keeps any intermediates produced so far, and the artifact's
revision status reflects it. No retries or branching in slice 1.

**Undo is state restore, not re-execution.** Replaying an LLM-derived stack
cannot reproduce the same result, so undo restores the
prior project state (revisions already provide snapshot semantics); the log
is for provenance, not replay.

**Testability.** The layering makes the pipeline layer testable end to end:

1. **Builder unit tests** (no IO): static validation, step ordering,
   parameter bounds, stack
   deserialization from JSON fixtures.
2. **Execution tests with a fake backend:** pipelines run against a stubbed
   `GenerationBackend`; assert request bodies (prompt, reference, LoRA, size) and
   revision state transitions. The VLLM step is injectable the same way.
3. **Wire serialization tests:** the `sdcpp` request/job types
   (the shared `clipforge-inference` crate, `../clipforge/crates/inference/src/sd/`)
   round-trip.
4. **Manual golden runs:** a scripted harness executes one pipeline against
   the real backend and drops the output image into a folder
   for human review.

Steps carry **seeds** so
a golden run can be replayed identically when a manual validation fails.

**CLI (`svs`).** The pipeline API gets a thin clap-based CLI so operations can
be driven and manually validated without the UI. It mirrors the operation
set: clap parses args → constructs the same typed operation → compiles to a
pipeline → executes — one code path shared with the UI and the VLLM.

- `svs project open <path>` — select the SQLite project
- `svs op create <kind> <description>`, `svs op variant <ref> <desc>`,
  `svs op regenerate <ref> [prompt] --seed --steps --size`,
  `svs op modify <ref> <description>`
- `svs stack run <stack.json>` — execute a serialized op-stack; doubles as
  the VLLM contract test bed, since the model emits the same JSON. `svs stack
  propose <message>` drives the LLM endpoint to produce a stack from free text.
- `--out <dir>` on any op drops the output image into a folder for human
  review — the manual validation tier.
- `svs log <ref>` — show an artifact's operation log.

The CLI is a separate binary crate sharing the domain modules
(config, generation client, runtime, operations/pipelines) without the
Dioxus UI: `crates/studio-core` holds the shared modules; `apps/studio/src/main.rs`
(UI) and `apps/cli/src/bin/svs.rs` (CLI) both build on it.

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
**built from scratch in this repository** — the sibling
`../storyfella-old-project-inspiration` prototype is reference material only,
not code to integrate. `storyfella-language` owns lexing, parsing, AST and shared
analysis; `storyfella-lsp` exposes it to external editors. Compiler and runtime
are independent crates. See [the language contract](language.md)
and [execution plan](language-plan.md). The sequencer becomes the
visual-authoring surface for `.sf` projects.
Three surfaces share one artifact registry:

| Surface | Role |
| --- | --- |
| Story Writer | Edits `.sf` with syntax highlighting; asset references resolve to live chips (name, thumbnail) as you type; click a chip to jump to canvas |
| Canvas | Creates/edits the artifacts the `.sf` references |
| Studio | Sequence/timeline view, *derived* from `.sf` structure (scenes in order, choices as branches) |

Decisions:

- **Text-driven sync:** writing `scene kitchen` in the writer auto-creates
  or links the kitchen scene artifact; the registry follows the text. The
  canvas is the visual-detail editor for things the text only names.
- **`.sf` is the story source of truth** — diffable, greppable, versionable.
  SQLite keeps only the asset side (artifacts, revisions, operation
  log); the `story_documents` table is dropped from the schema.
- **References by name in the DSL** (`character: mia`), ids under the hood;
  the compiler catches collisions; the writer's autocomplete resolves names
  to ids as you type.
- **Validation extends to assets, but unresolved is *pending*, not an
  error:** `character: mia` where `mia` does not exist is amber with a
  Create affordance, not red. Same information, opposite valence — the story
  reads as a work queue rather than a list of mistakes, and the canvas is
  the fix surface. (Settled while building the writer; see
  `docs/design/ui-plan.md` §3.)
- **The writer reuses the tokenized-editor primitive** designed for the
  prompt bar (`/op` highlighting, `c:` chips) — one editor component, two
  uses.
- Per-beat visual references (variant, composition mode) in the DSL are the
  next design layer below this one.

## 10. Persistence: SQLite

Decision: SQLite is the project format (`.svs-project.db`); the pre-SQLite
formats (TOML `ProjectStore`, stopgap JSON registry) were removed with their
one-time imports — the current format evolves via `schema_meta` migrations.
Assets (PNGs) stay as files on disk; portable project-relative image paths remain
planned (the current generator stores absolute paths). Schema versioning
via a `schema_meta` table. Applied so far: v1 (§10 tables), v2/v3 (the
legacy storyboard column, added then dropped), v4 (per-artifact default
size), v5 (per-artifact default steps and pinned seed), v6 (`RevisionOrigin`
on revisions), v7 (`projects.story_source`, the Writer's text), v8 (drops the
`masks` table with the removed mask edit path), v9 (exports Writer text to `.sf`
before dropping `projects.story_source`).

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

-- Masks are gone: `GenerateRequest` has no mask field and `modify` is an
-- instruction edit (see §5); schema v8 dropped the table.

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

Implementation notes: **resolved** — `rusqlite` (bundled, synchronous) with a
single writer connection behind a `parking_lot` mutex and WAL mode, in
`crates/studio-core/src/persistence/` (`ProjectDb`). Deliberate deviations from the sketch
above: timestamps are unix epoch integers (no chrono dependency),
`operation_log.id` is TEXT (the model uses `Uuid`), `op` holds the serde tag
name with the full `Operation` JSON in `args`, `artifacts.composition_mode`
holds the beat-only composition mode, `artifacts.drafts_json` is the
home for artifact-scoped generated text proposals, and `revisions.error` covers
the model field. Writes replace the registry rows in one transaction with
`PRAGMA defer_foreign_keys` (the circular `artifacts ↔ revisions`
reference).

## 11. Open questions

1. **Canvas vs Studio:** creation canvas as the main workspace, or
   side-by-side with Studio for sequencing? *(Lean: side-by-side.)*
2. **Naming:** beat vs slide vs panel.
3. **Execution model:** user-typed ops instant + undo; LLM proposals gated by
   approve. *(Proposed, pending confirmation.)*
4. **Short-id format** for `c:<ref>` chips.
5. **Layered-mode cutout quality** acceptance for final output (Qwen-Image-2.1's alpha channel is subject-dependent).
6. **Native mask input** — **resolved by removal:** the mask/inpaint path was deleted with the Krea/SAM migration; `modify` is an instruction edit and there is no mask source.
7. **SQLite crate** — **resolved:** `rusqlite` (bundled), one writer
   connection + WAL (`crates/studio-core/src/persistence/`). The TOML `ProjectStore` and the
   stopgap JSON registry were removed with their imports; formats evolve
   via SQLite migrations.

## 12. Suggested implementation order

**Workspace restructuring: implemented.** Studio is `apps/studio`
(`storyfella-studio`), with GUI-free `crates/studio-core`
(`storyfella-studio-core`). `apps/cli` and `xtask` remain; `apps/player`
is a visual Dioxus desktop host. Language, compiler,
LSP and story runtime are implemented as defined in
[`architecture.md`](architecture.md#storyfella-workspace-boundaries).
Studio preview and Player share execution, resource checks and controls. Studio
now edits authoritative `.sf` files; [the language execution plan](language-plan.md)
tracks the remaining integration and execution work.

**This section is the status tracker for planned work** — every feature we
plan to ship, with its status; update it when work lands.

**Atelier relocation: implemented.** `apps/atelier` (`storyfella-atelier`, binary
`atelier`) is the separate GPUI art app, moved from FineWineTTS with its current
sources, project/data identity and macOS renderer patch. Studio remains Dioxus.
**Shared hosted image execution: implemented in Atelier.** Clipforge owns the
async adapters for Replicate, fal.ai, Alibaba and GPT Image, shared presets,
job polling/cancellation and result metadata. Atelier retains its credential
and document boundaries. **Art editing and reuse: implemented.** Full-resolution
originals, a cross-project library, zoomable inspection, version branches,
before/after, editable image layers with placement, durable masks and supported
multi-reference editing are available in Atelier. Approved originals and painted
canvases import into Studio's completed revisions through portable `.sfart`
packages. Studio's hosted-provider selection and a text/vision art assistant
remain **planned**; see [Atelier](atelier.md) for limits and verification.

**Compact Atelier workspace and portable key storage: implemented.** Named icon
actions and optional image details follow DESIGN.md. Provider keys persist in
Atelier user app data across platforms; Keychain integration is deferred.
**Iteration gaps: implemented and checked on macOS.** The real-provider bookshop
walkthrough demonstrates chained edits and sibling branches. Source-specific
prompt drafts recover after restart, saved version names retain identity, prompt
reuse is available, and the repeated native Open check passes. First-time-user
and cross-platform acceptance remain outstanding. Local Qwen Image 2.1
generation/editing and Krea 2 Turbo generation are connected through Clipforge;
actual Linux CUDA inference requires a run on that machine.
See [iteration acceptance](atelier-iteration-acceptance.md).

1. **The API:** artifact domain model (artifact kinds, variants, scenes,
   beats — the registry), typed operation set + pipeline builder (closed
   vocabulary — `LoadImage`/`Checkpoint`/`Generate`/`Llm`; typed intermediates,
   static validation at `build()`, linear fail-fast stacks), and the
   slice-1 op compilers (create, variant, regenerate, compose, draft, modify).
   — **✅ implemented** (`docs/api-slice-1.md`); the Qwen-Image-2.1 migration
   removed the mask/inpaint path and made `modify` a reference + grounded
   instruction. Live validation of `modify`/`stack propose` on Linux/CUDA pending
2. **The CLI (`svs`):** ops, `stack run`/`propose`, `--out` golden runs —
   drives and validates the API without the UI. — **✅ implemented**
3. **SQLite foundation:** schema + storage layer (§10) — `crates/studio-core/src/persistence/`
   (`ProjectDb`: versioned migrations via `schema_meta` (v1 §10 tables, v2
   `project_json` for the storyboard), WAL, full-replace snapshot writes,
   lossless round-trip); the CLI persists to `.svs-project.db`; the GUI
   opens/creates projects from the Projects screen and saves via autosave
   or Cmd/Ctrl+S. — **✅ implemented**
4. **The canvas:** the artifact workspace in two states — a grid at rest
   (variants stacked behind their base, in-flight work dimmed and badged in
   place) and an image-dominant detail pane with an inspector rail, whose
   every control is also a command. The tokenized command bar and the
   `Ctrl+K` palette are further views of the same typed vocabulary as the
   `svs` CLI (`/create`, `/variant`, `/regenerate`, `/modify`, `/set`);
   undo/redo via registry snapshots. Revisions carry a `RevisionOrigin` so
   canon and experiment are distinguishable, and `Promote` (`/describe`) is
   what makes an experiment canonical. The kind-grouped sidebar it replaced
   was deleted with it. See `docs/design/ui-plan.md`. Artifacts carry an optional default generation size
   (`/create … --size WxH`; per-kind defaults otherwise: character 512×768,
   object 768×768, environment/scene/beat 1024×576) that `regenerate`/
   `modify` honor. Visual `create`/`variant` generate the artifact's first
   image as part of the operation (variants reference the base's active
   image); a failed generation keeps the artifact with a failed revision,
   and `/regenerate` is the retry. `modify` runs unattended — an instruction
   edit has no checkpoint and no segmentation step. `/revert` re-activates the
   previous completed image (a Direct op — no render); the history keeps both,
   nothing is deleted, and the reverted-from revision can be re-activated from
   the detail pane. It is on the slash bar, the palette, and `svs op revert`.
   Free-form
   composer messages (no leading `/`) go to the LLM, which proposes an
   operation stack against the closed vocabulary; the proposal is shown in
   an approval modal with per-op toggles before any op runs (LLM proposals
   are gated, per the execution model). The legacy storyboard model was
   removed
   with it (`src/models/`, `src/timeline/`, schema v3 drops `project_json`).
   — **✅ implemented**; `c:` autocomplete and command syntax highlighting
   are implemented. Artboard pan/zoom remains deferred (§6).
5. **Generation quality controls:** configurable per-kind default sizes
   (`[generation.sizes]`), default step count (`[generation] steps`), and
   the Settings screen to edit them live (persisted to `config/storyfella.toml`,
   applied to the next generation — canvas and CLI). The LLM expresses
   size/steps/adapters only when the user asks (the visual ops carry
   `size`/`steps`/`loras`; the `resize` op sets the default size and
   regenerates at it) — never invented. The planner discovers the registry
   and adapters via tool calling (`list_artifacts`/`list_loras`) with a
   strict schema on the final stack. `--name` on `variant` (explicit
   naming for variants). A Settings → Services toggle (`[runtime]
   prewarm`) starts the resident server when a project opens, so the first
   generation is not a multi-minute weight load. **Residency** is a launch
   profile per `sdcpp` provider (`resident` / `lowvram` = `--offload-to-cpu`),
   selectable in Settings → Services and validated at config load; the
   measured tradeoff (lowvram ≈9/16 GiB peak for +7%/+0.5% sampling; `--max-vram`
   rejected as too costly for edits) is in `docs/runtime-lifecycle.md`. A server
   holding the port that the app did not start is reported as `Degraded` (the
   resident model is named) and cleared with **Take over** on the image service
   — identified by the served executable and listen port, not a `pkill` pattern.
   A grounding regression harness (`.svs-e2e/scripts/grounding_suite.py`) runs a
   fixed edit suite against one or more planner models and judges each with
   `svs eval --expect`, so planner quality is comparable by command.
   **Image assessment in the plans:** a driver `expect_vision` step attaches an
   artifact's current image to a question and asserts a substring gate, so a
   plan checks *what* was generated rather than only that an image exists. The
   image-carrying chat call lives in `clipforge-inference`
   (`chat::Client::complete_with_images`); storyfella's `crates/studio-core/src/vision.rs`
   (
   `VisionSession`) adapts config and images to it, and `svs eval` uses the same
   path.
   — **✅ implemented**
6. **LoRA registry:** project-level adapter registry (SQLite
   `lora_registry` table, round-tripped) with per-artifact bindings
   (`/attach-lora`, `/lora-scale`, `/lora-remove` — slash, CLI, and LLM
   vocabulary), config-level use-case bindings (`[lora]`:
   per-kind defaults, one multiplier), discovery
   from `lora_dir` (recursive — subfolders and symlinked model trees included; a
   refresh icon in Settings rescans), and auto-injection into
   every generation (artifact-bound first, then kind default;
   per-run `--lora <file>@<mult>` flags on
   the CLI). — **✅ implemented**
 7. **Error codes and reporting:** a central registry (`crates/studio-core/src/errors.rs` —
    `ErrorCode`, one stable `SVS-<DOMAIN>-<NNN>` per failure, with a short
    message and a remediation hint) that every `thiserror` type implements
    via `CodedError` (transparent variants delegate to the inner code).
    `CodedError::report` builds the `ReportedError` payload the UI consumes —
    a global error toast (code + short message) with a Details action
    opening the full report modal (error text, request context, hint, and
    Copy — button and click-on-text, via `document::eval` with the
    execCommand fallback). The report carries a structured `context` list
    (operation summary, generation prompt, model, size, steps, seed, job
    id, work dir) attached at the failure site — `PipelineError::Backend`
    keeps the `GenerateRequest` context from `CreativeBackend::generate`,
    and the canvas attaches op/seed/work-dir — so a failed generation is
    answerable from one log line (`report.log()`, the single log point,
    emitted by the surface right before showing the report). Wired through
    the canvas ops, free-form proposals, autosave, the Projects screen,
    Settings service/model errors, and the `svs` CLI op paths (which also
    init a tracing subscriber so reports actually log). — **✅ implemented**
8. **Studio integration:** scenes group the timeline; re-sync flows per
   mode (compose beats in the canvas, story sequencing). — **🔧 active**
   Writer/Canvas reconciliation is implemented: valid source order maps authored
   scene/beat keys to persisted UUIDs; invalid drafts retain the last association;
   removals detach artwork. Joint source/registry undo, pending catalog creation,
   source origins and backlinks are connected. Writer keyboard completion,
   exact caret navigation, explicit identity-preserving rename, formatting and
   compiler diagnostics are implemented. Fixed-snapshot visual preview is
   implemented; source changes require applying a new draft. Re-sync remains.
9. **Storyfella DSL (in-repo):** `storyfella-language` (lexer/parser/shared
   analysis) and `storyfella-compiler` (validation/lowering); DSL asset references
   and round-trip to canvas. `storyfella-runtime` owns story execution,
   state and timing, shared by Studio preview and standalone Player.
   — **🔧 first slice implemented.** `storyfella-language` provides lossless
   tokens, a recovering typed AST, source positions, diagnostics, symbol/reference
   analysis and shared completion/navigation. Writer highlighting, scope and asset
   references now consume it. See [language.md](language.md) for the syntax.
   `storyfella-compiler` now lowers versioned, source-mapped plans with sequential
   scene fallthrough, terminal-flow validation and immutable asset readiness;
   `svs story check`/`compile` expose it. See [compiler.md](compiler.md).
   Studio opens/saves authoritative `.sf` files beside project databases.
   Schema v9 exports existing Writer drafts before removing the source column;
   disk conflicts, copy/reload and project pair moves are covered. See
   [story-files.md](story-files.md). Schema v10 persists source/registry associations.
   The shared headless runtime and CLI trace are implemented: typed events,
   manual advance/choice/jump/end/pause/restart, bounded automatic transitions,
   imported-plan validation and exact-plan save/restore. See [runtime.md](runtime.md).
   Studio preview and standalone visual Player share checked resources and
   Dioxus controls: still visuals, text/choices, pause/restart, save/restore and
   source snapshots/Writer return. The 22-step fixture-backed desktop plan checks
   actual execution. Both hosts now use a minimal widescreen stage with
   location artwork, softly shaded text and choices, and secondary controls
   in the options panel. See [playback.md](playback.md). Portable image paths,
   packaging/export and expressive media/voice remain planned.
10. **LSP tooling:** `storyfella-lsp` exposes shared language analysis to
    external editors. — **🔧 single-document tooling implemented.** Stdio, full-document
    sync, UTF-16, versioned diagnostics, completion, definitions, references and
    scene/beat symbols, resolved rename, conservative formatting and semantic
    tokens are supported. Explicit catalog export/refresh supplies shared
    compiler diagnostics for unsaved buffers. Workspace imports remain planned.
11. **Self-contained distribution (no user-side dependencies):** one
   downloadable artifact per platform (AppImage/`.deb` on Linux, `.app`
   bundle on macOS, installer/portable zip on Windows) containing the app
   binary and a release-built `sd-server` (CUDA or CPU flavor). The Rust
   spawn/supervision/readiness architecture is unchanged; the only code change
   is resolving `sd-server` relative to the bundle root instead of the
   config-relative path. Model weights stay out of
   any bundle (multi-GiB, licensed) and keep provisioning through
   `model_setup` — or, in the app, Settings → Services → "Model
   files", which drives the same `ModelStore` download with byte-level
   progress. The pinned manifest was verified against the live Hugging Face
   repo (2026-10-01): the three `Comfy-Org/Qwen-Image-2.1` remote paths,
   sizes, and SHA-256 digests all match. Explicitly out: embedding sd.cpp into the Rust process —
   the C API exists, but in-process inference would rework the entire
   runtime-lifecycle machinery for no user-facing gain. **Bundling mechanism
   (verified 2026-09-30):** the CLI itself bundles Linux AppImages —
   `dx bundle --platform desktop --package-types appimage` (run from
   `apps/studio`) — so no extra crate is needed; it lays out an `.AppDir` and
   drives `linuxdeploy`, which it downloads into `~/.local/share/.dx/tools`
   (the only networked step, run with `APPIMAGE_EXTRACT_AND_RUN=1` so FUSE is
   not required). `Dioxus.toml` now
   carries the bundle identity (`[application] name`, `[bundle] identifier`/
   `publisher`/`category`/descriptions); `[bundle] external_bin` and
   `[bundle] resources` are the hooks for the `sd-server`
   payload. Still missing for a shippable bundle: an icon (PNG) and that
   payload, plus resolving it relative to the bundle root. — **⏳ planned**

12. **Studio UI/UX remediation:** shared interaction, visual and accessibility
    rules and the 2026-10-07 app-wide audit are documented in
    [`DESIGN.md`](../DESIGN.md). All 25 findings now have implementation changes:
    direct error reports, shared native dialogs, draft/Quit protection, retained
    command input, truthful settings/runtime feedback, accessible controls,
    readable styling, useful recents/filters and safe preview replacement.
    Model storage now defaults to OS-local Storyfella data in all builds, with
    Change location/Open folder controls and explicit restart feedback. One
    `storyfella.toml` contains application and inference settings; old user
    settings migrate once while preserving originals and existing model files.
    The 2026-10-10 workspace pass adds stable icon navigation, visual art-action
    forms using shared typed operations, contextual palette actions, a separate
    command dock and quieter inspector/project surfaces. It fixes command
    clearing and repeated artwork pickers found during live macOS acceptance.
    The dock is now a compact searchable action entry: contextual forms and
    artwork opening via arrows/Enter, explicit assistant requests, and opt-in
    slash argument completion. Native selects match textboxes; field focus
    uses an inset treatment. Rapid Enter dispatch reads current input.
    Explicit edit queries now explain missing images/selections and offer
    generation for the selected artwork before editing, instead of falling
    through to the assistant. Missing/available-image paths passed live checks.
    Writer rename opens a focused form; Problems starts collapsed with
    navigable, filtered diagnostics. Quit uses styled save/discard controls and
    distinguishes transient action searches from unsent requests. Isolated
    macOS checks verified rename/undo, filtering/jumps, request protection and
    description persistence through Save and quit.
    §9 records changes, regression evidence and remaining cross-platform,
    assistive-technology and failure-injection coverage. — **✅ remediation implemented; broader manual acceptance tracked in DESIGN.md**

## 13. Implementation handoff

Design status: the canvas (roadmap item 4) replaced the legacy storyboard
model — the GUI is the artifact registry; the `svs` CLI shares the same
`ArtifactRegistry`/`ProjectDb` core (`docs/api-slice-1.md`). Work follows
§12 — items 5 (generation quality controls), 6 (LoRA registry), and 7
(error codes + reporting) are implemented; Studio integration (item 8) was
deferred by user reprioritization in favor of generation quality, the LoRA
registry, and UX fixes.

Text and vision are now capability-routed providers: the single `[llm]` section
is gone, replaced by `text` and `vision` capabilities resolved from
`storyfella.toml` (layered with one user settings overlay, so a key stays out of
git) — so a remote, text-only planner no longer blinds `svs eval`. The
`clipforge-inference` runtime is generalized to a shared native-server
supervisor, and a `llamacpp` provider can start a managed `llama-server` on
demand; weights are never bundled. Settings → **Models** gives each chat
capability an editable provider, base URL, model and API key (switching
`[defaults] text`/`vision` in the user overlay, and rebuilding the planner client
in place), so pointing the app at a remote or local server is a click and a field
edit rather than a file edit; the same page adds and removes providers. Settings
→ **Services** carries the image engine and the managed `llama-server` files.

### Entry points (existing code)

- `crates/studio-core/src/registry/` — artifact kinds, variants, scenes/beats, revisions,
  drafts; the typed operation set + pipeline builder; the slash
  parser (`slash.rs`) the canvas composer uses; `CreativeBackend` (live
  backend shared by the CLI, the canvas, and Settings).
- `apps/studio/src/state/mod.rs` — AppState: registry, selection, undo/redo stacks.
- `crates/studio-core/src/persistence/mod.rs` — `ProjectDb` (§10 schema; v3 dropped the legacy
  `project_json` column, v8 dropped the `masks` table).
- The shared `clipforge-inference` crate (`../clipforge/crates/inference`) —
  `sd::Client`, the typed client for the
  native `sd-server` `sdcpp` API; `GenerateRequest` is the wire shape. Also owns
  `sd::Runtime` (the resident process), `sd::qwen_image_2_1()` (the served profile),
  and `ModelStore` (provisioning). storyfella depends on it as a path dependency.
- `crates/studio-core/src/runtime/creative_runtime.rs` — `CreativeRuntime`, the adapter over
  the shared runtime + client (readiness, job wait, asset import) the pipeline
  builder wraps; `health.rs` probes the dependent services.
- `crates/studio-core/src/llm/` — LLM endpoint client; the operation registry is the
  planner business logic it will serve.

### Open decisions to settle first (with the user)

1. Execution model: user-typed ops instant + undo; LLM proposals gated by
   approval — proposed, unconfirmed.
2. Approval granularity: whole stack vs per-step checkpoints (draft-text
   approval is the remaining checkpoint).
3. Short-id format for `c:` references.
4. Native mask support for Krea — **removed** with the Krea/SAM migration;
   there is no mask source and no composite fallback.
5. Deferred with SQLite: crate choice (`rusqlite` vs `sqlx`). —
   **resolved:** `rusqlite` (bundled, WAL, one writer connection).

### Pitfalls

- Qwen-Image-2.1 is the only generation model (native, via
  stable-diffusion.cpp); Krea 2 and the SAM/segmentation path are gone.
- Undo is state restore, never pipeline re-execution.
- VLLM vocabulary is closed; LLM steps are soft dependencies.
- The builder is substrate: linear, fail-fast, no branching/DAG in slice 1.
- Storyfella is built from scratch in this repo; `../storyfella` is reference
  material only.

# Architecture

Smart Visual Sequencer is a desktop application with a deliberate boundary
between product code and machine-learning code:

- **Rust** owns the Dioxus UI, application state, domain models, project
  persistence, model download/provisioning, process lifecycle, directory
  resolution, and the LM Studio client.
- **Python** owns every ML framework and vision implementation detail. Rust
  never imports or knows about PyTorch, Transformers, or SAM.
- **HTTP** is the only boundary between them, and the Python side is a normal
  FastAPI service with stable contracts (`python/models/schemas.py`).
- **stable-diffusion.cpp** (`sd-server`) is a separate native process owned by
  Rust that keeps the Krea 2 diffusion model resident.
- **LM Studio** is an external OpenAI-compatible service accessed directly from
  Rust. It currently ships a client but no planner business logic.

## Component map

```text
┌────────────────────────────── Rust (Dioxus desktop) ──────────────────────────────┐
│                                                                                    │
│  src/ui/          Workspace shell, Canvas (artifact workspace), Settings             │
│  src/state/       AppState: registry, selection, undo/redo stacks                   │
│  src/registry/    Artifact model, typed ops + pipelines, slash parser,               │
│                   CreativeBackend (live backend)                                    │
│  src/persistence/ ProjectDb (SQLite, versioned migrations)                          │
│  src/llm/         LmStudioClient (OpenAI-compatible chat completions)               │
│  src/vision/      VisionClient (typed HTTP client for the Python runtime)           │
│  src/runtime/     CreativeRuntime facade + PythonRuntime + GenerationRuntime +      │
│                   ModelStore (download, resume, checksum verify)                    │
│  src/assets/      AssetCatalog (import/index boundary; currently minimal)           │
│                                                                                    │
│  spawns and supervises:                                                            │
│   ┌─────────────────────┐        ┌──────────────────────────────────────────────┐  │
│   │ python/main.py      │  HTTP  │ sd-server (stable-diffusion.cpp)             │  │
│   │ FastAPI on :8765    │ ─────▶ │ resident Krea 2 + Qwen3-VL + VAE on :7861    │  │
│   └─────────────────────┘        └──────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────────────────┘
  │
  │ OpenAI-compatible HTTP (src/llm/)
  ▼
┌──────────────────────────────┐
│ LM Studio (external, user)   │
└──────────────────────────────┘
```

The native generation server is started with `--listen-ip 127.0.0.1
--listen-port 7861`; the Python runtime listens on `127.0.0.1:8765`
(see [`runtime-lifecycle.md`](runtime-lifecycle.md)). Both are loopback-only.

## The prompt-to-revision flow

The Canvas is the primary workspace. A slash command in the composer parses
into a typed operation (`src/registry/slash.rs`); model-only operations
(`compose`, `create` of text-only kinds) apply instantly, while visual
`create`/`variant` (first image), `regenerate`, and `modify` run their
pipeline against `CreativeBackend` in a spawned task.

```text
Composer: /create character Mia…  /variant c:mia …  /regenerate c:mia …
    │  slash::parse_slash → Operation (compile validates against the registry)
    ▼
ops::execute (src/registry/ops.rs)
    ├─ Direct (compose / text-only create): registry mutation + log entry
    ├─ Create/Variant (visual): artifact + first-image pipeline (variants
    │    reference the base's active image)
    └─ Pipeline (regenerate/modify):
         │  AppState::snapshot_for_undo (registry snapshot — undo basis)
         ▼
       CreativeBackend::generate (src/registry/backend.rs)
         │  CreativeRuntime::ensure_profile_ready (cold start: sd-server + python)
         │  VisionClient::submit_generation(POST /generation/jobs)
         ▼
       sd.cpp generates into python's SVS_GENERATED_DIR
         │  CreativeRuntime::wait_for_job polls GET /generation/jobs/{id}
         ▼
       completed job exposes image_path
         │  CreativeRuntime::import_asset copies into <asset_dir>/generated/<uuid>.png
         ▼
       registry.finish_revision → completed revision + active image
         │  has_unsaved_changes → autosave writes the SQLite snapshot
         ▼
Artifact detail: image + revision history (served via /generated/<file>)
```

A `modify` is an **instruction edit**, not a regional one: `LoadImage →
Ground(instruction) → Generate(reference, grounded instruction)`. Krea 2's
identity-edit adapter takes an image plus a plain-language instruction and
preserves what the instruction did not mention, so nothing is segmented,
masked or composited, and the op runs unattended — there is no checkpoint.

The `Ground` step is why it holds together. A terse instruction
under-specifies the render and it collapses toward base-model priors, so the
instruction is rewritten to name the medium and re-state the artifact's own
description as the traits to keep. `ground_instruction` (`registry/ops.rs`)
does that deterministically; when LM Studio is configured, `llm_ground`
refines it by dropping the traits the edit overrides. The LLM is a soft
dependency — it never blocks, and a missing one falls back to the template.

**Every op that sends a delta grounds it**, not just `modify`: a variant
description ("in heavy yellow oilskins") and an explicit regenerate prompt
("make it warmer") are the same shape of terse instruction and drift the same
way — an ungrounded variant of a woman in her forties returned a different
person of a different sex. A variant grounds against its *base* artifact; a
regenerate against its own. A regenerate with no prompt is the exception: it
replays the previous revision's prompt, which was grounded when it was made.

Because the anchor is the *artifact* description and not the current
revision, edits do not accumulate: `dye her hair auburn` then `give her a bob
cut` grounds the second edit against the original description, and the auburn
washes out. That is deliberate — an artifact has one canonical look and edits
are experiments against it — so `describe` is the op that promotes an edit to
canonical (`/describe c:mara "…, auburn hair, …"`), and `variant` is how a
look becomes its own artifact. Anchoring on the current revision instead would
need a caption of the active image; `/caption` is a placeholder today.

Passing both `--mask-prompt` and `--inpaint-prompt` opts into the older
regional flow instead (`Segment → Checkpoint(confirm mask) → Generate →
Composite`, see `docs/ROADMAP.md` §5). That path still checkpoints — the
canvas overlays the segmentation candidates in a modal, the CLI offers
`--approve interactive`, and non-interactive runs auto-pick the best
candidate — and it is retained for inpaint/outpaint work that an instruction
cannot express. It does not currently produce correct results; see
`AGENTS.md`.

The interactive draft profile requests 768×448 at four steps to favor
iteration speed on slower hardware.

## Process boundaries

| Process | Owner | Lifetime | Purpose |
| --- | --- | --- | --- |
| Dioxus desktop app | Rust | Application session | UI, state, orchestration |
| `python/main.py` (uvicorn) | Rust (`PythonRuntime`) | On demand, resident after first generation | HTTP adapter, result store, segmentation, Diffusers POC |
| `sd-server` | Rust (`GenerationRuntime`) | On demand, resident | Persistent Krea 2 inference |
| LM Studio | User (external) | External | LLM planning/VLM (future) |

Rust does not start the Python runtime at app launch; the shell reports the
runtime as idle until a generation request triggers `ensure_ready` — or until
it is started manually from the Settings Status section
(see [`runtime-lifecycle.md`](runtime-lifecycle.md#on-demand-start-and-stop)).

## Directory resolution

Runtime, model, cache, and asset paths resolve through the `directories` crate
(`ProjectDirs::from("dev", "Smart Visual Sequencer", "Smart Visual Sequencer")`)
and can be overridden in [`config/app.toml`](../config/app.toml). The checked-in
config points everything at the repository (`models/`, `cache/`, `assets/`) for
development.

## What is deliberately absent

- **The planner lives in `src/registry/`.** The typed operation set, the
  pipeline layer, the slash parser, and the `svs` CLI (see
  `docs/api-slice-1.md`) are implemented; `src/llm/` is the client only,
  driven through `CreativeBackend`. The Canvas free-form composer messages
  are proposed as operation stacks (LLM → closed-vocabulary JSON → approval
  modal → executed through the same worker machinery as slash commands).
- **No ComfyUI.** Image generation is native (`stable-diffusion.cpp`) or a
  Diffusers POC behind `/generate`; ComfyUI is not a dependency.
- **No web/mobile targets.** The app is desktop-only; `cargo` features are
  limited to `desktop`.
- **Captioning is a placeholder.** `POST /caption` returns
  `status = "not_implemented"` until a captioning model is selected.

See the [README roadmap](../README.md#roadmap) for the ordered backlog.

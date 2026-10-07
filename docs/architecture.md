# Architecture

## Storyfella workspace boundaries

Studio, Studio core, CLI, the visual Player, xtask, shared playback UI,
language, compiler, runtime and stdio LSP are workspace members. The Writer
and LSP consume the same analysis. See [the language contract](language.md)
and [execution plan](language-plan.md) for implemented scope and next milestones.

| Component | Responsibility | Dependency boundary |
| --- | --- | --- |
| `apps/studio` / `storyfella-studio` | Dioxus authoring UI, project state, creative workflows and snapshot preview | Consumes GUI-free core and shared playback UI/runtime |
| `crates/studio-core` / `storyfella-studio-core` | Artifact registry, operations, persistence, config, creative inference adapters | No GUI dependencies; consumes sibling `clipforge-inference` |
| `apps/cli` / `svs-cli` | Existing `svs`, `model_setup`, and `vlm_probe` tools | Keeps current commands and behavior; consumes Studio core |
| `apps/player` / `storyfella-player` | Standalone visual Dioxus desktop playback | Shared core resource adapter, playback UI and runtime; no Studio state dependency |
| `crates/playback-ui` / `storyfella-playback-ui` | Shared visual presentation and controls | Consumes GUI-free host adapter and runtime; no dependency on either app |
| `xtask` | Build, launch, package and validation commands | Developer tooling only |

Language, compiler, LSP and story runtime are implemented below. No placeholder
crates are added; [runtime.md](runtime.md) records the execution contract.

| Language/execution crate | Responsibility | Dependency boundary |
| --- | --- | --- |
| `crates/language` / `storyfella-language` | Source model, lexer/parser, spans, diagnostics, symbols, references and shared language analysis | Independent of Studio, UI frameworks and inference backends |
| `crates/compiler` / `storyfella-compiler` | Validate/lower analyzed stories into a shared executable story representation | May consume language analysis; independent of Studio, UI frameworks and inference backends |
| `crates/runtime` / `storyfella-runtime` | Story execution, state transitions and timing; host-facing playback events | Shared by Studio preview and Player; no dependency on either application or an inference backend |
| `crates/lsp` / `storyfella-lsp` | LSP transport and editor-facing diagnostics, completion, navigation and other language services | Exposes shared language analysis to external editors; does not duplicate analysis or depend on Studio |

The compiler emits a versioned, source-mapped story plan with sequential scene
fallthrough and explicit terminal instructions; see [compiler.md](compiler.md).
The runtime consumes that representation with bounded host-driven execution.
Existing Studio core `runtime/` supervises creative generation through Clipforge;
it is distinct from the story execution runtime. FineWineTTS and Clipforge
remain separate repositories. The inference path dependency stays
`../clipforge/crates/inference` after the repository-directory rename.

`svs`, `model_setup`, and `vlm_probe` names, `SVS_*` settings, error codes,
SQLite formats and original platform config/data identities are retained to
preserve existing workflows. The directory move preserves local files, but
databases containing absolute paths into the old checkout can still reference
the old location; those paths are not silently rewritten by this restructuring.

## Existing Studio architecture

Storyfella Studio is a desktop application whose entire product code —
UI, state, domain models, persistence, provisioning, process lifecycle, and
path resolution — is Rust. There is no Python: the old FastAPI vision runtime
and its SAM/segmentation stack were removed with the Krea/SAM migration, and
the project's only generation backend is a native process Rust talks to over
HTTP.

- **Rust** owns the Dioxus UI, application state, domain models, project
  persistence, process lifecycle, directory resolution, the native generation
  runtime (the `CreativeRuntime` adapter over the shared `clipforge-inference`
  crate), and the LLM client.
- **`stable-diffusion.cpp`** (`sd-server`) is a separate native process owned by
  Rust that keeps the **Qwen-Image-2.1** diffusion model resident. Rust never
  links an ML framework; it is only HTTP client to the server's `sdcpp` API.
- **The LLM endpoints** (text and vision) are OpenAI-compatible services
  accessed directly from Rust and resolved from `providers.toml` as separate
  capabilities. An `openai` provider is external and never started; a
  `llamacpp` provider is a managed `llama-server` the app starts on demand.
  `crates/studio-core/src/llm/` is the client; the planner business logic lives in
  `crates/studio-core/src/registry/`.

## Component map

```text
┌────────────────────────────── Rust (Dioxus desktop) ──────────────────────────────┐
│                                                                                    │
│  crates/studio-core/src/  — the domain layer (no GUI dependency)                          │
│    registry/      Artifact model, typed ops + pipelines, slash parser,             │
│                   CreativeBackend (live backend)                                   │
│    persistence/   ProjectDb (SQLite, versioned migrations)                         │
│    story.rs       Exact .sf documents, disk snapshots and source-file saves       │
│    llm/           LlmClient (OpenAI-compatible chat completions)                   │
│    runtime/       CreativeRuntime adapter over the shared clipforge-inference      │
│                   crate (asset import, job wait, health, error mapping)            │
│                                                                                    │
│  apps/studio/src/  — the Dioxus application                                       │
│    ui/            Workspace shell, Canvas (artifact workspace), Settings           │
│    state/         AppState: registry, selection, undo/redo stacks                  │
│                                                                                    │
│  spawns and supervises:                                                            │
│   ┌──────────────────────────────────────────────────────────────────────────────┐ │
│   │ sd-server (stable-diffusion.cpp)                                             │ │
│   │ resident Qwen-Image-2.1 + Qwen3-VL encoder + VAE on 127.0.0.1:7861           │ │
│   └──────────────────────────────────────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────────────────────────────────────┘
  │
  │ OpenAI-compatible HTTP (crates/studio-core/src/llm/)
  ▼
┌───────────────────────────────┐
│ text/vision endpoints (LLM)   │
└───────────────────────────────┘
```

The generation server is started with `--listen-ip 127.0.0.1 --listen-port 7861`
and serves only on loopback; the image provider's `base_url` in
`providers.toml` names both the address the client calls and the port the
runtime binds. See [`runtime-lifecycle.md`](runtime-lifecycle.md).

### The shared inference runtime

The `sd-server` profile, the resident process, the `sdcpp` client, and weight
provisioning do **not** live in this workspace: they are in the sibling
`clipforge` project's crate, package **`clipforge-inference`**, pulled in as a
**path dependency** (`clipforge-inference = { path = "../clipforge/crates/inference" }`
in the root `[workspace.dependencies]`). storyfella keeps only the adapter —
`crates/studio-core/src/runtime/` (`CreativeRuntime`), `config.rs`, and the pipeline's
`GenerateResponse` (`crates/studio-core/src/registry/pipeline.rs`). The sibling checkout
at `../clipforge/crates/inference` must be present for storyfella to build;
the crate is not a workspace member.

## The `sd-server` (`sdcpp`) HTTP contract

Rust consumes the native server's async `sdcpp` API through the shared crate's
client, `clipforge_inference::sd::Client` (called via `CreativeRuntime::client`).
The authoritative field list is `examples/server/api.md` at the pinned upstream
revision (`leejet/stable-diffusion.cpp` @ `3f8527a`, the revision the `clipforge`
workspace builds `sd-server` from) — read it before adding a field, because
anything the server does not name in its JSON parser is dropped silently.

### Endpoints

| Method | Path | Purpose |
| --- | --- | --- |
| `POST` | `/sdcpp/v1/img_gen` | Submit an image-generation job (async) |
| `GET` | `/sdcpp/v1/jobs/{id}` | Poll a submitted job |
| `GET` | `/sdcpp/v1/capabilities` | Report the resident model |

The base URL is the image provider's `base_url` in `providers.toml`
(`http://127.0.0.1:7861` in the checked-in file).

### Request body (`POST /sdcpp/v1/img_gen`)

The client maps `GenerateRequest` (`clipforge_inference::sd::GenerateRequest`)
onto this body. Fields the app does not vary keep the server defaults.

| Field | Type | Value sent |
| --- | --- | --- |
| `prompt` | string | the (grounded) instruction or description |
| `width`, `height` | int | multiples of 32 within 256–2048 |
| `seed` | int | the requested seed, or `-1` for a random one |
| `batch_count` | int | `1` |
| `ref_images` | array of base64 strings | `[reference]` for an edit; the field is omitted when there is no reference |
| `sample_params.scheduler` | string | `discrete` (pinned explicitly — the server's compatibility default is unnamed) |
| `sample_params.sample_method` | string | `euler` by default |
| `sample_params.sample_steps` | int | the step count (default 25) |
| `sample_params.guidance.txt_cfg` | float | the text-guidance scale (default 6.0) |
| `lora` | array of `{path, multiplier}` | the adapters to apply; omitted when empty |
| `vae_tiling_params` | object | always sent, see below |
| `output_format` | string | `png` |

`vae_tiling_params` is sent with `enabled: true`, `tile_size_w`/`tile_size_h`
256, and `target_overlap` 0.5. This build ignores tiling when only `enabled` is
set, and the untiled decode buffers OOM on a 24 GiB card at a 1024×1536 canvas,
so the explicit tile size is what keeps tiled decoding the only path. `lora[].path`
is the name the server registers an adapter under (`LoraBindings::registered`),
never an absolute path.

### Response shapes

Submission (`202 Accepted`):

```json
{ "id": "job-uuid", "kind": "img_gen", "status": "queued", "created": 0, "poll_url": null }
```

Polled job (`GET /sdcpp/v1/jobs/{id}`):

```json
{
  "id": "job-uuid", "kind": "img_gen", "status": "completed", "created": 0,
  "started": 1, "completed": 2, "queue_position": 0,
  "result": { "output_format": "png", "images": [ { "index": 0, "b64_json": "…" } ] },
  "error": null
}
```

`status` is one of `queued`, `generating`, `completed`, `failed`, `cancelled`.
A failed job carries `error: { code, message }`. `sd::Job::first_image`
base64-decodes `result.images[0].b64_json`.

Capabilities (`GET /sdcpp/v1/capabilities`) is a large document; only the
resident-model block is consumed — `{ "model": { "name", "stem", "path" }, … }`.
Readiness compares the reported `name`/`stem` against the profile's diffusion
file name (either form is accepted).

## The prompt-to-revision flow

The Canvas is the primary workspace. Commands reach it from four places that
share one vocabulary: the `svs` CLI, the tokenized command bar, the `Ctrl+K`
palette, and the inspector rail — every control in the rail emits the command
a person would have typed, so the two cannot diverge. All of them parse
through `crates/studio-core/src/registry/slash.rs`; model-only operations
(`compose`, `create` of text-only kinds) apply instantly, while visual
`create`/`variant` (first image), `regenerate`, and `modify` run their
pipeline against `CreativeBackend` in a spawned task.

```text
Composer: /create character Mia…  /variant c:mia …  /regenerate c:mia …
    │  slash::parse_slash → Operation (compile validates against the registry)
    ▼
ops::execute (crates/studio-core/src/registry/ops.rs)
    ├─ Direct (compose / text-only create): registry mutation + log entry
    ├─ Create/Variant (visual): artifact + first-image pipeline (variants
    │    reference the base's active image)
    └─ Pipeline (regenerate/modify):
         │  AppState::snapshot_for_undo (registry snapshot — undo basis)
         ▼
       CreativeBackend::generate (crates/studio-core/src/registry/backend.rs)
         │  CreativeRuntime::ensure_profile_ready (cold start: sd-server)
         │  sd::Client::generate (POST /sdcpp/v1/img_gen)
         ▼
       CreativeRuntime::wait_for_job → sd::Client::wait polls GET /sdcpp/v1/jobs/{id},
       then decodes result.images[0].b64_json and writes
       <asset_dir>/generated/<uuid>.png (the canonical asset path)
         ▼
       registry.finish_revision → completed revision + active image
         │  has_unsaved_changes → autosave writes .sf and the SQLite registry snapshot
         ▼
Artifact detail: image + revision history (served via /generated/<file>)
```

A `modify` is an **instruction edit**, not a regional one: `LoadImage →
Generate(reference, grounded instruction)`. The model takes an image plus a
plain-language instruction and preserves what the instruction did not mention,
so nothing is segmented, masked or composited, and the op runs unattended —
there is no checkpoint.

Grounding is why it holds together. A terse instruction under-specifies the
render and it collapses toward base-model priors, so the instruction is
rewritten to name the medium and re-state the artifact's own description as the
traits to keep. `ground_instruction` (`crates/studio-core/src/registry/ops.rs`)
does that deterministically; when the LLM endpoint is configured, `llm_ground`
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
look becomes its own artifact.

## Process boundaries

| Process | Owner | Lifetime | Purpose |
| --- | --- | --- | --- |
| Dioxus desktop app | Rust | Application session | UI, state, orchestration |
| `sd-server` | Rust (shared `clipforge-inference` `sd::Runtime`) | On demand, resident | Persistent Qwen-Image-2.1 inference |
| text/vision endpoint | openai (external) or `llama-server` (managed) | External, or on demand | LLM planning/grounding and VLM evaluation |

Rust does not start `sd-server` at app launch; the shell reports it as idle
until a generation request triggers `ensure_profile_ready` — or until it is
started manually from the Settings Status section (see
[`runtime-lifecycle.md`](runtime-lifecycle.md)).

Only one model can be resident: Qwen-Image-2.1 peaks at ≈23.4 GiB on a 24 GiB
card, so the planner/LLM cannot run concurrently with a generation. Free VRAM
— stop the LLM model — before a generation on this hardware.

## Directory resolution

Runtime, model, cache, and asset paths resolve against the **project root**
(`crates/studio-core/src/paths.rs`): `$SVS_HOME` if set, otherwise the nearest
ancestor of the working directory (then of the executable) holding
`config/app.toml`. Relative paths in [`config/app.toml`](../config/app.toml)
resolve against that root, and unset values fall back to platform-appropriate
application directories via the `directories` crate
(`ProjectDirs::from("dev", "Smart Visual Sequencer", "Smart Visual Sequencer")`).
The config itself is layered: a user-level `app.toml` in that same platform
config directory (`paths::user_config_path()`) is deep-merged over the root file
key by key, so machine-local secrets and overrides stay out of the repository.
The image deployment — the `sd-server` executable, model tree, LoRA root,
endpoint, and weights — is resolved from [`providers.toml`](../providers.toml) at
that same root (also via `paths::providers_path()`); the deployment's
`AppConfig::image` is what the runtime, the health probe, `model_setup`, and
Settings read. The checked-in config points `cache/` and `assets/` at the
repository for development, while `providers.toml` roots the model tree at
`models/`.

## What is deliberately absent

- **The planner lives in `crates/studio-core/src/registry/`.** The typed operation set, the
  pipeline layer, the slash parser, and the `svs` CLI (see
  `docs/api-slice-1.md`) are implemented; `crates/studio-core/src/llm/` is the client only,
  driven through `CreativeBackend`. The Canvas free-form composer messages
  are proposed as operation stacks (LLM → closed-vocabulary JSON → approval
  modal → executed through the same worker machinery as slash commands).
- **No Python and no ML framework.** There is no FastAPI vision runtime, no
  segmentation, and no Diffusers pipeline. Generation is the native
  `stable-diffusion.cpp` process; the text/vision endpoints are separate
  providers (external, or a managed `llama-server`).
- **No ComfyUI.** Image generation is the native `sd-server`; ComfyUI is not a
  dependency.
- **No web/mobile targets.** The app is desktop-only; `cargo` features are
  limited to `desktop`.

See the [README roadmap](../README.md#roadmap) for the ordered backlog.

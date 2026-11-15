# Architecture

## Storyfella workspace boundaries

Studio, Studio core, CLI, the visual Player, xtask, shared playback UI,
language, compiler, runtime and stdio LSP are workspace members. The Writer
and LSP consume the same analysis. See [the language contract](language.md)
and [central tracker](ROADMAP.md#0-current-direction-and-actionable-backlog) for implemented scope and next milestones.

| Component | Responsibility | Dependency boundary |
| --- | --- | --- |
| `apps/studio` / `storyfella-studio` | Dioxus authoring UI, project state, creative workflows and snapshot preview | Consumes GUI-free core and shared playback UI/runtime |
| `apps/atelier` / `storyfella-atelier` | Independent GPUI drawing and image-generation app | Owns sketchbook documents, credentials and ComfyUI workflow; uses Agentkit's hosted image client; no Dioxus dependency |
| `crates/studio-core` / `storyfella-studio-core` | Artifact registry, operations, persistence, config, creative inference adapters | No GUI dependencies; consumes sibling `agentkit-ai` |
| `apps/cli` / `svs-cli` | Existing `svs`, `model_setup`, and `vlm_probe` tools | Keeps current commands and behavior; consumes Studio core |
| `apps/player` / `storyfella-player` | Standalone visual Dioxus desktop playback | Shared core resource adapter, playback UI and runtime; no Studio state dependency |
| `crates/playback-ui` / `storyfella-playback-ui` | Shared visual presentation and controls | Consumes GUI-free host adapter and runtime; no dependency on either app |
| `xtask` | Build, launch, package and validation commands | Developer tooling only |

Language, compiler, LSP and story runtime are implemented below. No placeholder
crates are added; [playback reference](ROADMAP.md#story-execution-and-playback-reference) records the execution contract.

| Language/execution crate | Responsibility | Dependency boundary |
| --- | --- | --- |
| `crates/language` / `storyfella-language` | Source model, lexer/parser, spans, diagnostics, symbols, references and shared language analysis | Independent of Studio, UI frameworks and inference backends |
| `crates/compiler` / `storyfella-compiler` | Validate/lower analyzed stories into a shared executable story representation | May consume language analysis; independent of Studio, UI frameworks and inference backends |
| `crates/runtime` / `storyfella-runtime` | Story execution, state transitions and timing; host-facing playback events | Shared by Studio preview and Player; no dependency on either application or an inference backend |
| `crates/lsp` / `storyfella-lsp` | LSP transport and editor-facing diagnostics, completion, navigation and other language services | Exposes shared language analysis to external editors; does not duplicate analysis or depend on Studio |

The compiler emits a versioned, source-mapped story plan with sequential scene
fallthrough and explicit terminal instructions; see [compiler.md](compiler.md).
The runtime consumes that representation with bounded host-driven execution.
Existing Studio core `runtime/` drives the shared native-server owner through
Agentkit; it is distinct from the story execution runtime. FineWineTTS and
Clipforge remain separate applications. Inference depends directly on
`../../zerune_labs/agentkit/crates/agentkit-ai`.

`svs`, `model_setup`, and `vlm_probe` names, `SVS_*` settings, error codes,
SQLite formats and original platform config/data identities are retained to
preserve existing workflows. The directory move preserves local files, but
databases containing absolute paths into the old checkout can still reference
the old location; those paths are not silently rewritten by this restructuring.

## Existing Studio architecture

Atelier lives alongside Studio as a separate GPUI application. Its hosted
generation/editing uses `agentkit_ai::images::Client`; ComfyUI workflows
remain app-local. [Atelier's integration](atelier.md) records the shared
AI boundary and the remaining image-workspace work. The Studio architecture
below describes the Dioxus application.

Storyfella Studio is a desktop application whose entire product code —
UI, state, domain models, persistence, provisioning, process lifecycle, and
path resolution — is Rust. There is no Python: the old FastAPI vision runtime
and its SAM/segmentation stack were removed with the Krea/SAM migration, and
the project's only generation backend is a native process Rust talks to over
HTTP.

- **Rust** owns the Dioxus UI, application state, domain models, project
  persistence, process lifecycle, directory resolution, the native generation
  runtime (the `CreativeRuntime` adapter over the shared `agentkit-ai`
  crate), and the LLM client.
- **`stable-diffusion.cpp`** (`sd-server`) is a separate native process owned by
  Rust that keeps the **Qwen-Image-2.1** diffusion model resident. Rust never
  links an ML framework; it is only HTTP client to the server's `sdcpp` API.
- **The LLM endpoints** (text and vision) are OpenAI-compatible services
  accessed directly from Rust and resolved from the shared `clipforge.toml` as
  separate capabilities. An `openai` provider is external and never started; a
  `llamacpp` provider is a managed `llama-server` the app starts on demand and
  reaches through the worker address the shared inference service allocates.
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
│    runtime/       CreativeRuntime adapter over the shared agentkit-ai      │
│                   crate (asset import, job wait, health, error mapping)            │
│                                                                                    │
│  apps/studio/src/  — the Dioxus application                                       │
│    ui/            Workspace shell, Assets (artifact workspace), Settings           │
│    state/         AppState: registry, selection, undo/redo stacks                  │
│                                                                                    │
│  drives the shared owner:                                                         │
│   ┌──────────────────────────────────────────────────────────────────────────────┐ │
│   │ sd-server (stable-diffusion.cpp)                                             │ │
│   │ resident Qwen-Image-2.1 + Qwen3-VL encoder + VAE on 127.0.0.1:7861           │ │
│   │ started detached by agentkit_ai::native::control over one shared state dir    │ │
│   └──────────────────────────────────────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────────────────────────────────────┘
  │
  │ OpenAI-compatible HTTP (crates/studio-core/src/llm/)
  ▼
┌───────────────────────────────┐
│ text/vision endpoints (LLM)   │
└───────────────────────────────┘
```

Studio, Atelier and Clipforge share one local inference service. It queues
text, vision, generation and editing work and assigns loopback ports to its
native workers. Hosted requests use the provider endpoints in Clipforge's
shared registry.

### The shared inference runtime

Native profiles, weight provisioning, HTTP clients and the inference service live in `agentkit_ai::native`. Provider resolution lives in
`agentkit_ai::deployments`, which accepts an application-supplied TOML path.
The workspace consumes `../../zerune_labs/agentkit/crates/agentkit-ai` with
`native-inference` enabled. No Clipforge crate or installed app is required.

`agentkit_ai::images` provides native/hosted execution, an `ImageModel` trait
and per-route `ImageCapabilities`. Hosted-only consumers enable `images`
without native support. `ProviderFactory::build_image_model` accepts the same
provider configuration and credential resolver as chat. Studio's planner and
text/vision helpers delegate to the existing Agentkit OpenAI-compatible provider.

Storyfella owns config/state locations, source/artwork persistence and the
`CreativeRuntime`/LLM adapters. The existing `clipforge.toml` keys, credentials,
model files and shared native-server state locations retain their meanings.

## The `sd-server` (`sdcpp`) HTTP contract

Rust consumes the native server's async `sdcpp` API through the shared crate's
client, `agentkit_ai::native::sd::Client` (called via `CreativeRuntime::client`).
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

The base URL is the image provider's `base_url` in the shared
`clipforge.toml` (`http://127.0.0.1:7861` in the default).

### Request body (`POST /sdcpp/v1/img_gen`)

The client maps `GenerateRequest` (`agentkit_ai::native::sd::GenerateRequest`)
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

Studio's Assets browses definitions, saved images and read-only revision history.
Definitions and reference-aware rename use `AppState::edit_catalog`; description
drafts use its guarded save/undo path. Open in Atelier saves the authoring state
and delivers the shared project/item identity. Atelier's ordinary linked Save
publishes immutable image revisions and Studio refreshes them. There is no
Studio image composer, generation inspector, revision activation, rerun or
`.sfart` import UI. The palette exposes authoring/navigation actions only.
Studio guide commands are restricted to descriptions and text-only creation,
with batch preflight before changes and no inference backend.

The independent `svs` CLI retains visual operations and the planner. Its Clap
arguments build typed operations; the shared slash parser provides the script
syntax used by Studio's restricted guides. Model-only operations
(`compose`, `create` of text-only kinds) apply instantly, while visual
`create`/`variant` (first image), `regenerate`, and `modify` run their
pipeline against `CreativeBackend`.

```text
svs op: create character Mia… / variant c:mia … / regenerate c:mia …
    │  CLI arguments → Operation (compile validates against the registry)
    ▼
ops::execute (crates/studio-core/src/registry/ops.rs)
    ├─ Direct (compose / text-only create): registry mutation + log entry
    ├─ Create/Variant (visual): artifact + first-image pipeline (variants
    │    reference the base's active image)
    └─ Pipeline (regenerate/modify):
       CreativeBackend::generate (crates/studio-core/src/registry/backend.rs)
         │  CreativeRuntime::ensure_profile_ready (cold start: sd-server)
         │  sd::Client::generate (POST /sdcpp/v1/img_gen)
         ▼
       CreativeRuntime::wait_for_job → sd::Client::wait polls GET /sdcpp/v1/jobs/{id},
       then decodes result.images[0].b64_json and writes
       <asset_dir>/generated/<uuid>.png (the canonical asset path)
         ▼
       registry.finish_revision → completed revision + active image
         │  CLI persistence writes the registry and immutable resources
         ▼
Studio: read-only saved image + revision history (served via /generated/<file>)
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
| `sd-server` | Rust (`agentkit-ai::native::sd::Runtime`, shared Agentkit controls) | On demand, resident | Persistent Qwen-Image-2.1 inference |
| text/vision endpoint | openai (external) or `llama-server` (managed) | External, or on demand | LLM planning/grounding and VLM evaluation |

Rust does not start `sd-server` at app launch; the shell reports it as idle
until a generation request triggers `ensure_profile_ready` — or until it is
started manually from the Settings **Services** page (see the shared native server lifecycle below).

Only one model can be resident: Qwen-Image-2.1 peaks at ≈23.4 GiB on a 24 GiB
card, so the planner/LLM cannot run concurrently with a generation. Free VRAM
— stop the LLM model — before a generation on this hardware.

## Directory resolution

Configuration is split by concern. **App config** is the platform-local
`studio.toml` in the platform application config directory
(`paths::config_path()`), created on first load when absent. It is not a
repository file. The **project root** that anchors explicit relative paths is
found separately (`crates/studio-core/src/paths.rs`): `$SVS_HOME` if set,
otherwise the nearest ancestor of the working directory (then of the executable)
holding the `config/plans` directory. Explicit relative runtime, model, cache, and
asset paths resolve against that root, and unset values fall back to
platform-appropriate application directories via the `directories` crate
(`ProjectDirs::from_path("dev.storyfella.studio".into())`, deliberately using
the bundle ID as a flat namespace across platforms):
models under the OS-local data dir, assets and project databases under the
platform data dir, and cache under the platform cache dir.
`identity.rs` performs a one-time copy from the former application directories
before default config loading, retaining originals and refusing to overwrite
existing destinations. SQLite backups and structured path updates preserve
projects, artwork, recent links and writing-session recovery. See
[application configuration migration](development.md#app-config).

**AI config** is the machine's shared `clipforge.toml`
(`paths::clipforge_config_path()`, via
`storyfella_studio_core::ai_config::default_config_path()`), shared with Atelier and
Clipforge. Clipforge manages provider entries; Studio reads them and persists its
own selections separately. A missing or empty registry offers no providers.
Relative paths in the shared file resolve against its own directory. The **shared native-server state
directory** (`paths::state_dir()`, via
`storyfella_studio_core::ai_config::default_state_dir()`, `$SVS_STATE_DIR` override)
holds the process markers and detached-server logs every front end shares, so a
server one app started is reused rather than duplicated. The image deployment —
the `sd-server` executable, model tree, LoRA root, endpoint, and weights — is
resolved from the AI config; the optional `AppConfig::image` is what the
runtime, the health probe, `model_setup`, and Settings read. Model storage
is configured in Clipforge; an omitted local image model directory resolves to
`models` under the shared config directory. A hosted or absent t2i default
leaves the native deployment unset: Writer, playback and text-provider settings
remain available. Studio never prewarms image services on project open;
native-only callers report
SVS-CFG-110 without starting servers or downloading weights. Shared hosted
defaults remain unchanged for Atelier. See
[model storage](development.md#inference-settings).

## What is deliberately absent

- **The planner lives in `crates/studio-core/src/registry/`.** The typed operation set, the
  pipeline layer, the slash parser, and the `svs` CLI (see
  `docs/development.md`) are implemented; `crates/studio-core/src/llm/` is the client only,
  driven through `CreativeBackend`. Visual operation stacks and free-form
  planning remain CLI capabilities; Studio's Writer assistant uses the
  separate writing workflow and reviewed source application.
- **No Python and no ML framework.** There is no FastAPI vision runtime, no
  segmentation, and no Diffusers pipeline. Generation is the native
  `stable-diffusion.cpp` process; the text/vision endpoints are separate
  providers (external, or a managed `llama-server`).
- **No ComfyUI.** Image generation is the native `sd-server`; ComfyUI is not a
  dependency.
- **No web/mobile targets.** The app is desktop-only; `cargo` features are
  limited to `desktop`.

See the [README roadmap](../README.md#roadmap) for the ordered backlog.

## Shared local inference service

Agentkit owns admission, model loading, worker supervision and inference HTTP
contracts. Storyfella supplies configuration and state paths. Idle workers can
be offloaded as memory fills, and active requests hold a lease on their model.
A worker's temporary port is independent of the configured provider address.

Clipforge manages providers and the resource policy. Studio's Models page
exposes its selections and health. The CLI's runtime commands can load or
offload the selected image model. `CreativeRuntime` uses a service lease
while an image job runs and handles generated asset storage. `ModelStore`
provisions checksum-verified artifacts and respects explicit weight overrides.
The provider profile defines residency mode and VAE tiling.

`paths::state_dir` and `paths::clipforge_config_path` resolve shared service
state and provider configuration. Plan runs isolate these paths under
`.svs-e2e/`. See [development](development.md).

## Registry operations and pipelines

The `svs` CLI consumes typed operations from
`crates/studio-core/src/registry/ops.rs`. The slash parser retains that vocabulary
for scripts; its former editor-completion module has no callers and is removed.
Studio creates definitions through the shared catalog API and applies authoring
mutations through AppState. Guides accept only metadata operations.

Visual create/variant operations generate their first image. Variants reference
the base image. Modify loads the current image and applies a grounded instruction;
its core pipeline has no mask-compositing stage. Describe and revert
change registry state without generation. Draft can propose text or accept manual
text. Compose records beat artwork relationships rather than editing authored
scene/beat order. Writer remains the structure editor.

The pipeline builder validates parameter bounds and handle ordering. Execution is
linear and fail-fast, retaining intermediates and recording failed/cancelled
revisions. LLM grounding can fall back to a deterministic instruction; draft
steps can fall back to manual input. Undo restores source/registry state rather
than rerunning inference. The roadmap replaces Studio's visual creation tools and
introduces Agentkit workflows; these are future changes, not current behavior.

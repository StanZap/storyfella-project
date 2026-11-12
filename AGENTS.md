You are working on **Storyfella**, the workspace for story authoring and playback. **Storyfella Studio** is the desktop-only Dioxus 0.7 application for writing, arranging and previewing visual stories. **Atelier** owns visual creation and editing. Rust owns the UI, state, models, persistence, process lifecycle and inference adapters. Native image generation uses `stable-diffusion.cpp` through Agentkit's `sdcpp` HTTP client. There is no Python in this project.

Current shape: the artifact registry + typed operation layer (`crates/studio-core/src/registry/`), the SQLite project store, the native generation runtime (`crates/studio-core/src/runtime/`, adapting the shared `agentkit-ai` crate; its image deployment is resolved from the shared `clipforge.toml`), and the canvas are implemented and driven by the `svs` CLI and the desktop app (see `# Current status` below). Read `docs/ROADMAP.md` §12 (the feature/status tracker) before planning changes.

Shared user projects now use `studio-core::project::ProjectFolder`:
`storyfella.toml`, `assets/story.sf` and `story.db`, portable immutable `artwork/` revisions
and `atelier/` working documents. Version-1 projects retain their root source
files and relative includes. Studio creates new folders and explicitly
adopts standalone pairs; Atelier opens the manifest. `studio-core::catalog`
reuses registry UUIDs for definitions, variations and independent illustrations.
GUI registry saves use the loaded `StoredProject::registry_version`; preserve
that optimistic check rather than replacing it with an unconditional snapshot.
Source-used catalog names are renamed in Studio with their resolved references.
See `docs/shared-project.md` and SF-PLAN-01–04. Both apps refresh and merge shared
catalog changes; overlapping fields require a decision. Studio owns existing
names, source and authoring associations; Atelier owns editable documents.
Studio saves source/catalog pairs through `save_authoring_checked`, whose v12
source intent recovers interrupted publication without overwriting unrelated
source text. Preserve these checks and rebase undo on external catalog updates.
Contextual opening uses `studio-core::atelier`: a validated, versioned local
inbox, per-Atelier-data-directory OS instance lease and completion replies.
Studio's shared OpenInAtelier action saves before delivery. Atelier reuses tabs
and linked artboards, retains prompt drafts and embeds existing artwork only on
first attachment; story context stays separate from generated artwork. Ordinary
Save/Save-and-close publishes accepted linked artboards through `shared_save` and
`catalog::save_canvases`, preserving immutable revisions and checking each
artboard's displayed image baseline. Recovery, navigation and Save As copies
save editable work without publishing images. These user
project folders are separate from the `SVS_HOME` application-resource root;
preserve its existing CLI/config resolution and standalone database behavior.

# Layout

A Cargo workspace: eleven packages.

| Path | Package | Owns |
| :--- | :--- | :--- |
| `crates/studio-core` | `storyfella-studio-core` | the domain layer — artifact registry and typed ops (`registry/`), `persistence/`, `llm/`, `runtime/` (the `CreativeRuntime` adapter over the shared `agentkit-ai` crate), `errors.rs`, `config.rs`, `story.rs`, `paths.rs`. **No GUI dependency** |
| `crates/language` | `storyfella-language` | lossless lexer, recovering parser, typed AST, source positions and shared analysis/services; independent of Studio |
| `crates/compiler` | `storyfella-compiler` | deterministic, versioned story plans with scene flow, exact asset revision bindings, source maps and readiness; independent of Studio |
| `crates/lsp` | `storyfella-lsp` | stdio LSP transport over the shared language analysis |
| `crates/runtime` | `storyfella-runtime` | deterministic story execution, events and verified save/restore; no GUI/inference |
| `apps/studio` | `storyfella-studio` | the Dioxus app — `ui/`, `state/`, `commands.rs`, `driver/`, `app/`, plus `Dioxus.toml`, `tailwind.css`, `assets/`, `package.json` |
| `apps/atelier` | `storyfella-atelier` | independent GPUI drawing/image app, binary `atelier`; `.atelier` projects and Atelier data identity; hosted images through Agentkit |
| `apps/cli` | `svs-cli` | `svs`, `model_setup`, `vlm_probe`, the VLM fixture example and its fixtures |
| `apps/player` | `storyfella-player` | standalone Dioxus visual Player; same runtime/resources/controls as Studio preview |
| `crates/playback-ui` | `storyfella-playback-ui` | shared Dioxus presentation and playback controls; independent of Studio state |
| `xtask` | `xtask` | `cargo xtask` — the developer task runner (build/run/bundle/check/test); not part of any shipped artifact |

`crates/studio-core` never depends on the GUI. `apps/studio` and `apps/cli` both depend on
`storyfella-studio-core`, and the desktop crate names `storyfella_studio_core::…` explicitly rather than re-exporting
it, so the boundary stays visible at every call site.

### Storyfella boundaries

`storyfella-language`, `storyfella-compiler` and `storyfella-lsp` are implemented.
`storyfella-runtime` is implemented with a real CLI trace caller; see
`docs/ROADMAP.md#story-execution-and-playback-reference`. Do not add empty crates or workspace members.
Language analysis and compilation must remain independent of Studio, GUI
frameworks, and inference backends. The story runtime owns execution, state,
and timing; Studio preview and Player consume the same runtime. The LSP
exposes shared language analysis to external editors. Studio core stays GUI-free.
Its existing `runtime/` is the creative inference adapter, separate from story
execution runtime. Clipforge and FineWineTTS remain separate repositories.
See `docs/language.md`, `docs/compiler.md` and `docs/ROADMAP.md` §0 for implemented language scope
and the remaining compiler, persistence and story-execution milestones.
Keep `svs` CLI binaries, command behavior, `SVS_*` environment variables, error
codes and databases stable during this move. Studio's platform data/config
namespace is now `dev.storyfella.studio`; `identity.rs` migrates existing data
once without overwriting a destination or removing originals.

Atelier is a separate GPUI executable, not a Dioxus module. Use
`cargo xtask atelier` (separate from Studio's `cargo xtask run` and the `svs`
CLI) and `cargo xtask atelier bundle` (macOS and Linux packages in
`dist/atelier/`). Preserve its Atelier data directory,
`com.storyfella.atelier` bundle identity. The vendored
`gpui-pre-apple` patch enables its shared BGRA Metal surfaces. The existing
FineWineTTS review app still uses GPUI, so that repository retains its patch.
Atelier's hosted API wire code and model presets live in
`agentkit_ai::images`. The GPUI worker owns a Tokio runtime and adapts
raw output bytes to artboard layers/candidates or references; Clipforge owns
provider setup and hosted keys in the shared `clipforge.toml`, while
ComfyUI workflow execution stays app-owned. Studio's creative runtime still uses the native
`sd` API; hosted-provider UI integration remains separate. See `docs/atelier.md`.
Atelier's shared library stores immutable full-resolution originals plus global
metadata and mask drafts under `data_dir/library`; `.atelier` projects embed
their own references/image layers. `editor.rs`/`editor_ui.rs` own the large-image
workflow; `edit.rs` owns mask encoding and protected-pixel compositing. The mask
editor offers paint, erase and a rectangle **box selection** (`Tool::Rect`).
Atelier's floating **Create / image prompt** composer handles new artboards,
full-artboard candidate edits, selection blends and separate images. Selection
blend sends full accepted artwork as context and crops the returned rectangle;
separate mode sends the selection crop to a new artboard. Request sizes and
sampling ride the shared `agentkit_ai::images::Request`; Settings retains
model presets, LoRA request choices and ComfyUI mappings. `workflow.rs` owns
artboard attachment/schema upgrades, and `project_tabs.rs` owns independent open
documents and session recovery. Provider
mask/reference capabilities and wire formats belong only in Agentkit. Approved
`.sfart` exchange lives in GUI-free `studio-core::artwork`; Atelier retains its
standalone export. Studio uses shared-project Save/refresh instead of package
import. Use isolated `.svs-e2e` data for live UI checks. On macOS,
duplicate old Atelier bundle registrations can misdirect automation by name;
give disposable test bundles a distinct identity, preserving the shipped one.

### Shared inference through Agentkit

The workspace consumes `../../zerune_labs/agentkit/crates/agentkit-ai` with
`native-inference` enabled. `agentkit_ai::native` owns Qwen/Krea/MOSS profiles,
sdcpp clients, LoRAs, verified provisioning, llama readiness, shared process
markers and controls. `agentkit_ai::deployments` resolves and persists the
existing provider TOML schema; `agentkit_ai::images` owns native/hosted image
execution and capability validation. The `images` feature works independently
of native servers. The `clipforge-inference` crate has been removed.

Storyfella owns `ai_config`/`paths`: the unchanged `clipforge.toml` and shared
server-state locations, including SVS overrides. It also owns model locations,
project/artwork persistence, prompts and planner tool execution. Studio's LLM
adapter consumes Agentkit's existing OpenAI-compatible provider; it owns no
second chat HTTP implementation. Clipforge's executable, installer and checkout
are not needed to build or execute Storyfella inference. Its optional setup app
and sd-server build tooling remain in the separate Clipforge repository.

`CreativeRuntime` adapts Agentkit to registry jobs and stored artwork.
`ImageCapabilities` drives Atelier queue preflight and provider execution;
unsupported controls are rejected, never silently discarded by the backend.
The native Qwen route preserves input alpha, supports verified transparent PNG
output and accepts a source plus two ordered references. Its profile owns these
limits; Atelier queue preflight and execution consume the same contract. Qwen's
optional native mask anchors source latents (`MaskSupport::OptionalLatent`);
VAE reconstruction can change protected pixels. Mask and source dimensions must
match, and masked output uses the source dimensions. Krea has no editing/mask
support. Exact protected-pixel compositing remains app-owned.

**The app's image deployment is a provider.** The `sd-server` executable, model
tree, LoRA root, endpoint, and weight overrides come from the shared
`clipforge.toml` (resolved by `paths::clipforge_config_path()`, so never
working-directory-relative), loaded with
`agentkit_ai::deployments::Providers::from_toml(…)` and resolved
with `image_deployment(Capability::T2i)` into an optional `ImageDeployment` kept on
`AppConfig::image`. Hosted or absent image defaults leave it unset; Studio
authoring/playback and text Settings must remain available, without native
prewarm, server actions or model downloads. Native-only callers use
`AppConfig::native_image()` or the checked runtime client and report
SVS-CFG-110 when unavailable. Never rewrite shared hosted defaults to a native
fallback. `CreativeRuntime`, the health probe, `model_setup`, and the
Settings screen all read that deployment; the LoRA discovery directory is the
provider's `lora_dir`. A provider may change *where* a model comes from, not
*which* model it is: `architecture` must be a served profile.

## Where paths come from

`crates/studio-core/src/paths.rs` anchors every path the app resolves for itself — the plan
directory and the configured `paths.*` directories — to one **project root**: `$SVS_HOME` if
set, otherwise the nearest ancestor of the working directory (then of the executable) holding
the `config/plans` directory. Relative `paths.*` values resolve against that root, so the GUI,
the CLI and the plans work from any directory. The app config is one platform-local file —
`studio.toml` in the platform application config directory (`paths::config_path()`,
`$SVS_USER_CONFIG` overrides) — created on first load when absent and seeded with
`[generation.sizes] character = "1024x1536"` (the portrait canvas); every other key falls back
to the built-in code defaults. CLI generation reads these defaults; Studio no
longer edits them or consumes the old prewarm key. Inference settings live in a
separate shared file, `clipforge.toml`
(`paths::clipforge_config_path()`, `$SVS_AI_CONFIG` overrides), resolved from the platform
config directory and shared verbatim with Atelier and Clipforge; the provider registry,
capabilities, `[defaults]`, endpoints, API keys, residency, `model_dir` and LoRA root persist
there. Relative values in `clipforge.toml` resolve against that file's own directory, so
`model_dir` is seeded absolute under `paths::model_dir()`: OS-local application data under
Storyfella, in every build. Generated assets and project databases default to the platform
data dir (`assets`, `projects`) and cache to the platform cache dir, each absolute; a
`paths.*` value is an explicit override. The first run migrates old user app/provider files
into the two files without deleting originals or replacing an existing file. Never
reintroduce a working-directory-relative path or a `parents[N]` walk.

# Project rules

Studio appearance (`appearance.theme`: dark/light/system) and the Models toggle
(`models.separate_vision`) persist in `studio.toml`. Separate Vision resolves the
shared registry's Vision selection; linked mode makes effective `AppConfig::vision`
follow Text without rewriting the saved separate selection or credentials.
Preserve existing Vision routing, including an unconfigured capability, when
the toggle preference is absent.
The `ui/theme.rs` host follows OS changes and CSS semantic colors theme editors
without replacing their document state. The former Studio Canvas is Assets;
use `ui/assets.rs`, `go_assets` in guides and `g a` navigation.

- **Central planning tracker.** Read `docs/ROADMAP.md` §0 before planning changes. Keep new TODOs, priorities and completion status there; link supporting specifications and acceptance evidence instead of adding separate backlogs. §12 retains existing feature status, with remaining work to reconcile through the §0 checklist.
- **Keep MCP tools current.** When adding or changing a feature, update the affected MCP tools, schemas, documentation, and tests in the same change. Use the same application operations and validation for MCP and the desktop UI.
- **Keep task work inside the current project checkout.** Implementation, documentation and every output whose location we control belong here. Use `.svs-e2e/` for scratch work and `target/` for build output. Do not choose `/tmp`, another checkout, Downloads, an agent scratchpad or platform application-data directories for work that can live here. This applies to scripts, downloaded inputs, generated media, test projects/databases, exports, screenshots, logs and caches. Check tool defaults and native file-picker destinations before writing; plan runs already isolate projects/artwork/cache under `.svs-e2e/`.
- **Permissions do not change the project boundary.** If the sandbox covers a sibling repository instead of this checkout, request the required tool escalation here; never relocate the work to an externally writable directory. Read installed tools, skills, system libraries and the existing sibling inference dependency in place, but do not edit those repositories or put this task's files there. A necessary external output requires an explicit user instruction; explain a concrete tool limitation before requesting an exception. Tool-owned storage that we cannot redirect does not authorize choosing an external workspace.
- **Desktop only.** Never add web, mobile, WASM, routing, or fullstack features. The `apps/studio/Cargo.toml` features are limited to `default = ["desktop"]` / `dioxus/desktop`. Ignore any Dioxus docs about routers, server functions, or hydration.
- **Clean code is required.** Keep each change focused, readable and complete. Fix quality issues found in the affected paths before committing; successful compilation alone is not a code review. Apply the code quality rules below to implementation, tests, manifests and documentation.
- **No stale/dead code; no backward-compatibility shims.** This project is pre-1.0: do not add or retain deprecated aliases, alternate old-format parsers, import shims, redundant wrappers or superseded implementations for backward compatibility. Replace the old shape and update all consumers in the same change. Remove unused functions, types, fields, modules, dependencies, feature flags, no-op handlers and commented-out code; Git preserves history. Never hide unused code with `#[allow(dead_code)]` or broaden lint suppressions to get a check to pass. Existing SQLite migrations remain active upgrade code for stored projects; they are not permission to keep a second runtime model or legacy parser. Preserve actual project data when changing persistence.
- **Unshipped formats may break.** Apply breaking changes directly to unreleased APIs, MCP schemas, configuration, and saved project formats. Update callers, fixtures, and docs together. Do not add missing-field defaults, aliases, fallback parsing, migrations, or compatibility tests solely to keep an earlier development build's format working. This applies even when the fallback would fit in the current loader. When a field can be empty in the current format, distinguish an explicit empty value from a field omitted by an older format. Keep existing migrations in place. If a specific project needs conversion, handle it as an explicitly scoped data task.
- **Reusable components before repetition.** Isolate repeated patterns into small shared components/modules and reuse them everywhere. The UI vocabulary in `apps/studio/src/ui/components.rs` (`SettingRow`, `EmptyVisual`, `StatusDot`, …) and `apps/studio/src/ui/icons.rs` is the model; apply the same instinct outside the UI (registry ops, pipeline steps, persistence helpers, config). When a new use case needs a variant, extend the shared component instead of copy-pasting a near-identical one, and keep similar use cases looking and behaving the same. A new control for a case an existing control already covers is a code smell.
  Consult `docs/ui-components.md` for available components and examples. Plain
  fields, fields with a trailing icon action and search use
  `ui/inputs.rs` (`TextInput`, `TextInputWithAction`, `SearchInput`); preserve the
  native text-value bridge and update the catalog when extending this vocabulary.
  Display-first metadata fields use `ui/editable.rs` (`EditableText`,
  `EditableMultiline`); saved-artwork previews use `ui/artwork.rs` so transparent
  PNGs share a visible display background without modifying their pixels.
- **No perl for code edits.** Never use `perl -i` (or similar opaque one-liner rewrites) to modify source files — they are hard to review and have corrupted code before. Make edits with the file editing tools (`edit_file`/`write_file`), which produce reviewable diffs; `cargo fmt` is the only bulk formatter allowed.
- **Rust talks to the generation backend over HTTP.** The native `sd-server` client lives in Agentkit (`agentkit_ai::native::sd::Client`, with `GenerateRequest` and the job types) and is consumed through `CreativeRuntime`; do not re-implement it here. It speaks the server's `sdcpp` API (`POST /sdcpp/v1/img_gen`, `GET /sdcpp/v1/jobs/{id}`, `GET /sdcpp/v1/capabilities`). Keep the request/response types in sync with `stable-diffusion.cpp`'s `examples/server/api.md` at the revision `clipforge` pins (`cargo xtask sd-server`); that revision is authoritative. There is no Python in this project — do not reintroduce one.
- **No ML frameworks in Rust.** Rust must never depend on or know about PyTorch, Transformers, or any other ML framework. Inference is the native `stable-diffusion.cpp` process; Rust is only its HTTP client.
- **Text and vision are capability-routed providers.** There is no `[llm]` section: the planner (`Capability::Text`) and vision (`Capability::Vision`) are declared in the shared `clipforge.toml` (`paths::clipforge_config_path()`, `$SVS_AI_CONFIG` overrides) so a key stays out of git. An `openai` provider is an external service the app never starts (the default); a `llamacpp` provider is a managed native `llama-server` the app starts on demand. Weights are never bundled. `crates/studio-core/src/llm/` is the client only; the planner vocabulary (typed operations + pipelines) lives in `crates/studio-core/src/registry/`; LLM steps are soft dependencies that degrade to manual input, never hard-fail.
- **UI and UX standard:** read `DESIGN.md` before changing user-facing flows, controls, feedback or styling. It is the central design reference, audit and remediation record; update its acceptance evidence when fixes land.
- **UI styling** is Tailwind CSS 4 utility classes in RSX, with shared styles in `apps/studio/tailwind.css`. `apps/studio/assets/tailwind.css` is generated output; regenerate with `npm run css:build` from `apps/studio`. Prefer the small shared component vocabulary in `apps/studio/src/ui/components.rs` (`SettingRow`, `EmptyVisual`, `StatusDot`, …) and icons from `apps/studio/src/ui/icons.rs`.
- **Writing diff editor:** CodeMirror merge owns continuous editing and diff layout in `apps/studio/ui-js/writing-diff.js`, bundled locally by esbuild through Studio's Cargo build script. Run `npm ci` in `apps/studio` after checkout/dependency changes; never hand-edit or commit the ignored generated bundle. Rust owns model requests, source/project guards, compiler checks, acceptance, undo and saving. The JavaScript bridge sends current text, UTF-16 ranges and edit sequences; it never writes files or calls providers. Keep pending edits separate from accepted source used by saves and Preview.
- **State mutations** go through `AppState` methods in `apps/studio/src/state/mod.rs`, which maintain the registry invariants documented in `docs/data-model.md`. Never mutate `Project` fields directly from UI code.
- **Build artifacts and tool temporary files stay inside the checkout.** Keep `.cargo/config.toml`: it selects gitignored `target/` and sets `TMPDIR`/`TMP`/`TEMP` to ignored `.cargo/tmp/` for Cargo, tests and inherited subprocesses. Keep that temporary directory's tracked `.gitignore` so it exists on a fresh clone. Do not override these paths with external locations. Use a checkout path without `:` for macOS/Linux library-path compatibility; do not reinstate an external target directory as a workaround. Older handoffs documenting an external build target do not override this policy.
- **Build Studio with `dx build --platform desktop` for runnable binaries, not plain `cargo build`.** Run `dx` from `apps/studio`, where `Cargo.toml` and `Dioxus.toml` live. The `asset!` pipeline (Tailwind CSS + custom protocols) is wired by the dx CLI: a plain `cargo build` Studio binary starts fine but renders **unstyled** (no CSS — the stylesheet is never embedded/served). `cargo check`/`cargo test` are workspace-wide (`--workspace`); launching Studio uses its dx artifact (`target/dx/storyfella-studio/debug/linux/app/storyfella-studio` on Linux). Player embeds its styles and supports Cargo launch. `cargo xtask build`/`run`/`serve`/`bundle` (and `check`/`test`/`fmt`/`clippy`) wrap the Studio commands from the workspace root — prefer them over retyping the invocations.
- **Validation logs and scratch files stay in the repo** under `.svs-e2e/` (gitignored). When live-testing the GUI, write its stdout/logs to `.svs-e2e/logs/` — never to `/tmp` or the repo root. When killing a stale app instance, use a bracket-quoted pattern so `pgrep`/`pkill` do not match their own command line (e.g. `pkill -f "storyfella-studi[o]"`).
- **The vision evaluator — the agent's eyes.** The agent cannot see images; when judging generations, edits, or regressions, ask the LLM endpoint's vision model through `svs eval -i <image>… -p "<question>"` (default model `google/gemma-4-e4b`, served by llama.cpp or LM Studio). Images are auto-downscaled to 1024px; default max-tokens 2048 (the model reasons before answering, and 512 was exhausted by reasoning). `--expect <substring>` / `--expect-not <substring>` gate the exit code for scripted checks — craft prompts that elicit strict YES/NO. Comparison workflow: attach the base first, then the candidate(s); the model refers to them as image 1, image 2, …. Common checks: identity preservation across an edit or variant, artifacts/distortion in a generation. Evaluations used to drive a decision should be recorded in `.svs-e2e/logs/`.
- Consult the documentation below before making changes; many decisions and known gaps are recorded there.

# Code quality and best practices

- **Implement current requirements.** Add APIs and abstractions for real callers or a concrete public contract, not hypothetical future use. Keep planned work in the roadmap. Deferred language instructions may remain only when a named milestone requires them, their limitation is documented, and diagnostics/readiness explicitly expose it; never silently pretend to execute them.
- **Keep one authoritative implementation.** Share parsing, validation and domain behavior across CLI, Studio, Player and LSP. Keep transport and UI adapters thin. Avoid duplicate models and recomputing domain rules differently in each host. Extract shared code when actual repetition justifies it; avoid unnecessary indirection.
- **Make invariants visible.** Prefer precise types, explicit inputs and clear ownership. Keep functions and modules cohesive. Use descriptive names and straightforward control flow. Validate untrusted input at its boundary; return actionable errors instead of panicking, silently dropping input or guessing a default. An internal `unwrap`/`expect`/`unreachable!` must follow an established invariant, not stand in for validation.
- **Keep effects explicit.** Separate pure language/compiler logic from files, databases, UI and inference. Do not mutate host state while analyzing or compiling. Preserve authored text and source locations, produce deterministic results, and distinguish source validity from linking and execution readiness.
- **Keep dependencies deliberate.** Reuse existing workspace dependencies and standard-library facilities first. Add a dependency only when its concrete benefit warrants it, declare it once at the workspace root, and respect the crate boundaries above. Remove dependencies made obsolete by a change.
- **Write useful comments and current docs.** Explain invariants, decisions and limitations rather than narrating syntax. Remove obsolete TODOs, examples, status claims and documentation of deleted APIs in the same change. A TODO must describe a real remaining gap and point to its tracked work; do not leave speculative placeholders in source.
- **Test behavior and failure boundaries.** Cover changed semantics, malformed input and meaningful regressions with deterministic tests. Use process/integration tests when exit status, protocol framing or filesystem effects are part of the contract. Do not add tests that merely mirror implementation details or assert documentation wording. Keep test fixtures isolated and clean them up.

# Documentation

- `DESIGN.md` — central UI/UX requirements, visual and accessibility standards, prioritized audit findings and acceptance checks.
- `docs/ROADMAP.md` — central plan and TODO tracker (§0), with earlier feature history (§12). Read §0 first and update its task status when work lands.
- `docs/architecture.md` — current component boundaries, registry operations, inference contract and native server lifecycle.
- `docs/development.md` — setup, build/run/test and CLI commands, configuration and troubleshooting.
- `docs/data-model.md` — domain types, state invariants, persistence format, known gaps.

# Current status

The API slice (`docs/ROADMAP.md` §12 items 1–3), the canvas (item 4), the
generation quality controls (item 5), the LoRA registry (item 6), and the
error-code registry (item 7) are implemented and validated:

- `crates/studio-core/src/registry/` — artifact registry (kinds, variants, scenes/beats + layers, revisions, drafts, LoRA entries), typed slice-1 operations (`create`, `variant`, `regenerate`, `compose`, `draft`, `modify`) + LoRA ops (`attach-lora`, `lora-scale`, `lora-remove`), pipeline builder (closed step vocabulary — `LoadImage`, `Checkpoint`, `Generate`, `Llm`; typed handles, static validation at `build()`, linear fail-fast stacks, checkpoints), live backend (`CreativeBackend`), slash-command parser (`slash.rs`) for metadata guides and core operation scripts.
- `c:<name>` references are primary (case-insensitive exact match, ambiguity rejected; UUID/8-hex fallbacks).
- `crates/studio-core/src/persistence/` — SQLite project store (`ProjectDb`): §10 schema (v3 dropped the legacy `project_json` column; v8 dropped the `masks` table; v9 exports Writer source to `.sf` and drops `projects.story_source`), versioned migrations (`schema_meta`), WAL, lossless round-trip (including the `lora_registry` table); the CLI persists to `.svs-project.db`. No legacy formats are parsed — the current format evolves via migrations.
- `apps/studio/src/ui/` — the project workspace has Story, Assets and Preview plus Projects, Plans and Settings navigation. Projects is a compact list without the workspace bars. Assets browses definitions, saved artwork and read-only revision history. `inspector.rs` edits source-facing names/descriptions; `art_actions.rs` creates definitions/variations through `AppState::edit_catalog` without inference. `palette.rs` exposes navigation and authoring actions only. The visual composer, image production controls, revision activation/rerun and `.sfart` import UI are removed by SF-PLAN-07. Open in Atelier owns the visual handoff; linked Save refreshes Studio. Writer's writing workflows remain. `shortcuts.rs` documents bindings and `keys.rs` owns the one key listener. Normal save/autosave, source/catalog undo and source conflict checks remain.
- `svs` CLI — `op` (incl. `--lora <file>@<mult>`, `attach-lora`/`lora-scale`/`lora-remove`), `stack run`/`propose`, `runtime serve --force` (Qwen-Image-2.1 profile only), `log`, `project`; `--out` golden runs; `--approve auto|interactive`; `eval` (vision evaluation through the LLM endpoint — see the project rule on the vision evaluator). There is no `segment` subcommand.
- **CLI generation defaults**: `studio.toml` retains `[generation]`, `[generation.sizes]` and `[lora]` loading for `svs`; existing configuration/provenance is preserved. Studio no longer edits image request preferences or prewarms image services on project open. Atelier owns visual production settings. Shared typed operations, planner, pipeline and CLI behavior remain independent of the GUI.
- **`modify` is an instruction edit, and every instruction is grounded.** The artifact's active revision is the reference image and the grounded plain-language instruction is the prompt — no mask, no segmentation, no composite. A terse instruction under-specifies the render and collapses into base-model priors — measured, `"make him smile"` returned an *anime illustration of a different person*, twice, at two aspect ratios. `ground_instruction` (ops.rs) restates the medium and the artifact's traits as invariants; when an LLM endpoint is configured, `llm_ground` refines it by dropping the attributes the edit overrides (restating both "long wavy blonde hair" and "a bob cut" makes the model split the difference). The LLM is a soft dependency here: it never blocks, it only sharpens the deterministic fallback. **The planner model matters**: this subtraction is model-sensitive — `gemma-4-e4b` kept "long dark hair" while the edit asked for auburn and in one phrasing invented a colour that appeared nowhere in the input; `bonsai-27b` updated the trait and handled the haircut and outfit cases too. `clipforge.toml` defaults to the larger planner. **Grounding is not modify-only**: a variant description and an explicit regenerate prompt are the same terse-delta shape and drift the same way — an ungrounded variant of a woman in her forties returned a different person of a different sex. Both ground too (a variant against its *base*); a regenerate with no prompt does not, since it replays an already-grounded prompt. **Grounding anchors on the artifact description, not the current revision**, so chained edits re-anchor to the description and earlier edits wash out (create dark → recolour auburn → haircut came back dark). That is deliberate — an artifact has one canonical look — and `describe` is the op that promotes an edit to canonical. `/variant` forks a look into its own artifact instead; variants cannot nest. `/revert` re-activates the previous completed image (a Direct op, no render): nothing is deleted, the reverted-from revision stays in the history, and it remains available through `svs op revert`; Studio's history is read-only.
- **The native backend (Qwen-Image-2.1)**: `sd-server` is launched with `--diffusion-model`/`--llm`/`--vae` pointing at the profile's three safetensors (diffusion, text encoder, VAE) — or at the local file a `clipforge.toml` `weights` override names — plus `--model-args qwen_image_2_1_prefix_cache_type=q8_0` (an explicit quantized prefix cache keeps the working set small; flash attention is on via `--diffusion-fa`). It has **no few-step turbo path** — the default is CFG 6.0 at 25 steps with the `euler` sampler. A reference edit sends the image as the first `ref_images[]` entry (base64). **VAE tiling must be explicit**: the client always sends `vae_tiling_params` with a 256px tile and 0.5 overlap, because this build ignores tiling when only `enabled` is set and the untiled decode OOMs at a 1024x1536 canvas. **VRAM budget on a 24GiB card**: measured peak ≈23.4 GiB (weights resident ≈15.9 GB) — the LLM/planner cannot be resident during a generation; planner and generator must be swapped, not co-resident. Measured on RTX 3090 (int8, 25 steps): t2i 1024² ≈52 s sampling (≈1.9 s/it with CFG), reference edit/cutout 1024×1536 ≈140 s. **Alpha/RGBA cutouts come from the model's prompt-driven alpha channel** (verified against upstream's own examples), not from any segmentation + matting: the prompt asks for transparency and the alpha survives only in PNG/WebP. Cutout quality is subject-dependent — reliable for logo/object subjects, but a fixed blonde portrait produced the same failure under four wordings (opaque hair, transparent face). The authoritative request/response fields live in `examples/server/api.md` at the revision `clipforge` pins — read it before adding a field.

**Verifying a generation input actually reaches the model**: anything the server does not name in its JSON parser is dropped without a warning. When a field "has no effect", grep the `clipforge` checkout's `stable-diffusion.cpp/examples/server/api.md` and its parser for the exact key before concluding the model ignores it.

- **LoRA serving**: sd.cpp registers only the *top-level* files of its lora directory. `prepare_lora_links` symlinks subdirectory adapters to the top level as `<subdir>-<basename>` before the server starts, and `LoraBindings::registered` maps request paths to those names (absolute paths are rejected by the server). Repeated `--lora` flags accumulate (slash + CLI). Multi-LoRA is validated end-to-end (several adapters applied together).
- **LoRA registry** (§12 item 6): project-level entries (SQLite `lora_registry`, round-tripped) with per-artifact bindings (`/attach-lora`, `/lora-scale`, `/lora-remove` — slash, CLI, LLM vocabulary), config use-case bindings (`[lora]`: per-kind defaults, shared multiplier), discovery from `lora_dir`, and auto-injection into every generation (artifact-bound → kind default; missing-on-disk adapters warn and skip).
- **Error codes + reporting** (§12 item 7): `crates/studio-core/src/errors.rs` is the central registry — every `thiserror` type implements `CodedError` mapping to a stable `SVS-<DOMAIN>-<NNN>` code (code, short message, remediation hint); `CodedError::report` builds the `ReportedError` payload. Workspace owns the notification signal; App owns the separate `ErrorDetails` context so a local Details action opens the complete report directly, including during configuration failure. Shared native `Dialog` provides modal behavior; Copy awaits clipboard success. `ReportedError` carries a `context` list (operation, generation prompt, model, size, steps, seed, job id, work dir) attached at the failure site (`PipelineError::Backend` carries it up from `CreativeBackend::generate`) — surfaces emit the single coded log line with `report.log()`. The CLI's op paths report the same way (svs.rs inits a tracing subscriber).
- **Text and vision are provider-driven too**: `AppConfig::text` and `AppConfig::vision` are `ChatDeployment`s resolved from the shared `clipforge.toml` (`Capability::Text`/`Capability::Vision`) (`paths::clipforge_config_path()`, `$SVS_AI_CONFIG`). The planner (`CreativeBackend`, `vlm_probe`) reads `planner_config()`; `svs eval` and the plan driver's `expect_vision` read the `vision` deployment through `crates/studio-core/src/vision.rs`, so a non-vision planner (e.g. DeepSeek) no longer blinds vision. `svs eval` starts a managed `llamacpp` vision server on demand; an `openai` provider has no server. A managed `llamacpp` provider is started on demand — `CreativeBackend::ensure_planner_ready` before a planner call, a `VisionSession` for `svs eval`, or **Start** on Settings → Services — and `stop_chat_servers` stops every managed chat server before a generation, so no chat model shares the card with the image engine. `CreativeBackend` supervises them by provider id (`start_chat_server`/`stop_chat_server`/`restart_chat_server`/`chat_server_running`), `set_chat` re-points it at a new config, and the same Agentkit `llama-server` runtime is used throughout. Settings is **General / Models / Services / Storage**. **Models** is the "what to use" page: **Text (LLM)** and **Vision (LLM)** cards — each with an inline status and the shared **Check now** — plus shared model-file administration. Both chat cards pick from one provider list (`AppConfig::chat_providers` — every `openai`/`llamacpp` provider, whatever it declares); selecting one for a capability records it there (`persist_provider_capabilities`), so any provider can serve text, vision, or both — the user decides. The **Model** is a dropdown of the ids the endpoint serves — read through the shared `agentkit_ai::deployments::chat::Client::list_models` (`fetch_models`) — with a refetch action; only served ids are offered, so a saved model the endpoint does not list reads as unserved instead of being shown. **Base URL**, **Model** and **API key** are editable for **every** provider — Text and Vision share one card, and a managed (`llamacpp`) server is not a different shape; editing a managed provider's base URL also moves the `llama-server` it launches (`with_chat_endpoint`), since that address derives from the URL, and an unparseable one is rejected. A managed provider that is not running reads `Idle · starts on demand` (not `Offline`) and offers no model until it is up (`OnDemand`), because the app starts it on demand for a planner call or `svs eval` — the same on-demand semantics the image engine gets; **Services** gives each managed provider **Start/Stop/Restart** (with the model file it loads), so its `/v1/models` list can be fetched immediately. A **Providers** section lists every configured chat provider and can **add** one (`persist_provider_added`, validated against the shared registry before it is written) or **remove** one the file defines (`persist_provider_removed`, which also drops its `[defaults]` entry); `AppConfig::user_providers` tracks which ids `clipforge.toml` owns. **Services** is the "how it runs" page: the image engine's status and start/stop/restart/take-over actions, executable, runtime endpoint, residency, the model-files provisioning list, and each managed `llama-server` provider's status with **Start/Stop/Restart**. **Storage** holds the resolved paths (models, assets, projects, cache) with Reveal and the Application configuration (`studio.toml`) and Inference configuration (`clipforge.toml`) files. An edit, or a provider switch, persists to `clipforge.toml` (`[providers.<id>]` or `[defaults] text`/`vision`), rebuilds the planner client, re-runs the health check and re-discovers the endpoint's models; `AppConfig::with_chat_endpoint` keeps a provider and its active deployment in step. The separate Status and Image-generation sections are gone. Text and vision are **optional**: a capability no provider serves resolves to `None` and degrades (`CreativeBackend` drops its LLM client, `svs eval` reports no `vision` provider, Settings shows "not configured") rather than failing config load. The image-carrying chat call lives in the shared `agentkit-ai` crate (`chat::Client::complete_with_images`, `ChatImage`); storyfella's `crates/studio-core/src/vision.rs` (`VisionSession`) is the one home that encodes an image file, starts a managed vision server, and gates a substring verdict — used by `svs eval` and the driver's `expect_vision` step.
- **The image backend is provider-driven**: the shared `clipforge.toml` declares the capability defaults and provider deployments. Configuration validates it through `agentkit_ai::deployments::Providers::from_toml(…)`; native t2i resolves to the optional `AppConfig::image`, and provider errors map to SVS-CFG-108. Hosted or absent t2i defaults keep authoring available. Native callers use the checked `AppConfig::native_image()` deployment or runtime client. An omitted model directory uses OS-local Storyfella storage; explicit overrides preserve existing model trees. LoRA discovery uses the selected native provider's `lora_dir`; `model_setup` provisions the same deployment, skipping local weight overrides.
- **The automation driver** (`apps/studio/src/driver/` + `commands.rs`) runs authoring/playback YAML plans in Guide or Check mode. `type_story`/`sample_project` animate source through the normal setter; `command` supports descriptions and text-only story/scene/beat creation. Plans preflight every command on load; Studio also validates complete operation batches before changes and executes with no image backend. Visual `compose`/simulated-render steps and generation plans are removed. The shipped plans are `smoke.yaml`, `ui-tour.yaml` and `playback.yaml`. State/image/optional vision expectations and isolated plan storage remain; see `docs/development.md`.
- Language, compiler and LSP tests cover source preservation, recovery, navigation, flow, linking/readiness and CLI/protocol behavior; run the workspace checks below for current results. The only generation profile is `qwen-image-2.1`; its t2i / instruction-edit / RGBA-cutout behaviour was measured on a CUDA `sd-server` (RTX 3090) in the 2026-09-30 spike — see `.svs-e2e/logs/2026-09-30-quality-evals.md`. LLM `draft`, `stack propose`, LoRA injection, and the size/step controls still await live validation.
- Studio integration (item 8) is active: Writer/Assets associations, schema-v10
  identity persistence, pending catalog creation, origins/backlinks and joint
  source/registry undo are implemented. Shared Writer completion/navigation,
  compiler diagnostics, rename, formatting, and catalog-backed LSP semantic
  tokens, shared headless execution, Studio preview and the visual Player are
  implemented. See `docs/ROADMAP.md#story-execution-and-playback-reference`; `docs/ROADMAP.md` §12 remains
  authoritative.

# Validation

Always verify changes that touch Rust code:

```sh
cargo check --workspace --all-targets
cargo test --workspace
cargo fmt --all -- --check
```

That is `cargo xtask check` (+ `test`), which wraps the same commands from the workspace root. Do not modify `Cargo.toml` features or add dependencies without a strong reason — dependencies live in `[workspace.dependencies]` at the root and are added there once, then referenced from a member with `name.workspace = true`; the `desktop` feature lives in `apps/studio` and `apps/player`.

Run `cargo clippy -p <changed-package> --all-targets -- -D warnings` for changed
Rust packages. Fix warnings introduced by the change; do not silence them to
pass validation. Report any existing failures separately and accurately.

Before committing, review the complete diff, including new files. Confirm that
replaced code and all consumers were updated, no compatibility or dead-code
paths remain in the affected scope, crate boundaries still hold, documentation
matches behavior, and tests cover the relevant failures. Run `git diff --check`
and inspect staged changes for unrelated edits, secrets, generated files and
scratch artifacts. Commit only the reviewed scope and report actual validation
results and any remaining limitations.

## Scratch files stay in the project, never in `/tmp`

Every temporary artifact of an experiment — probe scripts, generated images,
downloaded test inputs, server logs — goes under `.svs-e2e/`, which is
gitignored. Build products use `target/`; Cargo-managed temporary files use
`.cargo/tmp/`. These are local directories, not exceptions to the boundary.
Do not use `/tmp`, an agent scratchpad, or any other path outside
the checkout: those may be wiped between sessions, and generation experiments are
expensive to reproduce (minutes of model loading per run). An image that cost
40s to make and the exact script that made it must both survive to the next
session, or the next investigation restarts from zero — which is how this
project spent days re-deriving the same quality findings.

| Path | Holds |
| :--- | :--- |
| `.svs-e2e/inputs/` | Fixed test inputs (downloaded photos, hand-made test images) — the stable base an A/B is measured against |
| `.svs-e2e/scripts/` | Probe scripts. Resolve inputs and outputs from the project root, never hardcode a machine-specific absolute path |
| `.svs-e2e/quality/<experiment>/` | Generated outputs, one directory per experiment |
| `.svs-e2e/logs/` | sd-server logs, named for the configuration that produced them |
| `.svs-e2e/projects/`, `.svs-e2e/artwork/`, `.svs-e2e/cache/` | Isolated desktop-plan projects, generated artwork and cache |

Prefer a downloaded photo from `.svs-e2e/inputs/` over generating a base image
when the experiment is about *editing*: it removes a 20–40s generation from
every iteration and keeps the base bit-identical across runs, so a measured
difference is attributable to the change under test.

Write the measurement into the script, not the eyeball. Every probe should
print a number (change inside vs. outside a region, drift against the base)
so results are comparable across sessions; record the conclusions in
`.svs-e2e/logs/<date>-quality-evals.md`.

# Dioxus 0.7 essentials

Dioxus 0.7 changed every API. Only use the current documentation at <https://dioxuslabs.com/learn/0.7>. `cx`, `Scope`, and `use_state` are gone. Provide concise code examples with detailed descriptions.

## Dependency

```toml
[dependencies]
dioxus = { version = "0.7.1" }

[features]
default = ["desktop"]
desktop = ["dioxus/desktop"]
```

## Launching

Create a main function that sets up the Dioxus runtime and mounts your root component:

```rust
use dioxus::prelude::*;

fn main() {
	dioxus::launch(App);
}

#[component]
fn App() -> Element {
	rsx! { "Hello, Dioxus!" }
}
```

Serve with `dx serve --platform desktop`.

## UI with RSX

```rust
rsx! {
	div {
		class: "container", // Attribute
		color: "red", // Inline styles
		width: if condition { "100%" }, // Conditional attributes
		"Hello, Dioxus!"
	}
	// Prefer loops over iterators
	for i in 0..5 {
		div { "{i}" } // use elements or components directly in loops
	}
	if condition {
		div { "Condition is true!" } // use elements or components directly in conditionals
	}

	{children} // Expressions are wrapped in brace
	{(0..5).map(|i| rsx! { span { "Item {i}" } })} // Iterators must be wrapped in braces
}
```

## Assets

The asset macro links to local files. All links start with `/` and are relative to the root of your project:

```rust
rsx! {
	img {
		src: asset!("/assets/image.png"),
		alt: "An image",
	}
}
```

`document::Stylesheet` injects a stylesheet into the `<head>`:

```rust
rsx! {
	document::Stylesheet {
		href: asset!("/assets/styles.css"),
	}
}
```

Generated images are not static assets; they are served through the custom `generated` asset handler in `apps/studio/src/ui/mod.rs`.

## Components

- Components are functions annotated with `#[component]`.
- The function name must start with a capital letter or contain an underscore.
- A component re-renders only under two conditions: its props change (as determined by `PartialEq`), or internal reactive state it depends on is updated.

```rust
#[component]
fn Input(mut value: Signal<String>) -> Element {
	rsx! {
		input {
            value,
			oninput: move |e| {
				*value.write() = e.value();
			},
			onkeydown: move |e| {
				if e.key() == Key::Enter {
					value.write().clear();
				}
			},
		}
	}
}
```

Props are function arguments:

- Props must be owned values, not references. Use `String` and `Vec<T>` instead of `&str` or `&[T]`.
- Props must implement `PartialEq` and `Clone`.
- To make a prop reactive and copy, wrap the type in `ReadOnlySignal`. Reactive state like memos and resources that read `ReadOnlySignal` props re-run when the prop changes.

## State

A signal wraps a value and automatically tracks where it's read and written. Changing a signal's value reruns the code that relies on it.

### Local state

`use_signal` creates state local to a component. Call the signal like a function (e.g. `my_signal()`) to clone the value, `.read()` for a reference, `.write()` for a mutable reference.

`use_memo` creates a memoized value that recalculates when its dependencies change:

```rust
#[component]
fn Counter() -> Element {
	let mut count = use_signal(|| 0);
	let mut doubled = use_memo(move || count() * 2); // re-runs when count changes

	rsx! {
		h1 { "Count: {count}" } // re-renders when count changes
		h2 { "Doubled: {doubled}" }
		button {
			onclick: move |_| *count.write() += 1,
			"Increment"
		}
		button {
			onclick: move |_| count.with_mut(|count| *count += 1),
			"Increment with with_mut"
		}
	}
}
```

### Context API

A parent provides state with `use_context_provider`; any child consumes it with `use_context`:

```rust
#[component]
fn App() -> Element {
	let mut theme = use_signal(|| "light".to_string());
	use_context_provider(|| theme); // Provide a type to children
	rsx! { Child {} }
}

#[component]
fn Child() -> Element {
	let theme = use_context::<Signal<String>>(); // Consume the same type
	rsx! {
		div {
			"Current theme: {theme}"
		}
	}
}
```

## Async

For state that depends on an async operation, use `use_resource`. It takes an `async` closure and re-runs it whenever signals it reads are updated. Reading the `Resource` returns `None` while loading and `Some(value)` once loaded:

```rust
let mut dog = use_resource(move || async move {
	// api request
});

match dog() {
	Some(dog_info) => rsx! { Dog { dog_info } },
	None => rsx! { "Loading..." },
}
```

For fire-and-forget work (such as starting a generation and updating app state from a spawned task), use `spawn(async move { ... })` as in `run_operations` in `apps/studio/src/commands.rs`.

## Dioxus 0.7 usage notes (learned while building this app)

- `use_effect(move || { ... })` runs after the first render and re-runs whenever a signal it reads changes. The closure is `FnMut`, so it cannot move captured non-`Copy` values out — clone them inside (`runtime.clone()`) or move the originals in once and never use them again.
- `Signal::set(value)` consumes a copy of the signal, so the binding must be declared `mut` at the call site (`let mut prompt = use_signal(...); prompt.set(...)`). Prefer `*prompt.write() = value` when the binding is not `mut`.
- Signals are `Copy`. Pass them to child components as props (`Signal<T>`) and to plain helper functions that mutate them — the helper declares `mut signal: Signal<T>` parameters.
- `EventHandler<T>` props receive closures at the call site (`onclick: move |event| ...`). Handlers are `Clone`, so one handler can be shared between several components by binding it once.
- `use_hook` constructs one-time, non-reactive values (for example `CreativeRuntime::new(&config)` in `apps/studio/src/ui/mod.rs`).
- In `spawn`ed tasks, never hold a signal `.read()`/`.write()` guard across an `await`; read or clone first, await, then write (see `run_operations` in `apps/studio/src/commands.rs`).

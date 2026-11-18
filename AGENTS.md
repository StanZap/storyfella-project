You are working on **Storyfella**, the workspace for story authoring and playback. Studio is the desktop GPUI application; Atelier in `../storyfella-canvas` owns visual creation and editing. Clipforge owns provider management and general image inference tools. Studio selects text and vision providers and runs Writing help through Agentkit's Workflow API. Managed requests hold the service lease for their full operation.

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

A Cargo workspace: ten packages.

| Path | Package | Owns |
| :--- | :--- | :--- |
| `crates/studio-core` | `storyfella-studio-core` | the domain layer — artifact registry and typed ops (`registry/`), `persistence/`, `llm/`, `runtime/` (provider health checks), `errors.rs`, `config.rs`, `story.rs`, `paths.rs`. **No GUI dependency** |
| `crates/language` | `storyfella-language` | lossless lexer, recovering parser, typed AST, source positions and shared analysis/services; independent of Studio |
| `crates/compiler` | `storyfella-compiler` | deterministic, versioned story plans with scene flow, exact asset revision bindings, source maps and readiness; independent of Studio |
| `crates/lsp` | `storyfella-lsp` | stdio LSP transport over the shared language analysis |
| `crates/runtime` | `storyfella-runtime` | deterministic story execution, events and verified save/restore; no GUI/inference |
| `apps/studio` | `storyfella-studio` | the GPUI app — `ui/` (window, Home, workspace, Settings), `state/`, `commands.rs`, `host.rs` (the mailbox automation reaches the window through), `driver/`, `live_mcp/` |
| `apps/cli` | `sf-cli` | Story check/compile/trace and the project-scoped stdio MCP server |
| `apps/player` | `storyfella-player` | standalone GPUI visual Player; same runtime, resources and stage as Studio preview |
| `crates/playback-panel` | `storyfella-playback-panel` | the GPUI playback panel and stage, on the shared design system; Studio's Play view and the Player; independent of Studio state |
| `xtask` | `xtask` | `cargo xtask` — the developer task runner (build/run/plan/check/test); not part of any shipped artifact |

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
Studio core `runtime/` contains provider health checks; story execution remains separate. Clipforge and FineWineTTS remain separate repositories.
See `docs/language.md`, `docs/compiler.md` and `docs/ROADMAP.md` §0 for implemented language scope
and the remaining compiler, persistence and story-execution milestones.
Keep `sf-cli` story and MCP behavior, `SVS_*` environment variables, error
codes and databases stable. Studio's platform data/config
namespace is now `dev.storyfella.studio`; `identity.rs` migrates existing data
once without overwriting a destination or removing originals.

Atelier, the GPUI app for drawing, editing and generating artwork, lives in
its own repository at `../storyfella-canvas` and uses `storyfella-studio-core` from this
checkout by path. The shared project folder, catalog, `.sfart` exchange
(`studio-core::artwork`) and the Studio-to-Atelier inbox (`studio-core::atelier`)
are contracts between the two repositories: when a change touches them, run
`cargo test --workspace` in both before committing either. Studio uses
shared-project Save/refresh rather than package import. Atelier owns image
production; provider capabilities and wire formats belong in Agentkit.

### Shared inference through Agentkit

Studio binds its selected text provider through Agentkit's shared inference
service and runs Writing help as an Agentkit workflow. Clipforge manages the
shared provider registry and inference service settings. Atelier owns image
creation and editing; generic provider contracts belong in Agentkit.

## Where paths come from

`crates/studio-core/src/paths.rs` anchors every path the app resolves for itself — the plan
directory and the configured `paths.*` directories — to one **project root**: `$SVS_HOME` if
set, otherwise the nearest ancestor of the working directory (then of the executable) holding
the `config/plans` directory. Relative `paths.*` values resolve against that root, so the GUI,
the CLI and the plans work from any directory. The app config is one platform-local file —
`studio.toml` in the platform application config directory (`paths::config_path()`,
`$SVS_USER_CONFIG` overrides) — created on first load when absent and seeded with
`[generation.sizes] character = "1024x1536"` (the portrait canvas); every other key falls back
to the built-in code defaults. Studio does not use these settings to generate images. Inference settings live in a
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
without replacing their document state. Assets lives in `ui/assets.rs`;
guides use `go_assets` and navigation uses `g a`.

- **Central planning tracker.** Read `docs/ROADMAP.md` §0 before planning changes. Keep new TODOs, priorities and completion status there; link supporting specifications and acceptance evidence instead of adding separate backlogs. §12 retains existing feature status, with remaining work to reconcile through the §0 checklist.
- **Keep MCP tools current.** When adding or changing a feature, update the affected MCP tools, schemas, documentation, and tests in the same change. Use the same application operations and validation for MCP and the desktop UI.
- **Keep task work inside the current project checkout.** Implementation, documentation and every output whose location we control belong here. Use `.svs-e2e/` for scratch work and `target/` for build output. Do not choose `/tmp`, another checkout, Downloads, an agent scratchpad or platform application-data directories for work that can live here. This applies to scripts, downloaded inputs, generated media, test projects/databases, exports, screenshots, logs and caches. Check tool defaults and native file-picker destinations before writing; plan runs already isolate projects/artwork/cache under `.svs-e2e/`.
- **Permissions do not change the project boundary.** If the sandbox covers a sibling repository instead of this checkout, request the required tool escalation here; never relocate the work to an externally writable directory. Read installed tools, skills, system libraries and the existing sibling inference dependency in place, but do not edit those repositories or put this task's files there. A necessary external output requires an explicit user instruction; explain a concrete tool limitation before requesting an exception. Tool-owned storage that we cannot redirect does not authorize choosing an external workspace.
- **Desktop only.** Never add web, mobile, WASM, routing, or fullstack features. Studio and the Player are GPUI desktop windows.
- **Clean code is required.** Keep each change focused, readable and complete. Fix quality issues found in the affected paths before committing; successful compilation alone is not a code review. Apply the code quality rules below to implementation, tests, manifests and documentation.
- **No stale/dead code; no backward-compatibility shims.** This project is pre-1.0: do not add or retain deprecated aliases, alternate old-format parsers, import shims, redundant wrappers or superseded implementations for backward compatibility. Replace the old shape and update all consumers in the same change. Remove unused functions, types, fields, modules, dependencies, feature flags, no-op handlers and commented-out code; Git preserves history. Never hide unused code with `#[allow(dead_code)]` or broaden lint suppressions to get a check to pass. Existing SQLite migrations remain active upgrade code for stored projects; they are not permission to keep a second runtime model or legacy parser. Preserve actual project data when changing persistence.
- **Unshipped formats may break.** Apply breaking changes directly to unreleased APIs, MCP schemas, configuration, and saved project formats. Update callers, fixtures, and docs together. Do not add missing-field defaults, aliases, fallback parsing, migrations, or compatibility tests solely to keep an earlier development build's format working. This applies even when the fallback would fit in the current loader. When a field can be empty in the current format, distinguish an explicit empty value from a field omitted by an older format. Keep existing migrations in place. If a specific project needs conversion, handle it as an explicitly scoped data task.
- **Reusable components before repetition.** Isolate repeated patterns into small shared components/modules and reuse them everywhere. UI controls come from the shared design system (`../gpui-design-system/crates/components`: `TextField`, `ComboBox`, `SegmentedControl`, `ListBox`, `NumericControl`, `Dialog`, `CommandPalette`, `Theme::button`/`switch`, …); apply the same instinct outside the UI (registry ops, pipeline steps, persistence helpers, config). When a new use case needs a variant, extend the shared component in the design system instead of hand-rolling a near-identical one in Studio, and keep similar use cases looking and behaving the same. A new control for a case an existing control already covers is a code smell.
- **No perl for code edits.** Never use `perl -i` (or similar opaque one-liner rewrites) to modify source files — they are hard to review and have corrupted code before. Make edits with the file editing tools (`edit_file`/`write_file`), which produce reviewable diffs; `cargo fmt` is the only bulk formatter allowed.
- **No ML frameworks in Rust.** Rust must never depend on or know about PyTorch, Transformers, or any other ML framework. Inference is the native `stable-diffusion.cpp` process; Rust is only its HTTP client.
- **Text and vision are capability-routed providers.** There is no `[llm]` section: the planner (`Capability::Text`) and vision (`Capability::Vision`) are declared in the shared `clipforge.toml` (`paths::clipforge_config_path()`, `$SVS_AI_CONFIG` overrides) so a key stays out of git. An `openai` provider is an external service the app never starts (the default); a `llamacpp` provider is a managed native `llama-server` the app starts on demand. Weights are never bundled. `crates/studio-core/src/llm/` is the client only; the planner vocabulary (typed operations + pipelines) lives in `crates/studio-core/src/registry/`; LLM steps are soft dependencies that degrade to manual input, never hard-fail.
- **UI and UX standard:** read `DESIGN.md` before changing user-facing flows, controls, feedback or styling. It is the central design reference, audit and remediation record; update its acceptance evidence when fixes land.
- **UI styling** comes from the design system's `Theme`: colors, metrics and type scales are its tokens, never literal values chosen in Studio. Studio follows the persisted appearance (`ThemePreference`), resolving System from the window.
- **State mutations** go through `AppState` methods in `apps/studio/src/state/mod.rs`, which maintain the registry invariants documented in `docs/data-model.md`. Never mutate `Project` fields directly from UI code.
- **Build artifacts and tool temporary files stay inside the checkout.** Keep `.cargo/config.toml`: it selects gitignored `target/` and sets `TMPDIR`/`TMP`/`TEMP` to ignored `.cargo/tmp/` for Cargo, tests and inherited subprocesses. Keep that temporary directory's tracked `.gitignore` so it exists on a fresh clone. Do not override these paths with external locations. Use a checkout path without `:` for macOS/Linux library-path compatibility; do not reinstate an external target directory as a workaround. Older handoffs documenting an external build target do not override this policy.
- **Build Studio with cargo.** `cargo build -p storyfella-studio` produces a runnable binary at `target/<profile>/storyfella-studio`; `cargo run -p storyfella-player` launches the Player. `cargo xtask build`/`run`/`plan` (and `check`/`test`/`fmt`/`clippy`) wrap these from the workspace root — prefer them over retyping the invocations.
- **Validation logs and scratch files stay in the repo** under `.svs-e2e/` (gitignored). When live-testing the GUI, write its stdout/logs to `.svs-e2e/logs/` — never to `/tmp` or the repo root. When killing a stale app instance, use a bracket-quoted pattern so `pgrep`/`pkill` do not match their own command line (e.g. `pkill -f "storyfella-studi[o]"`).
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

Studio owns Writer, project and catalog editing, preview, and writing workflows.
Atelier owns artwork creation and publication. The GUI-free shared project,
catalog and artwork contracts remain consumed by both repositories. `sf-cli`
provides story checking, compilation, tracing and MCP authoring, including the
bridge to an open Studio. The plan driver accepts metadata operations only.

# Validation

Always verify changes that touch Rust code:

```sh
cargo check --workspace --all-targets
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

That is `cargo xtask check` (+ `test`), which wraps the same commands from the workspace root. Do not modify `Cargo.toml` features or add dependencies without a strong reason — dependencies live in `[workspace.dependencies]` at the root and are added there once, then referenced from a member with `name.workspace = true`.

The workspace is clippy-clean with warnings denied, and CI keeps it so. Fix
what clippy reports; do not silence it to pass validation.

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

# GPUI in Studio

- **One owner for app state.** `ui::Studio` owns `AppState` on the UI thread. Views read and change it directly; every change to the story ends in `Studio::changed`, which remembers the project, autosaves, rechecks compilation and redraws.
- **Automation goes through the mailbox.** The plan driver and the live MCP bridge run on their own threads with their own tokio runtimes, because GPUI's executors are not tokio. They reach the window with `host::Mailbox::call`, which runs a closure against `host::Driven` on the UI thread. Keep `Driven` toolkit-free so the driver's tests run against a plain fake.
- **Slow work leaves the UI thread.** Operations, compile checks, project scans and health checks run on `background_spawn`; their results come back through `WeakEntity::update`. Anything needing tokio (reqwest, timers) builds its own runtime inside that task.
- **Tests:** window behaviour uses `#[gpui::test]` with `TestAppContext`/`VisualTestContext` (see `apps/studio/src/ui/tests.rs`). A test module that glob-imports `gpui_kit::prelude::*` shadows `#[test]` with GPUI's macro; import only what the test needs.

## Scope and validation

Before changing or removing an integration or platform path, verify the intended scope and its existing callers. A dependency missing on the current machine is not evidence that support is obsolete. Prefer the smallest complete change, focused tool output and checks that address a specific uncertainty.

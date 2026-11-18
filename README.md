# Storyfella

Storyfella is the workspace for story authoring and playback. The existing
Dioxus application is **Storyfella Studio**; `sf-cli` provides story tools and an MCP server. Writer and Assets share persistent scene/beat associations
and authoring tools with the catalog-backed LSP. Language, compiler and story
runtime are implemented; Studio preview and the standalone visual Dioxus Player
share deterministic execution, exact artwork bindings and controls. See
[workspace boundaries](docs/architecture.md#storyfella-workspace-boundaries).

```text
storyfella/
├── apps/studio/       # storyfella-studio (Dioxus)
├── apps/cli/          # sf-cli: story tools and MCP server
├── apps/player/       # storyfella-player (visual Dioxus desktop host)
├── crates/studio-core/ # storyfella-studio-core (no GUI)
├── crates/language/    # shared lexer, parser, AST and analysis
├── crates/compiler/    # versioned story plans, linking and flow validation
├── crates/lsp/         # stdio editor services
├── crates/runtime/     # deterministic execution and verified saved state
├── crates/playback-ui/ # shared visual presentation and controls
└── xtask/            # developer tasks
```

Clipforge and FineWineTTS remain separate sibling repositories. Storyfella
consumes `../../zerune_labs/agentkit/crates/agentkit-ai` for provider resolution,
native and hosted inference, provisioning and shared server controls
(`native-inference` feature). Inference requires no Clipforge checkout or app.

Atelier, the GPUI drawing and image-generation app, lives in its own
repository next to this one (`../storyfella-canvas`) and uses `storyfella-studio-core`
by path. See [Atelier](../storyfella-canvas/README.md) and
[the shared AI boundary](../storyfella-canvas/docs/atelier.md).

Check or compile `.sf` stories without loading models or opening a project DB:

```sh
cargo run -p sf-cli -- story check examples/harbor.sf
cargo run -p sf-cli -- story compile examples/harbor.sf
```

See [the language contract](docs/language.md) and [compiler contract](docs/compiler.md).
Asset linking and unsupported directions remain explicit readiness requirements;
use `--require-ready` to make either command fail when they are unmet.

Studio edits [authoritative `.sf` files](docs/data-model.md) beside each project
database. Projects → Open accepts `.sf` or `.db`; Writer offers Save copy and
Reload for resolving external edits.

Try basic playback with procedural fixture images, without inference services:

```sh
cargo run -p storyfella-studio-core --example playback_fixture
cargo run -p storyfella-player -- .svs-e2e/inputs/playback/lantern.sf
cargo xtask plan config/plans/playback.yaml --mode guide --log
```

The fixture stays under `.svs-e2e/`. [Playback documentation](docs/ROADMAP.md#story-execution-and-playback-reference)
defines open inputs, fixed snapshots, keyboard controls and save/restore.
Voice, expressive media and portable packaging/export remain later work.

Storyfella Studio is a Dioxus desktop application for writing, arranging and
previewing visual stories. It owns authoritative source editing, catalog
definitions and saved artwork previews. Atelier owns image creation and editing;
both apps use the same portable project. `sf-cli` provides story checking, compilation,
playtesting and project-scoped MCP access. Source order drives scene/beat
associations and playback.

Studio runs on desktop. Its writing assistance uses Agentkit workflows and a
configured text provider. Image creation and editing belong to Atelier.

## Product surface

[DESIGN.md](DESIGN.md) defines Studio's shared UI/UX rules and tracks the open
findings from the app-wide usability audit.

- **Projects** creates a fresh story and reopens paired `.sf`/`.db` projects.
- **Writer** preserves exact drafts, reconciles valid scene/beat declarations with
  Assets identities, and supplies precise completion, navigation, diagnostics,
  resolved rename and conservative formatting.
- **Preview / Player** share still artwork, dialogue/thought/narration, choices,
  manual advancement, pause, restart, scene jump and verified save/restore.
- **Assets** browses catalog definitions, variations, saved images and read-only
  revision history. Names/descriptions stay editable as story requirements;
  undo/redo coordinates source and catalog changes.
- **Open in Atelier** saves the current story and opens the intended asset with
  its story context. Atelier owns image generation, editing and visual settings.
  Ordinary linked Save updates the artwork in Studio automatically.
- **Variations** are first-class catalog identities grouped under their base.
  Studio can define them without generating images; Atelier makes their artwork.
- **Settings** configures appearance, text and vision providers, and storage.
  Provider credentials and model setup are managed in Clipforge.

In Models, **Use a separate Vision LLM** reveals Vision's provider settings.
Turn it off to follow the Text LLM's provider, model, URL and key; that model must
accept images. Turning it back on restores the separate selection. Appearance
and this preference persist in Studio's app config; shared provider credentials
stay in the inference registry.

The shell deliberately uses a small reusable component vocabulary, low-contrast boundaries, and progressive disclosure instead of a dense card dashboard. Styling is primarily Tailwind CSS 4 utility classes in RSX, with a small shared CSS layer for global browser behavior and form controls.

### Current creative workflow

1. Write the story in Studio. Source declarations and the New catalog entry form
   define characters, places, props and illustrations without requiring images.
2. Open an item or scene in Atelier to create or edit its artwork.
3. Save the linked canvas normally. Studio refreshes its saved preview and asset
   availability; existing compiled Preview bindings retain their exact revision.
4. Continue writing and previewing in Studio. Reopen Atelier whenever artwork
   needs revision; saved history remains available without image-editing controls.

## Project layout

```text
Cargo.toml            Virtual workspace manifest ([workspace] members only)
.cargo/config.toml    checkout-local build output and tool temporary directories
crates/studio-core/          storyfella-studio-core — the domain layer (no GUI dependency):
  src/registry/         Artifact registry, authoring operations and guide parser
  src/persistence/      SQLite project store (versioned migrations)
  src/story.rs          Exact .sf source, disk snapshots and save conflict checks
  src/authoring.rs      Source associations and shared asset catalog
  src/playback.rs       Snapshot compilation and pinned resource checks
  src/planner.rs        Selected text-provider binding for the writing workflow
  src/runtime/          Text and vision provider health
  src/errors.rs         Error-code registry
  src/config.rs         Configuration loading
  src/paths.rs          Project-root path resolution
apps/studio/         storyfella-studio — the Dioxus application:
  src/app/              Dioxus root
  src/ui/               Desktop UI components (canvas, writer, projects, settings)
  src/state/            AppState: registry, selection, undo/redo
  src/commands.rs       Shared action language (slash parse → execute)
  src/driver/           YAML plan driver (Guide/Check modes)
  Dioxus.toml, tailwind.css, assets/, package.json
apps/cli/             sf-cli — story commands and project-scoped MCP
apps/player/          storyfella-player — standalone visual Dioxus desktop host
crates/language/      Shared front end and authoring services
crates/compiler/      Versioned plans, linking and flow validation
crates/lsp/           Catalog-backed stdio editor tooling
crates/runtime/       Independent deterministic story execution
crates/playback-ui/   Shared Dioxus presentation and controls
xtask/                cargo xtask — the developer task runner (build, run, bundle,
                      check, test); not part of any shipped artifact
config/               plans/*.yaml (+ clipforge.example.toml) (data, at the workspace root)
assets/generated/     Optional dev asset location via a paths.asset_dir override; the
                      default is the platform data dir's assets/
docs/ cache/ .svs-e2e/
```

`crates/studio-core` never depends on the GUI; `apps/studio` and `apps/cli` both depend on `storyfella-studio-core`, and the desktop crate names `storyfella_studio_core::…` explicitly rather than re-exporting it.

Configuration is split between Studio's application settings and the shared
`clipforge.toml` provider registry. Text and vision providers are selected in
Studio; Clipforge manages provider setup. Artwork is created in Atelier.

## Documentation

In-depth material lives in the [documentation index](docs/README.md): current architecture, development instructions, language/runtime contracts, persistence and Atelier behavior. Plans and agreed changes live in [ROADMAP.md](docs/ROADMAP.md).

## Prerequisites

- Rust stable
- The current stable Dioxus CLI (`dx`)
- Node.js/npm (for the pinned Tailwind CSS and CodeMirror diff-editor toolchain)
- An LLM endpoint (llama.cpp `llama-server` or LM Studio), running separately when LLM features are exercised
- Linux desktop builds: the system packages required by Dioxus/WebKitGTK

## Run the desktop application

Install the frontend toolchain once (from `apps/studio`, which holds `package.json`):

```bash
cd apps/studio
npm ci
```

The `tailwind.css` file in `apps/studio` selects Tailwind CSS 4 and lets `dx` automatically compile `assets/tailwind.css`. Most component styling is expressed as Tailwind utility classes in RSX. To build or watch CSS independently, use `npm run css:build` or `npm run css:watch`. Studio's Cargo build script also bundles the local CodeMirror diff editor with esbuild; see [the editor toolchain](docs/development.md).

```bash
cd apps/studio && dx serve --platform desktop
```

The workspace also carries a task runner for the common commands — `cargo xtask
run` (build and launch), `cargo xtask bundle` (release AppImage into `dist/`),
`cargo xtask test`, and more; see [`docs/development.md`](docs/development.md).

For a compile-only check:

```bash
cargo check --workspace --all-targets
```

This checkout keeps build artifacts in gitignored `target/` through `.cargo/config.toml`. Cargo, tests and inherited subprocesses use ignored `.cargo/tmp/` for temporary files. Keep task scratch files and desktop-plan projects under `.svs-e2e/`. Use a checkout path without `:`, which macOS and Linux treat as a separator in dynamic-library path lists.

Configuration is split between the platform-local `studio.toml` and the shared
`clipforge.toml` provider registry. Configure providers in Clipforge, then select
the available text and vision providers in Studio Settings.

## Roadmap

Plans, TODOs and completion status live in the
[central tracker](docs/ROADMAP.md#0-current-direction-and-actionable-backlog).
The current planning focus is Studio for DSL writing, Atelier for visual creation,
and a shared project workflow. Existing plans are linked there for reconciliation
with that direction.

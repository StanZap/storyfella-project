# Architecture

## Storyfella workspace boundaries

Studio, Studio core, CLI, the visual Player, xtask, shared playback UI,
language, compiler, runtime and stdio LSP are workspace members. The Writer
and LSP consume the same analysis. See [the language contract](language.md)
and [central tracker](ROADMAP.md#0-current-direction-and-actionable-backlog) for implemented scope and next milestones.

| Component | Responsibility | Dependency boundary |
| --- | --- | --- |
| `apps/studio` / `storyfella-studio` | Dioxus authoring UI, project state, creative workflows and snapshot preview | Consumes GUI-free core and shared playback UI/runtime |
| `crates/studio-core` / `storyfella-studio-core` | Artifact registry, operations, persistence, config, creative inference adapters | No GUI dependencies; consumes sibling `agentkit-ai` |
| `apps/cli` / `sf-cli` | Story checking, compilation, tracing and MCP authoring | Consumes story crates and Studio core |
| `apps/player` / `storyfella-player` | Standalone visual GPUI desktop playback | Shared core playback model, stage composition and runtime through `playback-panel`; no Studio state dependency |
| `crates/playback-panel` / `storyfella-playback-panel` | GPUI playback panel and stage | Draws Studio core's stage composition with the shared design system; no dependency on either app |
| `crates/playback-ui` / `storyfella-playback-ui` | Studio's Dioxus preview panel | Draws the same stage composition until Studio moves to GPUI |
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
The runtime executes that representation with bounded host-driven execution.

## Studio and Canvas

Studio owns story source, catalog definitions, project persistence, writing
review and preview. Atelier lives in the sibling `storyfella-canvas` repository
and owns creation, editing and publication of artwork in shared projects.
Project and catalog contracts are shared through `storyfella-studio-core`.

Studio's Writing help builds an Agentkit workflow with a language-model step.
Studio binds its selected text provider and retains workflow runs and events in a
project-local SQLite store. The plan driver can assess saved images using the
selected vision provider. Clipforge manages shared provider configuration and
local inference services; Studio uses their public Agentkit interfaces.

## Process boundaries

`sf-cli story` operates on source and compiled story plans. `sf-cli mcp` exposes
project-scoped story tools over stdio. With `--studio-channel`, it connects to
a running Studio's private channel to edit its Writer draft and inspect or
capture Preview. Atelier exposes its own MCP server for project artwork.
See [agentic production](agentic-vn-production.md) for connection and ownership
rules.

## Directory resolution

Application preferences are stored in the platform-local `studio.toml`; the
shared `clipforge.toml` contains provider configuration. Studio persists its
provider and model selections separately and reads provider changes from the shared
registry. Model choices come from the provider's model list; the provider may
not report each model's image capabilities. Managed native providers expose their
configured model. `SVS_USER_CONFIG` and `SVS_AI_CONFIG` override these paths.

The resource root is `SVS_HOME` when set, otherwise an ancestor containing
`config/plans`. Portable project folders hold story source, the project
database and artwork. See [shared projects](shared-project.md) and
[development](development.md).

## Shared local inference service

Agentkit owns admission, native worker supervision and inference protocols.
Studio supplies the shared state path and holds a lease while a managed text
provider handles a writing request. A worker's allocated port can differ from
the configured provider address. Clipforge manages provider configuration
and service policy.

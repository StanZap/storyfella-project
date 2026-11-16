# Storyfella documentation

[ROADMAP.md](ROADMAP.md) records agreed direction, TODOs and dependencies.
The references below describe the implementation that exists today; roadmap
features are not implemented merely because they are agreed.

| Document | Purpose |
| --- | --- |
| [ROADMAP.md](ROADMAP.md) | Central plan and TODO tracker. |
| [architecture.md](architecture.md) | Components, operation pipelines, inference boundaries and server lifecycle. |
| [development.md](development.md) | Setup, build/run/test commands, CLI usage, configuration and troubleshooting. |
| [language.md](language.md) | Authoritative DSL syntax, grammar, language services and LSP setup. |
| [compiler.md](compiler.md) | Flow lowering, immutable bindings, source maps and readiness. |
| [playback reference](ROADMAP.md#story-execution-and-playback-reference) | Execution, save/restore, Studio preview and standalone Player. |
| [data-model.md](data-model.md) | Registry invariants, project files, persistence and source conflict handling. |
| [atelier.md](../../storyfella-canvas/docs/atelier.md) | Current canvas, image tools, references, project recovery and model setup. |

Read [AGENTS.md](../AGENTS.md) before development and [DESIGN.md](../DESIGN.md)
before changing UI behavior or styling. Keep plans in the roadmap, current
contracts in these references, and executable acceptance scenarios in
`config/plans/` and tests.

The lighthouse PNG under `docs/poc/image-generation/latest/` is retained solely
because Studio's sample-project code uses it as a fixture. Historical experiment
notes, generated reports and interface prototypes are no longer documentation.

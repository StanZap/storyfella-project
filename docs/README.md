# Storyfella Studio — documentation

This directory documents how the application is built, how its pieces talk to
each other, and how to develop it. It is the companion to the top-level
[`README.md`](../README.md), which covers product surface and prerequisites.

Proof-of-concept notes live in [`docs/poc/`](poc/README.md) and record what was
learned while validating each ML backend; they are not instructions for the
current product code.

## Guide to this directory

| Document | Purpose |
| --- | --- |
| [`DESIGN.md`](../DESIGN.md) | Central Studio UI/UX rules, visual/accessibility standards, prioritized audit and acceptance checks. |
| [`studio-power-user-guide.md`](studio-power-user-guide.md) | Current workflows, keyboard/AI/CLI paths, multi-step limitations and a prioritized UX assessment. |
| [`architecture.md`](architecture.md) | System boundaries, the Rust → `sd-server` (`sdcpp`) contract (the client lives in the shared `clipforge-inference` crate), process model, and the creative data flow from prompt to revision. |
| [`development.md`](development.md) | Prerequisites, build/run/test commands (including end-to-end plans), configuration, and troubleshooting. |
| [`api-slice-1.md`](api-slice-1.md) | The artifact registry, the typed operation/pipeline layer, and the `svs` CLI — the vocabulary shared by the CLI and the canvas. |
| [`runtime-lifecycle.md`](runtime-lifecycle.md) | How the app launches and supervises the native Qwen-Image-2.1 `sd-server` (through the shared `clipforge-inference` crate), plus model provisioning and residency. |
| [`data-model.md`](data-model.md) | The artifact-registry domain model, the persistence format, and known gaps. |
| [`language.md`](language.md) | Initial `.sf` syntax, shared front-end API and stdio LSP setup/limits. |
| [`compiler.md`](compiler.md) | Story flow, versioned plan/source maps, immutable asset linking, readiness and story CLI commands. |
| [`story-files.md`](story-files.md) | Authoritative `.sf` files, Studio open/save, disk conflicts and SQLite draft migration. |
| [`language-plan.md`](language-plan.md) | Language/compiler/runtime implementation sequence and acceptance gates. |
| [`ROADMAP.md`](ROADMAP.md) | Product design and the feature/status tracker (§12): every planned feature with its status. |

## Reading order

For user-facing work, read [`DESIGN.md`](../DESIGN.md) first.

1. Start with [`architecture.md`](architecture.md) to see the moving parts and
   the Rust → `sd-server` HTTP boundary.
2. Read [`runtime-lifecycle.md`](runtime-lifecycle.md) to understand cold
   startup and residency guarantees.
3. Use [`development.md`](development.md) whenever you set up a machine or need
   a command reference.
4. Read [`data-model.md`](data-model.md) before touching project persistence or
   registry invariants.
5. Read [`api-slice-1.md`](api-slice-1.md) before changing an operation, a
   pipeline step, or the slash vocabulary — the CLI and the canvas compile to
   the same typed ops, so a change there lands in both surfaces.

`AGENTS.md` at the repository root is the other half of this: it records the
measured behaviour of the generation backend — the settings that matter and the
failures they cause. Read it before tuning a generation parameter.

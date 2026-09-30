# Smart Visual Sequencer — documentation

This directory documents how the application is built, how its pieces talk to
each other, and how to develop it. It is the companion to the top-level
[`README.md`](../README.md), which covers product surface and prerequisites.

Proof-of-concept notes live in [`docs/poc/`](poc/README.md) and record what was
learned while validating each ML backend; they are not instructions for the
current product code.

## Guide to this directory

| Document | Purpose |
| --- | --- |
| [`architecture.md`](architecture.md) | System boundaries, process model, and the creative data flow from prompt to revision. |
| [`development.md`](development.md) | Prerequisites, build/run/test commands (including end-to-end plans), configuration, and troubleshooting. |
| [`api-slice-1.md`](api-slice-1.md) | The artifact registry, the typed operation/pipeline layer, and the `svs` CLI — the vocabulary shared by the CLI and the canvas. |
| [`http-api.md`](http-api.md) | The Python vision runtime's HTTP contract: endpoints, schemas, jobs, and error behavior. |
| [`runtime-lifecycle.md`](runtime-lifecycle.md) | How Rust launches and supervises the Python runtime and the native Krea generation server, plus model provisioning. |
| [`data-model.md`](data-model.md) | The project/storyboard/timeline domain model, the persistence format, and known gaps. |
| [`ROADMAP.md`](ROADMAP.md) | Product design and the feature/status tracker (§12): every planned feature with its status. |

## Reading order

1. Start with [`architecture.md`](architecture.md) to see the moving parts and
   the explicit Rust/Python/HTTP boundary.
2. Follow the endpoint walkthrough in [`http-api.md`](http-api.md) while
   reading `apps/vision/runtime/service.py`.
3. Read [`runtime-lifecycle.md`](runtime-lifecycle.md) to understand cold
   startup and residency guarantees.
4. Use [`development.md`](development.md) whenever you set up a machine or need
   a command reference.
5. Read [`data-model.md`](data-model.md) before touching project persistence or
   storyboard/timeline invariants.
6. Read [`api-slice-1.md`](api-slice-1.md) before changing an operation, a
   pipeline step, or the slash vocabulary — the CLI and the canvas compile to
   the same typed ops, so a change there lands in both surfaces.

`AGENTS.md` at the repository root is the other half of this: it records the
measured behaviour of the ML backends — the settings that matter, the ones
that turned out to be no-ops, and the failures they cause. Read it before
tuning a generation parameter.

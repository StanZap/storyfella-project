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
| [`development.md`](development.md) | Prerequisites, build/run/test commands, configuration, and troubleshooting. |
| [`http-api.md`](http-api.md) | The Python vision runtime's HTTP contract: endpoints, schemas, jobs, and error behavior. |
| [`runtime-lifecycle.md`](runtime-lifecycle.md) | How Rust launches and supervises the Python runtime and the native Krea generation server, plus model provisioning. |
| [`data-model.md`](data-model.md) | The project/storyboard/timeline domain model, the persistence format, and known gaps. |
| [`artifact-canvas.md`](artifact-canvas.md) | Forward-looking product design: artifact registry, creation canvas, typed operations, and SQLite persistence. |

## Reading order

1. Start with [`architecture.md`](architecture.md) to see the moving parts and
   the explicit Rust/Python/HTTP boundary.
2. Follow the endpoint walkthrough in [`http-api.md`](http-api.md) while
   reading `python/runtime/service.py`.
3. Read [`runtime-lifecycle.md`](runtime-lifecycle.md) to understand cold
   startup and residency guarantees.
4. Use [`development.md`](development.md) whenever you set up a machine or need
   a command reference.
5. Read [`data-model.md`](data-model.md) before touching project persistence or
   storyboard/timeline invariants.

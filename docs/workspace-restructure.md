# Storyfella restructuring handoff

Completed on 2026-10-05. The reviewed starting tree was clean with no untracked
source files, so checkpoint `bce62a9` is intentionally empty. The repository
directory was moved intact from `smart-visual-sequencer` to `storyfella`,
preserving `.git`, history, ignored local models, caches and other local files.

The workspace now contains `apps/studio` (`storyfella-studio`), `apps/cli`
(`svs-cli`), `apps/player` (`storyfella-player`), `crates/studio-core`
(`storyfella-studio-core`) and `xtask`. Package imports, task paths, Dioxus
identity, generated-CSS ignore rules and documentation match this layout.
The CLI retains its binaries, commands, settings and data identities.

Studio remains in Dioxus; Player is a dependency-free executable that prints
a placeholder message. The future language/compiler/runtime/LSP crates are
documented in [architecture.md](architecture.md#storyfella-workspace-boundaries)
and have not been added. DSL/LSP implementation begins after this handoff.
Clipforge and FineWineTTS remain separate repositories; the sibling
`../clipforge/crates/inference` dependency is unchanged.

Validation on macOS with Rust 1.95.0 and dx 0.7.10:

- `cargo fmt --all -- --check`: passed.
- `cargo check --workspace --all-targets --locked`: passed.
- `cargo test --workspace --locked`: 231 passed, none failed.
- `cargo build -p svs-cli -p storyfella-player --locked`: passed.
- `cargo xtask build`: built the Studio `.app`, including Tailwind assets.
- `cargo xtask plan config/plans/smoke.yaml --mode check --log`: launched
  Studio and passed all 7 assertions without an inference backend.
- CLI help for all three binaries and Player launch: passed.
- CLI project creation, story/beat operations and log readback: passed;
  SQLite persistence verified across separate processes (2 artifacts).
- Rename diff, stale package/path scan and `git diff --check`: passed.

Validation also fixed two existing macOS assumptions: platform path tests now
apply Linux-specific suffix assertions only on Linux, and xtask finds dx's
macOS `Contents/MacOS` executable layout. A regression test covers executable
discovery. Runtime config/data resolution itself remains unchanged.

Logs and smoke databases are under ignored `.svs-e2e/`; generated stylesheets,
node_modules, model weights and secrets are excluded from Git.

Remaining limitations:

- Linux CI was added but has not run remotely. Unauthenticated access to the
  configured Clipforge GitHub repository returns 404; a private sibling needs
  the `CLIPFORGE_READ_TOKEN` Actions secret documented in development.md.
- The unchanged npm lockfile reports four high-severity dependency advisories.
  Dependency upgrades are outside this restructuring.
- A database storing absolute asset paths beneath the old repository directory
  may need path repair after the rename. No persisted paths were rewritten.
- Live inference and distribution packaging were not revalidated; the
  restructuring smoke tests exercise backend-free existing functionality.

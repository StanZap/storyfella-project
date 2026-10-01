# TODO — seamless local workflow

Working backlog, drafted 2026-09-30, refreshed 2026-10-01. This is a scratch list
of *pending* work, not the feature tracker: [`docs/ROADMAP.md`](docs/ROADMAP.md)
§12 remains authoritative for product status. When an item here lands, update §12
and delete it from this file.

The theme is one goal: **the GPU is for image generation, and the loop around it
should not require manual process juggling.**

## Recently landed (baseline)

- User-level config layer: `paths::user_config_path()` + deep-merge over
  `config/app.toml`. Settings pickers persist there; secrets stay out of git.
- **Text and vision are capability-routed providers.** The old §2 is done: `[llm]`
  is gone, and `Capability::Text`/`Vision` resolve from `providers.toml`, layered
  with a user providers overlay (`paths::user_providers_path()`,
  `$SVS_USER_PROVIDERS`). The planner reads `planner_config()`, `svs eval` and the
  health probe read `vision_config()`, so a remote text-only planner no longer
  blinds vision. Both capabilities are optional and degrade to `None` instead of
  failing config load. This also closed all of the old §9: `api_key_env` exists,
  text/vision resolve from providers (`Provider` gained `api_key`), and the
  providers file is layered per-user.
- **One server supervisor.** `clipforge-inference`'s `server::Server` owns the
  spawn/readiness/stop lifecycle for both `sd-server` and the new managed
  `llama-server` (`llama` module; `ProviderKind::LlamaCpp` starts one on demand).

---

## 1. GPU/RAM residency manager (ComfyUI-style) — the headline item

**Goal.** Let models live in system RAM and move to the GPU only when needed, so
several models can share one card (or one model can run on a card too small for
it) without hand-stopping processes — the behaviour ComfyUI gives you.

**What the backend already gives us** (verified against the checked-out
`stable-diffusion.cpp` revision, `examples/common/common.cpp`):

- `--offload-to-cpu` — *"place the weights in RAM to save VRAM, and automatically
  load them into VRAM when needed"*. This is the ComfyUI-style paging, per
  process.
- `--max-vram <GiB>` (single value or per-device, e.g. `cuda0=6,vulkan0=4`; `0` =
  live free VRAM, negative reserves) — budget for managed weights and runner
  buffers during **automatic graph-cut** execution.
- `--backend <component>=<device>` / `--params-backend` — pin individual parts
  (`vae=cpu`, `te=cpu`, `params-backend diffusion=disk`). The old
  `--clip-on-cpu` / `--vae-on-cpu` flags are deprecated aliases of this.
- `--mmap`, `--eager-load`, `--split-mode`, `--rpc-servers` round out the set.

**The crux.** Every one of these is a **launch-time** flag. `examples/server/api.md`
and `examples/server/routes_sdcpp.cpp` expose no unload/reload/free endpoint (only
`vae_tiling_params`), so changing residency today means **restarting `sd-server`**
— which pays the multi-minute weight load again. Any manager we build must work
within that, or push upstream for an unload call.

**The other GPU tenant.** A `llamacpp` provider is now a managed `llama-server`
the app can start on demand (what used to be an absolute "the app never starts the
LLM" rule is now a per-provider choice), with its own offload/CPU mode (`-ngl 0`).
If the planner is off-GPU — a remote `openai` provider, the default — this is
moot; otherwise it is the second resident competing for VRAM and the manager must
budget for both.

**Options to evaluate (pick one and scope it):**

- **A. Launch profiles.** Two (or three) named profiles for `sd-server` —
  `resident` (today: fastest, ≈15.9 GB weights) vs `lowvram`
  (`--offload-to-cpu --max-vram N`) — chosen in Settings / `providers.toml`;
  switching restarts. Simplest real win.
- **B. Auto budget.** Probe free VRAM at launch and derive `--max-vram`
  automatically, so a shared card "just works".
- **C. Swap coordinator.** Stop the local LLM before a generation, restart after —
  as `xtask` tooling, or inside the app now that `llama-server` is a managed
  provider. Only needed if the planner stays local.
- **D. Upstream an unload/reload endpoint** so residency changes without a
  restart. Long pole; would make A/B/C unnecessary.

**Open questions to answer first (measure, don't guess):**

- What does `--offload-to-cpu` actually cost at our canvases — t2i 1024² and
  reference edit 1024×1536? (`docs/poc/native-generation/README.md` says the
  checked-in config deliberately avoids CPU offload to favour latency; quantify
  it with the existing perf harness, record in `.svs-e2e/logs/`.)
- Is `--max-vram` compatible with `--diffusion-fa` and the
  `qwen_image_2_1_prefix_cache_type=q8_0` prefix cache?
- Does restarting to switch modes truly cost a full reload every time?

**Plumbing note.** `Profile::launch_args` builds a fixed flag list
(`../clipforge/crates/inference/src/sd/profile.rs`), and `Provider` still has **no
generic args passthrough** (only `executable`/`model_path`/`mmproj`, added for the
llama backend); a launch profile needs either new fields on the provider or an
args passthrough — a change in the sibling `clipforge-inference` crate. The shared
`server::Server` supervisor is where a residency mode would be applied.

## 3. Hermetic runs for tests and the plan driver

We added `$SVS_USER_CONFIG` (and, with the provider split, `$SVS_USER_PROVIDERS`),
but nothing sets them, so `--plan` runs and e2e pick up the developer's real
`~/.config/smartvisualsequencer/{app,providers}.toml`. Point the driver and
`cargo xtask` at scratch files under `.svs-e2e/` so check runs are deterministic
and personal config stays personal. Small.

## 4. Surface provisioning and first run

`model_setup` is CLI-only and shows no progress in the app (documented gap in
`docs/runtime-lifecycle.md`). Reuse `ModelStore`'s `DownloadProgress` callback for
a first-run "Prepare image model" action with byte counts. Also confirm the
manifest `remote_path`s against the live Hugging Face repo — the docs flag them
as best-effort/unverified.

## 5. Prewarm toggle

Optional Settings → Image generation toggle to start `sd-server` on project open,
so the first generation isn't a multi-minute wait. Must be **off** while any
local model shares the card — pairs with §1.

## 6. Graceful shutdown

App exit relies on `kill_on_drop` and does not cleanly stop the child (documented
gap). Stop `sd-server` with a bounded timeout on exit so a mid-generation quit
doesn't orphan or half-kill it. The shared `server::Server` now owns the child, so
this is a single place to add a bounded `stop()` on exit.

## 7. Take over a stale/foreign server instead of `pkill`

A server on the provider's port that we don't own can only be cleared by hand
(`pkill -f "sd-server --diffusion"`). Record the child PID (and a small marker
file) so the runtime can identify, adopt, or kill it automatically, and give a
better message than a regex. Detection exists (`ServerError::Foreign` →
`ProfileMismatch`); ownership does not.

## 8. Grounding/variant quality regression harness

Planner quality is model-sensitive (measured: `bonsai-27b` > `gemma-4-e4b` at the
`modify` subtraction). Build a fixed prompt+base-image suite driven by
`svs eval --expect`, logged to `.svs-e2e/logs/`, so "is DeepSeek as good as
bonsai at grounding?" is a command, not an afternoon. **Unblocked:** `svs eval`
now resolves its own `vision` provider, so the harness can run against a
vision-capable endpoint while the planner points anywhere.

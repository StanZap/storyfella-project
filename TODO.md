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

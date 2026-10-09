# TODO — seamless local workflow

Working backlog. Drafted 2026-09-30, refreshed 2026-10-01. This is a scratch list
of *pending* local-workflow work, not the feature tracker: [`docs/ROADMAP.md`](docs/ROADMAP.md)
§12 remains authoritative for product status. When an item here lands, update §12
and delete it from this file.

The theme is one goal: **the GPU is for image generation, and the loop around it
should not require manual process juggling.**

## Backlog

Empty as of 2026-10-01. Every item landed:

1. GPU/RAM residency manager — a `resident`/`lowvram` launch profile per image
   provider, chosen in Settings, with the measured tradeoff in
   `docs/runtime-lifecycle.md`.
3. Hermetic runs — `--plan` runs read/write scratch config under `.svs-e2e/`.
4. Surface provisioning — Settings → Services → Model files downloads
   with progress; the pinned manifest was verified against the live HF repo.
5. Prewarm toggle — start the server when a project opens.
6. Graceful shutdown — bounded stop at exit, including the check-mode exit.
7. Take over a foreign server — precise identification (executable + listen
   port, not a `pkill` pattern) and an in-app Take over action.
8. Grounding regression harness — `.svs-e2e/scripts/grounding_suite.py`, driven
   by `svs eval --expect`.

Add the next item here.

# Storyfella — central plan and TODO tracker

Start here for planned work. Keep new TODOs, priorities and completion status in
this document; link other documents for specifications and acceptance evidence.
When work lands, check its item and record the evidence here. Unchecked items
describe planned work, not existing functionality.

## 0. Current direction and actionable backlog

### Priority foundation: consume Agentkit for intelligence · 2026-10-28

Agentkit is the shared framework dependency for intelligence capabilities and
agentic workflows. Storyfella consumes its public APIs as an external framework;
application behavior stays in Storyfella. Extend Agentkit only for demonstrated
framework defects or missing reusable capabilities. Docgraph's integration is a
reference for application-owned workflows, tool adapters, run persistence and
streaming presentation, not domain code to copy into Storyfella.

- [x] **SF-PLAN-27 — Consolidate reusable inference in Agentkit.** Coordinate the
  migration of Clipforge's reusable inference layer into optional Agentkit
  capabilities: model provisioning/verification, managed native server lifecycle,
  deployment readiness and reuse, provider adapters and image generation/editing
  contracts. Reuse Agentkit's existing chat-provider contracts for running
  endpoints; managed local deployment is the additional lifecycle responsibility.
  Update Clipforge, Studio and Atelier consumers through public APIs, preserve
  existing configuration, credentials and model files, and remove superseded
  provider/client/server-management implementations as callers migrate. Done when
  consumers share one inference implementation, local Qwen/Krea and existing
  hosted routes still work, and Storyfella needs no installed/running Clipforge
  app for inference. Hosted-only consumers must not require local model setup.
  - [x] Extract native Qwen/Krea profiles, sdcpp client, LoRA discovery,
    verified model provisioning and process supervisor into optional
    `agentkit-ai::native` (`native-inference` feature). Update Studio, Atelier,
    CLI and Clipforge callers to use the public Agentkit types; remove the
    superseded source from Clipforge.
    Evidence (2026-10-29): Storyfella workspace tests and all-target checks;
    Clipforge `cargo xtask check`; Agentkit AI native-feature tests and public
    wire-contract test (source bytes, seed, steps, LoRAs, returned bytes); separate
    hosted-only build with native inference disabled. Live model generation is
    not verified by this extraction. The subsequent consolidation also fixes
    the previously recorded Gemini/HTTP lint findings; strict Agentkit Clippy passes.
    Logs: `.svs-e2e/migration/`.
  - [x] Consolidate deployment resolution, shared process controls/markers,
    llama lifecycle and hosted image adapters in Agentkit. Remove the
    `clipforge-inference` crate and update every application consumer. Keep
    config/state identities in the apps, preserving the existing TOML keys,
    credentials and model files. Studio's planner and Clipforge text/vision
    consumers reuse Agentkit's existing chat transport, including structured
    generation controls and tool-message replay. Hosted image models also bind
    through the existing ProviderFactory/ProviderRouter and credential resolver.
    Clipforge's media analyzer also uses that transport; its obsolete blocking
    HTTP client is removed. Atelier validates before server startup and loads
    selected owned models while preserving foreign servers. Native job polling
    respects its deadline even when a server accepts but never answers.
    Evidence (2026-10-29): Storyfella workspace tests, planner-host wire test and
    all-target checks; Clipforge `cargo xtask ci` including analyzer-host tests
    and both MCP transports; Agentkit native/core tests, hosted-only tests,
    strict Clippy and runtime/workflow/eval consumer checks. Shared HTTP tracing
    excludes endpoint credentials, queries, fragments and raw transport errors.
    Shared native registration (2026-10-30): Clipforge desktop startup publishes
    its effective native model deployments to the shared registry, preserving
    hosted defaults, keys and local weight overrides. Atelier can resolve the
    same server address and absolute launch paths even when the earlier config
    contained only hosted providers. Regression tests cover fresh and hosted-only
    configs and Atelier's generation/edit resolution; workspace checks/tests and
    strict Atelier/Agentkit Clippy pass. This is configuration validation, with
    no additional model download or image-generation run. The rebuilt Atelier
    UI was reloaded against the existing server and showed
    `Local server: running · qwen_image_2.1-Q8_0.gguf` without the missing-provider
    composer error.
  - [x] Complete live acceptance on a supported native inference machine and
    configured hosted routes: Qwen generation/editing, Krea generation, model
    switching/reuse, cancellation and saved output. Offline tests establish
    protocol preservation and application binding, not generated-image quality.
    Live evidence (2026-10-29, user-reported): Atelier image generation and
    editing both work through Replicate; project persistence also works.
    Native Qwen Metal acceptance (2026-10-29): **generation and reference editing
    passed on M3 Max**, using the packaged `sd-server` (`3f8527a`, GGML
    `89c4413f`) through Agentkit's public runtime/client. Both runs used 512×512,
    20 steps and seed 42; the generated red cup was successfully edited blue
    while retaining the scene. The opt-in `agentkit-ai/tests/native_metal.rs`
    test saved both PNGs and passed in 379.65 seconds; evidence is in Agentkit's
    `target/metal-live-retry.log` and `target/qwen-metal-live/`.
    Agentkit now provisions pinned Q8_0 GGUF diffusion/text weights and the F16
    vision projector on macOS, checks all four artifacts and passes
    `--llm_vision` for editing. The VAE is reused and existing INT8 files are
    preserved. Other hosts retain the existing INT8 defaults. This resolves the
    original wrong-format default and missing Metal INT8-kernel crash without
    modifying GGML. A separately reproduced interrupted-poll connection is
    handled by up to three transport reconnects to the same job, within caller
    deadlines; render submissions and HTTP failures are not retried. Native,
    hosted-only, persistence/projector, consumer workspace and retry-bound tests
    pass. Native Krea, model switching/reuse and cancellation were subsequently
    verified on macOS in the 2026-10-31 acceptance below.
    Sources: [Qwen Image 2.1](https://github.com/leejet/stable-diffusion.cpp/blob/master/docs/qwen_image_2.1.md),
    [INT8 backend support](https://github.com/leejet/stable-diffusion.cpp/blob/master/docs/int8_convrot.md).
  - [x] Complete macOS native lifecycle acceptance (2026-10-31, Apple M3 Max).
    Qwen generation at 512×512, 20 steps and seed 42 completed in 138.85 seconds;
    output saved and decoded from disk. Aborting a submitted generating job
    returned locally in 102 ms; the test drained continued GPU work and verified
    a subsequent request succeeded. Qwen → Krea → Qwen switching, same-model
    server reuse and saved output passed in 149.62 seconds using installed
    GGUF overrides (Krea IQ4_XS diffusion, Q4_K_M text encoder, Wan VAE).
    Switching used two Qwen steps and eight Krea steps as lifecycle smoke tests,
    not visual-quality evidence. Both Qwen outputs also survived Atelier result
    attachment and `.atelier` save/reopen with embedded bytes and provenance.
    The first switching run exposed Agentkit's Linux-only process ownership
    checks: macOS treated its own server as foreign and refused the switch.
    Agentkit now verifies macOS ownership through the kernel executable path;
    its regression failed before the fix and passes afterward, including
    preservation of a process with a mismatched executable. Owned server cleanup
    passed on the repeated real switching run.
    Isolated Atelier UI acceptance against the real resident Qwen server also
    passed: pause held queued work while a running request finished, queued and
    running Abort retained individual history, a later request completed, and
    both resulting artboards survived Save → close → reopen. UI runs used two
    steps and establish queue/persistence behavior only. No hosted request was
    submitted. Workspace checks/tests/format, strict changed-package Clippy,
    Agentkit native-feature tests/Clippy and its format check pass; the vendored
    GPUI dependency still emits existing Cocoa deprecation warnings.
    Reproduction: [live native acceptance](atelier.md#repeatable-live-native-acceptance).
    Evidence: `.svs-e2e/native-acceptance/` (logs, images and `.atelier` projects).
    Configured hosted-route cancellation/deadline acceptance is completed below;
    Linux NVIDIA packaging/acceptance remains the separate SF-PLAN-30.
  - [x] Complete configured hosted-route acceptance (2026-10-31). Replicate is
    the only configured hosted image provider; its `alibaba/qwen-image-3` route
    passed submitted-job abort (143 ms), polling deadline expiry (1.15 seconds
    including submission) and remote cancellation acknowledgement for both jobs.
    Subsequent generation (11.62 seconds) and reference editing (12.90 seconds)
    saved and decoded 1024×1024 outputs. The route advertises aspect-ratio sizing,
    so a square request does not promise an exact pixel size. Both images passed
    Atelier result attachment and `.atelier` save/reopen with embedded bytes,
    dimensions and provenance preserved; native output persistence was rechecked.
    The repeatable opt-in scenario spaces submissions to respect Replicate's
    limited-account rate limit. Cancellation acknowledgement does not establish
    when remote compute stops; this run establishes lifecycle and persistence,
    not visual quality or acceptance of unconfigured providers. Workspace tests,
    all-target checks, format and strict changed-package Clippy pass (existing
    vendored GPUI Cocoa deprecation warnings remain).
    Reproduction: [live hosted acceptance](atelier.md#repeatable-live-hosted-acceptance).
    Evidence: `.svs-e2e/hosted-acceptance/` (live/validation logs, images, results
    and `hosted-results.atelier`). SF-PLAN-27 is complete for the supported macOS
    native machine and configured hosted route.
- [ ] **SF-PLAN-28 — Integrate Agentkit workflows as an application foundation.**
  Depend on Agentkit's workflow/session APIs and supply Storyfella-owned context,
  domain tools, model bindings, durable run storage and UI event handling. Build
  Studio's rewrite/expand features (SF-PLAN-15–17) as workflows; make the same
  framework available to Atelier for conversational edits and asset iteration.
  Keep proposal application, source conflict checks, undo, canvas mutations and
  project saving in their owning apps. Done when a Studio writing workflow and
  an Atelier asset-iteration workflow run through public Agentkit contracts with
  responsive feedback, cancellation and no duplicated framework execution loop.
  - [x] First workflow integration (2026-10-31): consume the public runner and
    durable controller through GUI-free `studio-core::workflows`, with local
    transactional run/event storage. Writer offers scoped Rewrite/Expand,
    custom instructions, streamed proposals, original/replacement review,
    diagnostics, accept/reject/another version and cancellation. Source/path
    snapshots guard application, which uses normal reconciliation and one undo.
    Atelier's captured image queue runs image execution and saved-output steps
    through the same framework; progress, Abort and document attachment remain
    application-owned. See [workflow contracts and scope](workflows.md).
    Offline evidence: workspace tests, the real streaming-HTTP writing contract,
    stored run/event round trips, stalled-step cancellation, source conflict and
    one-edit undo tests, and Atelier's mocked workflow/save/reopen. Isolated
    macOS desktop checks verify review, Accept/Undo, running Cancel, concurrent
    source-edit protection, Reject and invalid-proposal diagnostics. No real
    model submission was made for this slice. Logs: `.svs-e2e/workflows/`.
    Review UX correction (2026-11-01): compact request composer, collapsed brief,
    full-height Suggestion/Original reading area and pinned acceptance actions.
    Existing warnings stay in Problems; new review notes are grouped and source
    errors still block acceptance. Failures use one contextual recovery notice
    and a neutral technical report. Native release checks cover long review,
    comparison, Accept/Undo, HTTP failure and startup configuration recovery.
    Writing validity gate: compile the complete proposed document before review,
    repair invalid/empty output with compiler feedback (two attempts maximum),
    and never publish invalid results. Offline tests cover repair, exhaustion,
    cancellation and invalid-draft preflight without model submission.
    Response-boundary correction: restore trimmed indentation/newlines before
    validation rather than spending repair calls on formatting. Seven recorded
    rejected outputs now validate on replay; genuine syntax failures remain
    guarded. Repair progress is numbered and diagnostics include source lines.
  - [x] Writing workspace recovery: project-local SQLite checkpoints retain
    request controls, captured scope, partial output, pending/manual review
    edits, comparison mode and unsaved accepted draft. Reopen restores text;
    interrupted calls require explicit retry. Source conflicts preserve the
    saved session until restore/discard is chosen. See [recovery behavior](workflows.md).
    Native acceptance also verifies reset-arrow/checkbox gutters, grouped
    Keep/Reset actions, selection recovery across close/reopen, and one-step
    undo for grouped acceptance. Evidence: `.svs-e2e/workflows/recovery-native-acceptance.md`;
    workspace checks/tests, strict Studio/core Clippy and release build pass.
    Manual-edit protection on retry: three-way source merging retains manual
    additions/deletions and reset choices across rewrites and reopening;
    conflicting or invalid combinations keep the current review. Checkpoint
    migration preserves existing work. Deterministic tests cover repeated
    retries, model-reproduced edits, CRLF/Unicode, conflicts and migration.
    Entire-file Rewrite/Expand is available alongside selection/beat/scene;
    deterministic workflow tests cover multiple scenes and exact CRLF/file
    boundaries. Writer tools, Problems counts and line position share one bar;
    toggled details expand below it. UI acceptance is assigned to the user.
  - [ ] Complete foundation acceptance: configured-provider quality checks,
    reactive session/context tools. Preserve existing workflow request history
    without automatically repeating paid calls after restart. SF-PLAN-15–17
    retain the broader writing-context and concurrent-edit acceptance criteria.
  Startup correction (2026-11-01): valid hosted-image defaults and text-only
  provider registries no longer block Studio authoring. Native image deployment
  is optional, prewarm skips it when absent, and native-only actions fail before
  server/download side effects. Settings exposes text/vision configuration and
  explains unavailable native controls. Shared defaults and keys are unchanged.
  Missing text-provider setup is now shown directly in Writing help, with an
  Open Settings action; Generate remains disabled until a text binding exists.
  The Settings provider selector also remains available for a single configured
  endpoint, enabling first-provider assignment. LM Studio text setup and a
  minimal completion were verified locally with the already-loaded Gemma model.
- [ ] **SF-PLAN-29 — Controlled model downloads through Agentkit.** Extend
  Agentkit's model provisioning with typed download state, progress events and
  explicit Pause, Resume and Cancel controls. Report current-file and aggregate
  bytes, totals and verification state; expose transfer rate/ETA only when known.
  Pausing releases the active network request and retains partial files; resuming
  continues safely from disk, including after an app restart. Cancellation stops
  transfer/verification promptly, retains partials by default and preserves
  completed verified files. Discarding partial files is a separate explicit
  action. Validate range responses and handle servers ignoring ranges,
  already-complete partial files, checksum failures and disk/write errors without
  accepting corrupt artifacts or silently appending incompatible data. Apps own
  the controls and presentation and consume this one framework implementation.
  Done when a real host shows progress and can pause/resume/cancel a model
  download without exiting, restart and resume retained work, and recover from
  failed transfers. Cover stalled connections, interruption, range handling,
  verification and cancellation with offline protocol tests. Current provisioning
  already reports per-file progress and can recover partial downloads on rerun;
  it has no explicit pause/resume/cancel API. Starting Qwen generation in Atelier
  does not automatically provision missing model weights.

**Delivery order:** start SF-PLAN-27 early, before expanding the former Clipforge
inference implementation. Establish the image contract in SF-PLAN-25 as part of
that migration, then add MOSS (SF-PLAN-24) and expose the capabilities in Atelier
(SF-PLAN-26). SF-PLAN-28 provides the foundation for writing/creative workflows.
Shared-project and catalog design can proceed independently; ordinary saving,
asset synchronization and authored playback remain normal application behavior.
Task IDs remain stable and are not an execution sequence.

| Owner | Target responsibility |
| --- | --- |
| Agentkit | Chat/vision/image capability contracts, provider adapters, optional native deployment management, workflow execution and reactive sessions. |
| Clipforge | Application, CLI and MCP surfaces consuming Agentkit for model setup, server management and experimentation; its clipping/conversion utilities remain app-owned. |
| Studio / Storyfella | Story context, DSL-aware tools, writing/narrative workflows, asset catalog and editor interactions. |
| Atelier | Canvas tools and application-defined creative workflows using Agentkit; normal save updates shared project assets. |
| Docgraph | Independent framework consumer able to adopt image capabilities for its own document features. |

The shared foundation enables more than individual chat calls. Studio workflows
can support character interviews, scene rehearsal, alternate-scene exploration
and continuity assistance. Atelier workflows can support conversational image
iteration, selected-area changes, reference-assisted edits, variation proposals
and consistency comparisons. These are feature directions, not mandatory
production stages or new image-approval/publishing gates. Storyfella characters,
Atelier layers and Docgraph nodes remain application concepts, not framework types.

**Future exploration — a living visual novel:** use Agentkit reactive sessions
to generate dialogue and narrative events during play while reusing existing
saved artwork. Begin with one location and two characters. Storyfella owns
authoritative world facts, character-specific knowledge, validated consequences,
output selection and staging; models propose events rather than directly
rewriting world state. Save actual generated events for resume/replay. This mode
requires runtime intelligence and latency investigation; ordinary authored
playback stays independent of inference, and neither mode requires Atelier.
Detailed scope and delivery priority remain to be decided after the foundation.

Inference execution and deployment management now live in Agentkit. Application
configuration, prompts, domain tools, result persistence and UI remain app-owned.
SF-PLAN-27 records the completed migration and native/hosted live acceptance.

### Studio for writing, Atelier for visual creation · 2026-10-28

The proposed direction is Studio for writing and previewing stories with the
Storyfella DSL, and Atelier for creating illustrations, character designs, places
and other visual assets. Writers can define story elements before artwork exists,
then open Atelier from the relevant story context. Atelier can also create new
characters, illustrations and variations independently; Studio can subsequently
reference those shared items. Both apps understand one story project, organized
as a folder of sources, metadata and assets.

Studio supports one authoritative `.sf` file plus a companion SQLite database.
New Studio projects use a shared folder with project-local artwork and an Atelier
working-document directory; existing standalone pairs remain readable and can be
explicitly copied into that layout. Atelier opens the shared manifest and also
retains standalone `.atelier` projects and `.sfart` exchange. Automatic handoff,
live synchronization and multi-file DSL compilation remain separate planned work.
See the [shared storage contract](shared-project.md).

The following checklist describes the shared-project workstream. Multi-file
language support can follow the initial shared-project workflow.

- [x] **SF-PLAN-01 — Define app responsibilities.** Inventory Studio's visual
  creation, generation, editing and asset-management surfaces and assign their
  destination in Atelier. Studio retains DSL authoring, story definitions,
  references, lightweight asset previews, readiness and playback. Done when each
  affected workflow has a destination and migration acceptance criteria.
  Evidence (2026-11-01): [workflow inventory and migration contract](app-responsibilities.md)
  maps current forms, Canvas/Writer/Settings, commands, history, CLI and Atelier
  surfaces to owners and explicit acceptance. Production controls remain until
  SF-PLAN-04–07 provide their working replacements.
- [x] **SF-PLAN-02 — Design a shared project folder.** Specify source files,
  metadata, artwork, Atelier working documents, manifest and database ownership.
  Use portable asset references and preserve existing source, revisions and
  associations. Done when a populated project moves to another folder and opens
  in both apps with its links intact.
  Evidence (2026-11-01): [versioned folder and ownership contract](shared-project.md),
  GUI-free create/adopt/validate/resource resolution, Studio create/open/recents
  and explicit nondestructive adoption, Atelier manifest opening and project-local
  Save. Storage tests remove the original location and move a populated project;
  exact source, identities, associations, variations and all image revisions
  reopen. Atelier's adapter test saves/moves/reopens an embedded visual document.
  Native UI acceptance is assigned to the user; live sync/publication remain 04–06.
- [x] **SF-PLAN-03 — Shared asset catalog.** Define stable project identities for
  characters, places, props, illustrations and variations, independently of
  artwork. DSL definitions create catalog items before images exist; Atelier
  can create items independently and make them available for Studio references.
  Preserve identity across rename, edits and later file moves. Distinguish story
  requirements from visual working documents and saved artwork. Done when an
  item created in either app appears in the other without manual duplication,
  and later artwork remains attached to the original identity.
  Evidence (2026-11-01): [shared catalog contract](shared-project.md#shared-asset-catalog)
  reuses registry UUIDs for characters/places/props, explicit variations and
  independent illustrations. Studio reconciles valid declarations without
  rendering and offers definition-first forms and source-aware rename/undo.
  Atelier's Assets panel creates/updates entries, imports saved image revisions
  and links artboards; source-used names are renamed in Studio. Schema v11
  preserves existing graphs and guards GUI saves against stale snapshots.
  Tests cover both directions, later artwork attachment, identity/variation
  retention, rename/undo, invalid drafts, stale writes, storage-worker events,
  artboard links and relocation. Native UI acceptance is assigned to the user.
  Reopen/Refresh supplies initial visibility; live sync/merging and normal
  canvas-save publication remain SF-PLAN-04–06.
- [ ] **SF-PLAN-04 — Synchronization and simultaneous use.** Synchronize catalog
  items, requirements, work state and saved artwork in both directions. Assign
  write ownership for story sources, visual documents and shared metadata;
  choose change notification, conflict handling and refresh behavior. An
  Atelier-created item becomes available in Studio without silently inserting or
  rewriting authored DSL. Coordinate any requested source edit with Studio's
  unsaved buffer. Done when creation from either app, renames, simultaneous edits,
  save failures and reopening preserve identities and work without silent
  overwrites or duplicate items.
- [ ] **SF-PLAN-05 — Open Atelier from story context.** Carry the project, stable
  element identity and relevant authored context from a character, place, prop
  or scene in Studio. Reuse linked Atelier documents where appropriate. Done when
  the intended work opens without manual export/import or repeated descriptions.
- [ ] **SF-PLAN-06 — Refresh Studio from Atelier saves.** Saving an asset-linked
  canvas persists its editable document and updates its image in the shared
  project. Refresh Studio's asset previews and availability automatically, with
  no export, import, approval or publishing step. Preserve exact saved revision
  bindings for compiled story snapshots. Done when a save updates the intended
  asset in Studio with both apps open and survives reopening.
- [ ] **SF-PLAN-07 — Remove visual creation workflows from Studio.** Once the
  shared workflow works, remove migrated controls and unused implementation.
  Review shared core and CLI callers before removing operations. Done when
  writing → Atelier → save → Studio preview works, existing project
  data is preserved and no creation controls lead to missing functionality.
- [ ] **SF-PLAN-08 — Support multiple DSL source files.** Specify the entry point,
  modules/imports, shared definitions, resolution, ordering and cycle diagnostics
  before implementing a project graph. Extend Studio, compiler, CLI and LSP
  together. Done when chapters and shared definitions occupy separate files with
  consistent diagnostics, navigation and identity-preserving rename.
  SF-PLAN-12 covers the related workspace tooling.
- [ ] **SF-PLAN-14 — Atelier production backlog.** Present the shared catalog as
  actionable character, variation, place, prop and illustration work, including
  items without images. Group variations under their character and show saved
  artwork versus requirements, story context and canvases with unsaved work.
  Derive basic availability from saved files; no mandatory approval statuses.
  Offer all requirements, used-in-story and missing-artwork filters. Open
  sketches, references, artboards and candidates without automatically generating
  images. Done when a project with four characters, seven requested variations
  each and eighteen illustrations exposes that work and its progress in Atelier,
  while Studio shows previews, readiness and Open in Atelier.

End-to-end acceptance: introduce a character while writing, open it in Atelier,
create and save its artwork, see it in Studio and preview the story. Repeat
with both apps open, unsaved story edits, a renamed element and a moved project.

### Agreed catalog, backlog and synchronization behavior · 2026-10-28

SF-PLAN-03, SF-PLAN-14 and SF-PLAN-04 form the shared-asset delivery group.
Creation may begin in either Studio or Atelier; the catalog is shared project
data, not an export of Studio's requirements. Catalog availability and use in the
story are separate: creating a character or variation in Atelier does not require
inserting it into a scene, and defining an unused character in Studio still makes
it visible in Atelier.

- Story requirements describe what is needed; Atelier owns drawing, generation,
  image editing, polishing, extraction, production prompts and settings. Studio
  can specify story-facing descriptions and variations but does not impose a
  production method or rewrite model prompts.
- Required variations are explicit. Two outfits and four expressions do not
  automatically demand every combination. Atelier may hold extra variations and
  experiments without turning them into required work.
- Saved artwork is available for story use without a separate approval gate.
  A changed story requirement does not delete or invalidate the current saved
  image. The creator decides whether to revise it in Atelier.
- Removing a story requirement preserves its catalog item and visual work;
  Atelier distinguishes assets no longer requested by the story. Deleting an
  asset is a separate explicit action. Renames retain working documents,
  revisions and canvas associations.
- Synchronization must protect incomplete drafts and unsaved work. It must not
  interpret a temporary syntax error as a request to remove catalog items.

Acceptance also runs in reverse: create a character, illustration or variation in
Atelier, see it become available in Studio, reference it in the story and reopen
both apps with the same identity and artwork links intact.

### Agreed LLM writing assistance in Studio · 2026-10-28

- [ ] **SF-PLAN-15 — Editor rewrite and expansion.** Offer Rewrite, Expand and
  custom instructions over a selection, beat or scene. Provide preserve-facts
  and allow-creative-changes choices. Review proposals beside the original with
  accept, reject and request-another-version actions. Done when an accepted
  proposal applies as one undoable source edit and rejected proposals leave the
  story and asset catalog untouched. Use the Agentkit workflow integration in
  SF-PLAN-28 for execution.
  Editor review (2026-11-01): continuous Inline and Side-by-side diff views,
  editable proposals, per-change reset/selection, grouped Keep/Reset and panel-owned navigation,
  retry and overall actions. Pending edits stay out of autosave/Preview;
  acceptance remains compiler-checked and undoable. Native acceptance and
  boundaries are recorded in [DESIGN.md](../DESIGN.md) and [workflows](workflows.md).
- [ ] **SF-PLAN-16 — Relevant writing context.** Supply nearby passages, relevant
  character descriptions, scene participants and project writing instructions.
  Preserve DSL structure and resolved references, surface diagnostics on proposed
  changes and make introduced characters or illustration requirements explicit.
  Connect accepted new requirements to the shared catalog through normal source
  reconciliation. Done when rewrites follow the selected scope and instructions,
  and proposed asset requirements become production work only after acceptance.
- [ ] **SF-PLAN-17 — Protect concurrent writing and proposal state.** Bind each
  proposal to its original source snapshot and range. If the writer edits the
  source while the model works, retain the proposal and report the mismatch
  rather than overwriting newer text. Cancellation or provider failure preserves
  the draft and request. Done when delayed responses, changed selections, source
  edits and project switches cannot apply a proposal to the wrong text/project.

The first delivery is a writing collaborator within the editor. Example requests:
make a character's suspicion implicit; expand a passage into a tense conversation
with a specified endpoint; or suggest choices and their narrative consequences.
Branch suggestions are reviewed before becoming DSL structure. Expanding dialogue
must not incidentally generate an entire branching story.

Studio's writing assistant edits authored text and story-facing specifications.
Visual model prompts, image editing and asset production remain Atelier-owned.
Whole-story restructuring and autonomous writing are outside this first delivery.

### Agreed character staging and independent playback · 2026-10-28

- [ ] **SF-PLAN-18 — DSL staging and compiled scene plans.** Specify background
  selection, character instances and variations, left/center/right groups,
  alignment, front-to-back order, spacing/overlap and optional per-instance
  position/scale adjustments. Validate references and lower staging into explicit
  compiled instructions with pinned saved assets. Exact syntax remains to be
  designed. Done when the same declarations produce a deterministic scene plan
  with actionable diagnostics for invalid staging or unavailable assets.
- [ ] **SF-PLAN-19 — Shared runtime staging and visual presentation.** Track the
  current background, character instances, selected variations, groups, positions,
  depth, visibility and scale in runtime state. Render that state in Player and
  Studio preview using the same presentation behavior. Start with a fixed stage
  and immediate changes. Preserve staging until explicitly changed; dialogue
  does not automatically rearrange characters or enlarge the speaker. Done when
  two characters (one left, one right), three (two left, one right) and five
  (two left, three right) render with authored overlap and front-to-back order.
- [ ] **SF-PLAN-20 — Variation consistency and author-controlled sizing.** Support
  switching saved variations without changing the character instance's scene
  placement or presentation scale. Use consistently prepared sprite images as
  the initial asset contract, with matching canvas dimensions, apparent body
  scale and framing within each compatible variation set. Character height
  relationships and deliberate close/far/emphasis effects are authored scene
  choices; do not normalize every character to the same visible height. Done
  when an expression swap preserves framing and explicit scale adjustments affect
  only the intended instance without modifying its source image.
- [ ] **SF-PLAN-21 — Playback independent of Atelier.** Resolve all required
  images, variations and presentation information from ordinary project assets
  and the compiled story. Atelier is an artwork/sprite creation tool, not a
  playback service or required asset resolver. Done when a prepared project plays
  on a machine without Atelier installed, and Studio previews its existing
  artwork without opening Atelier or invoking generation/editing providers.

Scene alignment, grouping, overlap, depth, position and scale belong to the DSL,
compiler, runtime and visual hosts. They are independent of the image-creation
process. Atelier does not dictate stage positions or group layouts. It can help
prepare consistent variation images, but playback must not depend on Atelier's
working documents or private metadata. Begin with a consistent PNG sprite
contract rather than requiring separate asset registration metadata.

Groups may overlap their members and place characters behind one another. Do not
force equal spacing, shrink the cast to reveal every complete sprite or require
every member to be equally visible. Position and scale adjustments are explicit
presentation choices, separate from visual edits to the assets. Animation and
elaborate camera movement are outside this first staging delivery.

Acceptance includes variation swaps within an overlapping group, authored
differences in character height and scale, continued staging across dialogue,
save/restore of scene state and matching Studio preview/Player output. Run the
asset-only playback check independently of Atelier and all inference services.

### Agreed asset-linked canvases and seamless saving · 2026-10-28

- [ ] **SF-PLAN-22 — Associate existing canvases with project assets.** Link the
  current Atelier canvas/artboard to a character, variation, illustration, place,
  prop or other shared visual item. Open the associated work from the backlog or
  Studio; allow an Atelier-created canvas to create or select a catalog item.
  Retain the existing drawing, layer, generation and image-editing tools. Done
  when the creator can freely work on an asset's canvas from either entry point
  without duplicate documents or a required sequence of production stages.
- [ ] **SF-PLAN-23 — Save editable work and its usable image together.** Extend
  normal Atelier save to persist the linked editable document and materialize
  the canvas image at its project asset location, including alpha when present.
  Notify Studio to refresh that asset through SF-PLAN-04/06. Never expose a
  partially written image; report failed saves and retain recoverable work rather
  than claiming the asset updated. Done when drawing, editing or extracting a
  sprite and saving updates Studio automatically, and reopening preserves both
  the editable canvas and its usable project image.

The canvas is the asset's working surface. There is no mandatory design approval,
sprite approval, publishing, export or import step between Atelier and Studio.
The backlog organizes work rather than prescribing a production process. Tools
for transparency, extraction and consistent variation dimensions are available
when useful; they are not required stages. Optional model candidate comparison
does not introduce an asset-level approval gate.

Saved images are ordinary shared-project files. Studio and Player consume those
files and catalog references without opening Atelier or reading its private
working documents. Atelier remains unnecessary after artwork has been saved.
Acceptance covers saving a new character, revising an illustration and saving a
transparent variation, observing each change in Studio without manual transfer,
and continuing to use those assets with Atelier closed or uninstalled.

### Agreed local models and Atelier capabilities · 2026-10-28

- [ ] **SF-PLAN-24 — Add MOSS local image generation.** Integrate MOSS alongside
  Krea 2 and Qwen Image 2.1 through Agentkit's migrated native inference support
  (SF-PLAN-27), using the shared image contract in SF-PLAN-25.
  Verify the pinned backend, add the model profile and provisioning for diffusion
  weights, text encoder, VAE and tokenizer, and expose it in Atelier's model
  selection. Keep generation/editing support explicit; benchmark the selected
  checkpoint rather than assuming speed from the model name. Done when a creator
  can select MOSS, generate into the current asset workflow and switch between
  local models without changing the canvas or linked story asset.
  Progress: Agentkit has the pinned SFT profile, four-artifact provisioning and
  tokenizer overrides; Atelier exposes MOSS as generation-only. Clipforge's
  updated Metal runtime successfully loaded the downloaded model and tokenizer
  on the M3 Max. Generation benchmarking and model switching with saved artwork
  remain pending; see [Atelier's MOSS acceptance](atelier.md#moss-image-sft).
- [x] **SF-PLAN-25 — Expand the shared image capability interface.** Establish
  Agentkit image contracts during SF-PLAN-27 and represent capabilities and limits
  per model/deployment: generation/editing,
  ordered reference inputs, supported masks/local-edit mechanisms, alpha input
  and output, dimensions and applicable sampling controls including seed, steps
  and LoRAs. Remove universal restrictions where verified routes support more;
  validate actual native/hosted wire behavior rather than assuming model-level
  features are exposed by every provider. Done when callers use one capability
  contract and supported references, alpha and controls reach the backend without
  being silently dropped or flattened.
  - [x] Add per-route operation, reference-count, mask, dimensions, sampling and
    alpha metadata. Share queue preflight with execution; reject unsupported
    seed, steps, LoRAs and alpha requirements before submission. Remove the
    universal reference-count limit; each route supplies its verified limit.
    Expose ImageModel and hosted image construction through Agentkit's normal
    provider configuration and credential resolver. Native seed mapping is
    retained; known hosted routes retain ordered references and mask pixels.
  - [x] Verify native Qwen ordered references and RGBA before advertising them.
    Live Metal evidence (2026-10-31, Apple M3 Max, pinned server `3f8527a`):
    transparent generation and RGBA reference editing at 512×512, 20 steps and
    seed 42 completed in 141.59 and 188.68 seconds. Their PNGs contained 162,831
    and 161,624 nearly transparent pixels of 262,144, with opaque subjects and
    partially transparent edges. The shared image client then accepted a source
    plus two ordered RGBA references with both alpha flags enabled; an eight-step
    transport smoke run completed in 104.32 seconds and retained transparency.
    Agentkit's profile now owns the three-input limit and Qwen alpha metadata;
    its native wire client carries every reference in order without rewriting PNG bytes.
    An opaque response to a transparent-output requirement fails with its job ID.
    Offline regression failed on the original one-input restriction and now
    checks exact ordered PNG bytes, excessive inputs, masked-input geometry, opaque
    output rejection and Krea's narrower capabilities. Atelier's existing picker,
    queue preflight and execution consume this contract; obsolete single-input
    gates are removed. All three PNGs survived Atelier result attachment and
    `.atelier` save/reopen with encoded bytes and provenance intact.
    Reproduction: [native capability probes](atelier.md#native-image-capability-probes).
    Evidence: `.svs-e2e/image-capabilities/` (alpha/reference/persistence logs,
    framework regression logs, PNGs and `qwen-alpha-results.atelier`). This verifies
    transport, transparency and persistence; edit fidelity and live picker/queue
    interaction with multiple inputs have not been visually accepted.
    Workspace tests/all-target checks, strict Studio-core/Atelier Clippy and
    formatting pass. Agentkit native-feature and hosted-only tests, strict
    native-feature Clippy and package formatting pass. Existing vendored GPUI
    Cocoa deprecation warnings remain.
  - [x] Verify native model-specific masked editing before advertising it.
    Controlled Qwen Metal acceptance (2026-10-31, Apple M3 Max, server `3f8527a`)
    used the same 512×512 source, prompt, two steps and seed 42 with black, white
    and half-image masks. RGB MAE (0–255) versus the black-mask reconstruction:
    white 54.149913, half-mask editable region 40.590393, far-protected region
    0.000000. Black-mask reconstruction differed from the source by 0.945464;
    the half-mask far-protected region differed by 0.826960. This verifies latent
    anchoring and demonstrates that protected source pixels are not exact.
    A shared-client masked edit with two additional RGBA references passed in
    61.58 seconds: editable MAE 89.607081, far-protected MAE 0.000000 against the
    same reconstruction. All four PNGs survived Atelier attachment and project
    save/reopen with embedded bytes and provenance preserved.
    Agentkit now identifies optional latent masking explicitly, transmits source,
    mask and ordered references without rewriting their PNG bytes, and requires
    matching source/mask/output dimensions before startup. Krea still rejects
    editing. The existing Use mask tooltip explains possible unmasked-area drift;
    exact compositing stays app-owned. Regression first failed on the unsupported
    shared mask contract, then passed with exact mask/source/reference bytes,
    strength/sampling controls, invalid geometry and unchanged unmasked wire
    behavior covered. Hosted controls without verified mappings stay unavailable.
    Reproduction: [native mask acceptance](atelier.md#native-mask-acceptance).
    Evidence: `.svs-e2e/native-mask-acceptance/` (inputs, masks, four outputs,
    metrics, logs and `qwen-mask-results.atelier`). These two-step runs establish
    mechanism and persistence, not artistic quality or live tooltip interaction.
    Workspace tests/all-target checks, strict changed-package Clippy and
    formatting pass. Agentkit native-feature and hosted-only tests, strict
    native-feature Clippy and package formatting pass. Existing vendored GPUI
    Cocoa deprecation warnings remain.
    SF-PLAN-26 owns additional tool exposure and transparent-extraction UX.
- [ ] **SF-PLAN-26 — Connect capabilities to Atelier's existing tools.** Expose
  supported reference inputs, selections/masks, transparent extraction and
  applicable generation controls within the canvas workflow. Distinguish model
  guidance from compositing that preserves protected pixels, and distinguish
  replacing/extracting content from adding an overlay. Preserve transparency
  through editing and normal project save. Done when a creator can combine
  character/outfit/pose references on a supported route, edit a selected area,
  extract a transparent sprite and save it for Studio without manual transfer.

Krea 2 and MOSS serve local generation; Qwen Image 2.1 serves generation, editing
and transparent extraction. The creator chooses the model in Atelier. Model
switching retains the current canvas and asset association. Capability-dependent
tools explain unsupported combinations before queueing; they do not impose a
production sequence or move image operations into Studio. Additional provider
models can expose the same tools when their adapters support the capabilities.

### Native runtime distribution · 2026-10-29

- [ ] **SF-PLAN-30 — Ship and validate the Linux NVIDIA CUDA runtime.**
  Build the pinned `sd-server` for each supported Linux release target during
  packaging and include its required redistributable CUDA/runtime libraries.
  Resolve the bundled executable independently of the working directory; keep
  downloaded weights in writable application data and preserve explicit provider
  paths. Define supported GPU compute architectures and minimum driver versions;
  the current build defaults to compute capability `86`, which is not evidence
  of broader GPU coverage. Users need a compatible NVIDIA driver, without a CUDA
  development toolkit, compiler, Git or source checkout. Clipforge owns its
  release packaging; Studio and Atelier need equivalent application packaging
  and acceptance while consuming Agentkit's inference APIs. Done when a clean
  supported NVIDIA Linux machine installs the package, provisions weights,
  generates and edits with Qwen, generates with Krea, switches/reuses models,
  cancels work and saves output using the bundled runtime. Test relocation and
  launching outside the checkout, and report incompatible drivers/GPUs clearly.
  Record the distribution, GPU, driver, packaged library versions and acceptance
  logs for each supported release target. This makes the Linux CUDA portion of
  [§12 item 11](#12-suggested-implementation-order) actionable and complements
  SF-PLAN-27's live inference acceptance.
  Current evidence: Clipforge's macOS release bundle includes the pinned
  Metal-enabled server and passes a relocated packaged-runtime launch check.
  This does not establish Linux CUDA packaging or generation readiness.

### Existing plans to reconcile

The language and prototype UI plans have been consolidated here. Review older
Studio visual-creation proposals against SF-PLAN-01 before executing them.
The broader product and UX assessments still need the review tracked below.

- [ ] **SF-PLAN-09 — Reconcile the product backlog.** Review remaining work in
  [§12](#12-suggested-implementation-order), including re-sync, distribution,
  persistence, scheduling and generation validation. Move actionable remainders
  into this checklist with owning apps and evidence; retire superseded proposals.
- [ ] **SF-PLAN-10 — Design expressive performance and media.** Design speech
  annotations using real authored scenes: speaker identity, voice version,
  delivery scopes, timing, events and pronunciation must be typed and
  source-mapped. Adapt selected performance to FineWineTTS without embedding its
  backend syntax in the DSL. Define variants, composition, transitions and media
  durations against the runtime plan. Done when authored intent and exact
  bindings survive compilation and playback, with unsupported directions reported
  as readiness blockers rather than silently replaced by neutral performance.
- [ ] **SF-PLAN-11 — Reconcile UX and workflow follow-ups.** Review outstanding
  acceptance in [DESIGN.md](../DESIGN.md#8-delivery-order-and-verification), the
  [development and regression scenarios](development.md)
  and the prototype acceptance notes below. Verify current behavior before
  promoting findings into TODOs and assign visual creation work to Atelier.
  - [x] Project navigation availability (2026-11-01): hide Story, Canvas and
    Preview until a project is open; palette destinations and keyboard shortcuts
    obey the same prerequisite. Global navigation remains available. Regression
    coverage checks palette destinations before/after opening a project;
    manual sidebar acceptance is assigned to the user. See [DESIGN.md](../DESIGN.md).
- [ ] **SF-PLAN-12 — Extend workspace language tooling.** Add workspace navigation
  and concrete code actions once the multi-file contract in SF-PLAN-08 is defined.
  Done when Studio and external editors agree on analysis, rename touches only
  resolved references, formatting preserves prose/comments and cyclic imports or
  invalid edits produce bounded, actionable diagnostics.
- [ ] **SF-PLAN-13 — Specify further language semantics.** Design variables,
  conditions, expressions and localization with explicit grammar, semantics and
  fixtures before implementing them. These are not implied by the current DSL.
  Done when each adopted feature has a documented contract and shared
  language/compiler/runtime tests.

### Consolidated language and UI implementation notes

The shared lossless front end, recovering AST, UTF-16 positions, diagnostics,
completion/navigation, single-document LSP, rename, formatting and semantic tokens
are implemented. The compiler supplies deterministic versioned plans, source
maps, explicit flow, immutable asset bindings and readiness. Studio persists
authoritative `.sf` files with conflict handling and scene/beat artwork
associations. The shared runtime, CLI trace, Studio preview and Player implement
manual playback and verified save/restore. Current contracts and evidence live in
[language.md](language.md), [compiler.md](compiler.md),
[data-model.md](data-model.md) and [runtime.md](runtime.md).
Project-relative asset paths belong to SF-PLAN-02;
multi-file analysis belongs to SF-PLAN-08/12.

The earlier UI prototype phases for completion, command entry, Canvas grid,
detail/inspector and keyboard palette were implemented. Their formerly blocked
Writer, pending references, origins/backlinks and run-mode work is also
implemented as recorded in §12. The
Writer and Canvas prototypes have been removed;
[DESIGN.md](../DESIGN.md) governs current interactions. Retained principles:
pending artwork is distinct from syntax failure; descriptions and visual settings
use appropriate controls; revisions distinguish canon from experiments; commands
and direct UI use shared operations; Enter submits and Tab completes. Prototype
implementation instructions and line-number references are superseded.

Acceptance follow-up under SF-PLAN-11: verify command entry, Canvas navigation,
revision promotion and a complete keyboard-only workflow using the current
`config/plans/` harness and native UI checks. For language changes, run focused
language/LSP tests and the repository-required workspace checks; editor caret,
highlighting and navigation behavior also need interactive verification.

### Supporting design and implementation history

Sections 1–13 preserve the earlier canvas design and feature status. The direction
above governs new planning where it changes Studio and Atelier responsibilities.

## 1. Vision recap

The product lets a user write a visual story and create its slides, iterating
on both until the result is right. The current app already covers prompt →
storyboard beat → Qwen-Image-2.1 generation → revision history. The target
shape adds:

- a shared **artifact registry** (characters, environments, objects, scenes,
  beats) with persistent visual variants;
- a **creation canvas** workspace where artifacts are created and edited
  through language (`/create …`, `/modify …`) plus direct manipulation;
- LLM-assisted story writing (plot → bible → scenes → beat narration) with
  propose/approve semantics;
- full per-beat generation history and a LoRA
  registry for character/environment consistency.
- the **Storyfella DSL** as the authored story surface: `.sf` files that
  reference registry artifacts, with the canvas as the visual-asset editor
  (see §9).

**Model focus:** Qwen-Image-2.1 (served by the native `stable-diffusion.cpp`
process) is the only generation model. The stack stays as documented in
`architecture.md`; Qwen-Image-2.1's prompt-following and quality are the
baseline and LoRAs build on top of it. Krea 2 and the older SAM/segmentation
edit path were removed in the Qwen migration.

## 2. Vocabulary

| Term | Meaning |
| --- | --- |
| Beat | One slide/panel of the visual story. A beat is a composition of layers referencing other artifacts. |
| Scene | A group of beats set in one place with a cast. New level above beats. |
| Artifact | Any first-class project entity: story, scene, beat, character, object, environment. One unified id space (`c:<ref>`). |
| Character | Reusable person artifact with named visual variants. |
| Object | Reusable prop/object artifact (same pattern as character). |
| Environment | Reusable place artifact (house, city, forest, …). |
| Backdrop | A *role inside a composition*, not an artifact kind: the layer slot a beat fills with an environment variant or a fresh generation. |
| Variant | A named visual variation of an artifact (character: outfit/age/body/hair/expression; environment: time-of-day/weather/season/mood). |
| Composition | A beat's description of how layers (backdrop, baked elements, dynamic characters) combine into one image. |
| Operation | A typed command from the operation set (Section 7), invoked via slash syntax or proposed by the LLM. |

## 3. Artifact model

```text
Project
├── Story(s)                     — one or more stories per project
│   ├── story documents          — premise, plot (prose, separate from artifact text)
│   └── Scene(s)                 — location + cast (character × variant) + beats
│       └── Beat(s)              — slides; each a composition of layers
│           └── layers           — references to character/environment/object artifacts
│               └── revisions    — persisted generation history per visual artifact
├── Character(s) (+ variants)    — bible level, reusable across stories
├── Environment(s) (+ variants)  — bible level; rooms are children, not variants
└── Object(s) (+ variants)       — bible level, reusable props
```

Rules agreed so far:

- **Unified registry, one id space.** Every artifact (including beats) is the
  same kind of thing to the system; `c:<ref>` references work uniformly.
- **Variants are artifacts** with `variant_of` pointing at their base. Axes
  (outfit, age, body, hair, expression, time-of-day, weather, season, mood)
  are tags for organization, not structure.
- **Rooms are children, not variants.** A house *contains* a kitchen; a
  kitchen's *variants* are states of that same space (rainy evening, morning).
- **A beat is a composition** — a list of layers referencing other artifacts
  with roles; it is not limited to a single environment + character.
- **Scene = location + cast + beats.** The timeline groups clips by scene.

## 4. Composition modes

A beat renders in one of three modes:

| Mode | Description | Change cost |
| --- | --- | --- |
| Baked | Everything in one Qwen-Image-2.1 generation (today's flow). Max quality. | Regenerate the region. |
| Layered | Environment base + character/object cutouts composited by the app. | Instant: swap variant/expression and recomposite. |
| Hybrid | Baked backdrop (environment + background characters baked in) with dynamic character layers composited on top. | Only dynamic layers regenerate. |

Consequences:

- The beat needs a **composition spec** separate from the rendered image:
  `background` (environment variant ref or "generate fresh"), `layers:
  [{artifact_ref, variant_ref, role, anchor}]`, and `mode`.
- **Re-sync is mode-dependent:** layered → recomposite; baked → regenerate; hybrid → regenerate dynamic layers only.
- **Cutouts come from the model's prompt-driven alpha channel**, not from
  segmentation + matting: Qwen-Image-2.1 renders an alpha channel when the
  prompt asks for transparency, and it survives only in PNG/WebP (verified
  against upstream's own examples). Cutout quality is subject-dependent —
  reliable for logo/object subjects, weaker for some portraits — so layered
  mode is the iteration mode while baked/hybrid dominate final quality.
  *(Cutout quality acceptance: open.)*
- **Positioning is direct manipulation, not an operation.** Ops carry intent
  (what), gestures carry geometry (where, size, z-order).

## 5. Generation & consistency

- **LoRA registry.** Existing plumbing: `--lora-model-dir` on `sd-server`, up to
  eight `LoraSelection { path, multiplier }` per generation request
  (`clipforge_inference::sd::LoraSelection`, in the shared crate). The project-level
  registry (name, file,
  base model, tags), binding a LoRA to an artifact, and auto-injection into
  every generation referencing that artifact are implemented (§12 item 6).
- **Character/environment consistency** is achieved by injecting canonical
  reference images and bound LoRAs into every generation that references the
  artifact. Trained character LoRAs are expected to be the main lever.

### Regional edits — `modify` is an instruction edit

The flagship use case is a visual `modify` on a character ("change her hair
style"). It is a single operation, not a mask sequence:

1. `modify <char> <description>` — the change description is grounded (with LLM
   help) against the artifact's description, producing a plain-language
   instruction.
2. `generate` — the artifact's active revision is the reference image and the
   grounded instruction is the prompt; the model re-renders, preserving what the
   instruction did not mention.
3. The result is stored as a new revision; `/describe` promotes an edit to the
   artifact's canonical look, and `/variant` forks it into its own artifact.

`modify` is **not** localized by a user-selected mask: the instruction's
semantics decide what changes. The earlier mask/inpaint/composite flow
(segmentation, mask confirmation, inpaint) was removed with the Qwen migration —
there is no mask source, no segmentation step, and no composite. Localization is
a quality goal of prompt-following, not a region drawn by the user.

The same flow applies to environments and objects; character hair is the
canonical example and the first to be built.

## 6. Creation canvas

The central workspace for creating and modifying artifacts:

- A large (effectively infinite) **artboard** with floating artifact cards,
  pan/zoom viewport.
- A **prompting area** below the artboard:
  - `/create <kind> <description>`, `/modify <ref> <description>`, … — the UI
    parses slash syntax into typed operations, with autocomplete and argument
    hints.
  - Free-form messages go to the LLM, which responds with a *proposed
    sequence of the same typed operations*.
  - `c:<ref>` mentions of existing artifacts, autocompleted from the registry.
  - Syntax highlighting for `/operations` and `c:refs` — implemented as a
    transparent textarea over a tokenized render (code-editor style); a
    dedicated chunk of UI work, deferred.
- **Artifact card anatomy:** written features/operations on the right, visual
  grid of variants/revisions on the left. Clicking an image expands it to fill
  the visual space with left/right nav actions through the variants.
- The canvas is the place where artifacts are born and edited; the operation
  log per artifact is its history, provenance for re-sync, and later the
  timeline for animations/transformations.

*(Relationship to the current Studio — creation canvas as the main workspace
vs. side-by-side with Studio for sequencing: open; lean is side-by-side.)*

## 7. Operation set (draft)

Typed operations defined in Rust (this is the planner business logic that is
currently unimplemented). Slash syntax is parsed by the UI; the LLM maps free
text onto the same set.

Every operation has a **kind**:

| Kind | Meaning |
| --- | --- |
| Primitive | Atomic, single effect; the atoms of the system |
| Compound | A named, saved sequence of primitives (data, not code) |
| Pure | Read-only (ask, summarize) — no mutation, no log entry |

**Slices** define when each operation ships. Slice 1 is the five-operation
core that drives prototyping; slices 2 and 3 extend it.

| Slice | Operation | Kind | Syntax | Purpose |
| --- | --- | --- | --- | --- |
| 1 | create | primitive | `/create <kind> <description>` | New artifact (story, scene, beat, character, object, environment) |
| 1 | variant | primitive | `/variant <ref> <description> [axis]` | New visual variant of an artifact |
| 1 | regenerate | primitive | `/regenerate <ref> [prompt]` | New revision of the active image (fresh seed, or edited prompt with current image as reference) |
| 1 | revert | primitive | `/revert <ref>` | Re-activate the previous image; nothing is deleted, the history keeps both |
| 1 | compose | primitive | `/compose <scene> <description> [layer refs…]` | New beat in a scene |
| 1 | draft | primitive | `/draft <ref> <request>` | LLM proposes story text; user approves |
| 2 | modify | primitive | `/modify <ref> <description>` | Change the artifact's defining text |
| 2 | write | primitive | `/write <ref> <text>` | Direct text (narration, premise, …) |
| 2 | rename | primitive | `/rename <ref> <name>` | Rename |
| 2 | delete | primitive | `/delete <ref>` | Remove (cascade rules TBD) |
| 2 | outpaint | primitive | `/outpaint <ref> <description>` | Extend the image beyond its borders |
| 2 | layer | primitive | `/layer <beat> <ref> [role] [variant]` | Add/swap a layer (role: backdrop / baked / dynamic); geometry added by drag afterward |
| 2 | promote | primitive | `/promote <ref>` | Mark a revision as the artifact's canonical reference image |
| 2 | attach-lora | primitive | `/attach-lora <ref> <lora> [multiplier]` | Bind a registry LoRA; auto-injected into the artifact's generations |
| 2 | import | primitive | `/import <file>` | Bring an external image in as a revision |
| 2 | ask | pure | `/ask <question>` | LLM Q&A about the story; no mutation |
| 2 | variations | compound | `/variations <ref> <n>` | Generate an N-revision grid for comparison |
| 2 | re-sync | compound | `/re-sync <ref>` | Regenerate/recomposite every beat referencing the artifact |
| 3 | cutout | primitive | `/cutout <ref>` | Extract subject to a transparent PNG using the model's prompt-driven alpha channel, for layered mode |
| 3 | lora-scale | primitive | `/lora-scale <ref> <multiplier>` | Adjust a bound LoRA's weight |
| 3 | lora-remove | primitive | `/lora-remove <ref>` | Unbind a LoRA |
| 3 | style | primitive | `/style <ref> <description>` | Project/story-level style binding (style LoRA + prompt prefix) |
| 3 | duplicate | primitive | `/duplicate <ref>` | Branch an artifact (explore without losing the original) |
| 3 | tag | primitive | `/tag <ref> <tag>` | Organizational tags |
| 3 | note | primitive | `/note <ref> <text>` | Author annotation; never affects generation |
| 3 | move | primitive | `/move <beat> <scene>` | Relocate a beat between scenes |
| 3 | crop | primitive | `/crop <ref>` | Geometry-only crop |
| 3 | upscale | primitive | `/upscale <ref>` | Raise resolution |
| 3 | pose | primitive | `/pose <ref> <description>` | Re-pose variant without changing identity |
| 3 | expression | primitive | `/expression <ref> <description>` | Expression variant (dynamic layers) |
| 3 | transition | primitive | `/transition <beat> <effect>` | Fade/cut/dissolve out of a beat; future animation input |
| 3 | camera | primitive | `/camera <beat> <description>` | Camera note (zoom/angle/pan); metadata now, animation later |
| 3 | export | primitive | `/export <beat\|scene\|story>` | Render composed output (image now; presentation/video later) |
| 3 | summarize | pure | `/summarize <ref>` | Condense revision/operation history into a description |
| 3 | consistency-pass | compound | `/consistency-pass <character>` | Regenerate every beat of a character after a bible change |
| 3 | scene-fill | compound | `/scene-fill <scene>` | Draft beats for every story point, then compose + generate |
| 3 | re-style | compound | `/re-style <scene> <description>` | Apply a style across a scene's beats |
| 3 | mood-pass | compound | `/mood-pass <scene> <mood>` | Apply a mood variant across a scene's beats |

Semantics notes:

- **Visual `create`/`variant` generate the first image** as part of the
  operation (prompt = description; the variant's first image is an edit of
  the base — its active image as reference, the size it inherits). A failed
  generation keeps the artifact with a failed revision; `/regenerate` is
  the retry. Text-only kinds (story, scene, beat) still apply instantly.
- `create`/`variant` accept `--name`; without it, names derive from the
  description (create) or the base (variant: `{base}-{axis}`/`{base}-{slug}`).
- `/modify` on a visual artifact is an **instruction edit** (see §5): the
  artifact's active revision is the reference image and the grounded
  description is the prompt. There is no mask/segment/inpaint path. The
  flagship use case is a character hair-style change.
- `/layer` carries intent only; where a layer goes is a drag afterward.
- **Slice 1 core (the five):** create, variant, regenerate, compose, draft —
  enough to exercise the full pipeline: artifact creation, visual iteration,
  composition, and LLM-assisted story text.

**Execution model (pending confirmation):** user-typed ops apply immediately
and are undoable via the operation log; only LLM-proposed op sequences go
through approve-before-apply. Rationale: slash ops are explicit intent;
free-form messages are suggestions, and generation is slow/expensive.

### From operations to pipelines (execution layer)

Two layers, kept distinct:

- **Operations (intent)** — the typed set above: semantic, logged,
  approval-gated, user/LLM-facing. The "what".
- **Pipelines (execution)** — deterministic sequences of primitives built
  with a typed builder in Rust; the "how" behind each operation.

Each operation compiles to a pipeline:

```rust
let pipeline = GenerationPipeline::new()
    .model("qwen-image-2.1")
    .size(1024, 1536)
    .steps(25)
    .reference_image(&revision.asset_path)
    .lora("character.safetensors", 0.8)
    .build();           // static validation happens here

let job = pipeline.run(&runtime).await?;
```

Rules:

- Each operation compiles to a pipeline; pure operations (`ask`, `summarize`)
  are plain LLM calls, not pipelines.
- The builder validates statically at `build()` (sizes within contract bounds,
  step counts, LoRA count); runtime failures surface as job
  statuses, unchanged.
- The VLLM (the LLM endpoint) never produces code — it emits operation stacks as
  structured JSON (function calling against a schema Rust defines); Rust
  deserializes, validates, and only *proposes* for approval before executing.
- Slice 1 keeps stacks linear (a sequence of steps); parallel execution
  (e.g., variations grids) can layer on later.
- The operation log records each executed stack with its arguments.

**Typed intermediates.** Steps are not a flat list of side effects; they
produce values later steps consume, and the builder returns typed handles:

```rust
let pipeline = Pipeline::new()
    .reference_image(&revision.asset_path)
    .generate("give her a bob cut", Some(image))
    .build();
```

Handles resolve at run time; step kinds are checked statically, so a caption
cannot be fed into an image slot. There is no mask handle — the closed step
vocabulary is `LoadImage`, `Checkpoint`, `Generate`, `Llm`.

**The VLLM is also a pipeline primitive.** Besides proposing whole stacks it
can be a step inside one: `describe(image) -> caption/prompt`, `plan(text) ->
stack`, `critique(image, prompt) -> feedback`. The LLM endpoint is external
and may be offline, so LLM steps are soft dependencies: they degrade to
manual input and never hard-fail a stack.

**Closed vocabulary.** The VLLM combines step kinds, it never invents them.
Step kinds are a Rust enum; the JSON schema for stacks contains only
enumerated kinds with validated parameters.

**Failure semantics.** Stacks are linear and fail fast: a failed step marks
the stack failed, keeps any intermediates produced so far, and the artifact's
revision status reflects it. No retries or branching in slice 1.

**Undo is state restore, not re-execution.** Replaying an LLM-derived stack
cannot reproduce the same result, so undo restores the
prior project state (revisions already provide snapshot semantics); the log
is for provenance, not replay.

**Testability.** The layering makes the pipeline layer testable end to end:

1. **Builder unit tests** (no IO): static validation, step ordering,
   parameter bounds, stack
   deserialization from JSON fixtures.
2. **Execution tests with a fake backend:** pipelines run against a stubbed
   `GenerationBackend`; assert request bodies (prompt, reference, LoRA, size) and
   revision state transitions. The VLLM step is injectable the same way.
3. **Wire serialization tests:** the `sdcpp` request/job types
   (the shared `clipforge-inference` crate, `../clipforge/crates/inference/src/sd/`)
   round-trip.
4. **Manual golden runs:** a scripted harness executes one pipeline against
   the real backend and drops the output image into a folder
   for human review.

Steps carry **seeds** so
a golden run can be replayed identically when a manual validation fails.

**CLI (`svs`).** The pipeline API gets a thin clap-based CLI so operations can
be driven and manually validated without the UI. It mirrors the operation
set: clap parses args → constructs the same typed operation → compiles to a
pipeline → executes — one code path shared with the UI and the VLLM.

- `svs project open <path>` — select the SQLite project
- `svs op create <kind> <description>`, `svs op variant <ref> <desc>`,
  `svs op regenerate <ref> [prompt] --seed --steps --size`,
  `svs op modify <ref> <description>`
- `svs stack run <stack.json>` — execute a serialized op-stack; doubles as
  the VLLM contract test bed, since the model emits the same JSON. `svs stack
  propose <message>` drives the LLM endpoint to produce a stack from free text.
- `--out <dir>` on any op drops the output image into a folder for human
  review — the manual validation tier.
- `svs log <ref>` — show an artifact's operation log.

The CLI is a separate binary crate sharing the domain modules
(config, generation client, runtime, operations/pipelines) without the
Dioxus UI: `crates/studio-core` holds the shared modules; `apps/studio/src/main.rs`
(UI) and `apps/cli/src/bin/svs.rs` (CLI) both build on it.

## 8. Story text pipeline

- A single **story document** per story: premise → plot → bible (characters,
  environments) → scenes → beat narration. Written in stages; the LLM assists
  at each stage via `/draft`, always propose/approve — no auto-apply. This
  story document *is* the `.sf` file (see §9); there is no separate
  `story_documents` table.
- **Recursive refinement via provenance:** beats/scenes store *references* to
  characters, variants, and locations by id. When the bible changes, affected
  beats are flagged ("uses Kitchen v2 — re-sync?") and re-sync is
  mode-dependent (Section 4). No automatic cascade.
- **Mixing text and visuals is bidirectional:** beat narration → image prompt
  with bible context auto-injected; generated image → narration text via
  captioning (captioning endpoint is currently a placeholder).

## 9. Storyfella DSL (built in this project)

Storyfella is a DSL and toolkit for interactive narrative experiences. It is
**built from scratch in this repository** — the sibling
`../storyfella-old-project-inspiration` prototype is reference material only,
not code to integrate. `storyfella-language` owns lexing, parsing, AST and shared
analysis; `storyfella-lsp` exposes it to external editors. Compiler and runtime
are independent crates. See [the language contract](language.md)
and [central backlog](#0-current-direction-and-actionable-backlog). The sequencer becomes the
visual-authoring surface for `.sf` projects.
Three surfaces share one artifact registry:

| Surface | Role |
| --- | --- |
| Story Writer | Edits `.sf` with syntax highlighting; asset references resolve to live chips (name, thumbnail) as you type; click a chip to jump to canvas |
| Canvas | Creates/edits the artifacts the `.sf` references |
| Studio | Sequence/timeline view, *derived* from `.sf` structure (scenes in order, choices as branches) |

Decisions:

- **Text-driven sync:** writing `scene kitchen` in the writer auto-creates
  or links the kitchen scene artifact; the registry follows the text. The
  canvas is the visual-detail editor for things the text only names.
- **`.sf` is the story source of truth** — diffable, greppable, versionable.
  SQLite keeps only the asset side (artifacts, revisions, operation
  log); the `story_documents` table is dropped from the schema.
- **References by name in the DSL** (`character: mia`), ids under the hood;
  the compiler catches collisions; the writer's autocomplete resolves names
  to ids as you type.
- **Validation extends to assets, but unresolved is *pending*, not an
  error:** `character: mia` where `mia` does not exist is amber with a
  Create affordance, not red. Same information, opposite valence — the story
  reads as a work queue rather than a list of mistakes, and the canvas is
  the fix surface. (Settled while building the writer; see the consolidated UI
  implementation notes in §0.)
- **The writer reuses the tokenized-editor primitive** designed for the
  prompt bar (`/op` highlighting, `c:` chips) — one editor component, two
  uses.
- Per-beat visual references (variant, composition mode) in the DSL are the
  next design layer below this one.

## 10. Persistence: SQLite

Decision: SQLite is the project format (`.svs-project.db`); the pre-SQLite
formats (TOML `ProjectStore`, stopgap JSON registry) were removed with their
one-time imports — the current format evolves via `schema_meta` migrations.
Assets (PNGs) stay as files on disk; portable project-relative image paths remain
planned (the current generator stores absolute paths). Schema versioning
via a `schema_meta` table. Applied so far: v1 (§10 tables), v2/v3 (the
legacy storyboard column, added then dropped), v4 (per-artifact default
size), v5 (per-artifact default steps and pinned seed), v6 (`RevisionOrigin`
on revisions), v7 (`projects.story_source`, the Writer's text), v8 (drops the
`masks` table with the removed mask edit path), v9 (exports Writer text to `.sf`
before dropping `projects.story_source`).

```sql
CREATE TABLE schema_meta (version INTEGER NOT NULL, applied_at TEXT NOT NULL);

CREATE TABLE projects (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

-- One id space for every artifact; c:<ref> references resolve here.
CREATE TABLE artifacts (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id),
  kind TEXT NOT NULL CHECK (kind IN
    ('story','scene','beat','character','environment','object')),
  name TEXT NOT NULL DEFAULT '',
  description TEXT NOT NULL DEFAULT '',
  variant_of_id TEXT REFERENCES artifacts(id),   -- NULL = base artifact
  variant_axis TEXT,      -- outfit | age | body | hair | expression |
                          -- time-of-day | weather | season | mood
  parent_id TEXT REFERENCES artifacts(id),       -- scene→story, beat→scene, room→building
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE revisions (
  id TEXT PRIMARY KEY,
  artifact_id TEXT NOT NULL REFERENCES artifacts(id),
  prompt TEXT NOT NULL,
  asset_path TEXT,               -- relative to project asset dir
  status TEXT NOT NULL,          -- queued | generating | completed | failed | cancelled
  seed INTEGER,
  model TEXT,
  created_at TEXT NOT NULL
);

-- Masks are gone: `GenerateRequest` has no mask field and `modify` is an
-- instruction edit (see §5); schema v8 dropped the table.

-- Beat composition layers.
CREATE TABLE layers (
  id TEXT PRIMARY KEY,
  beat_id TEXT NOT NULL REFERENCES artifacts(id),
  position INTEGER NOT NULL,
  artifact_ref TEXT NOT NULL REFERENCES artifacts(id),
  variant_ref TEXT REFERENCES artifacts(id),
  role TEXT NOT NULL,            -- backdrop | baked | dynamic
  anchor TEXT                    -- position/size JSON
);

CREATE TABLE lora_registry (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id),
  name TEXT NOT NULL,
  path TEXT NOT NULL,            -- relative to configured lora_dir
  base_model TEXT,
  multiplier REAL NOT NULL DEFAULT 1.0,
  artifact_id TEXT REFERENCES artifacts(id)  -- optional binding
);

-- Every applied/proposed operation; the basis for undo and provenance.
CREATE TABLE operation_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id TEXT NOT NULL REFERENCES projects(id),
  artifact_id TEXT REFERENCES artifacts(id),
  op TEXT NOT NULL,              -- name from the typed operation set
  args TEXT NOT NULL,            -- JSON
  origin TEXT NOT NULL,          -- user | llm
  status TEXT NOT NULL,          -- proposed | applied | rejected | reverted
  created_at TEXT NOT NULL
);

-- Story prose: not stored in the database. `.sf` files (see §9) are the
-- story source of truth; SQLite stores the asset side only.
```

Implementation notes: **resolved** — `rusqlite` (bundled, synchronous) with a
single writer connection behind a `parking_lot` mutex and WAL mode, in
`crates/studio-core/src/persistence/` (`ProjectDb`). Deliberate deviations from the sketch
above: timestamps are unix epoch integers (no chrono dependency),
`operation_log.id` is TEXT (the model uses `Uuid`), `op` holds the serde tag
name with the full `Operation` JSON in `args`, `artifacts.composition_mode`
holds the beat-only composition mode, `artifacts.drafts_json` is the
home for artifact-scoped generated text proposals, and `revisions.error` covers
the model field. Writes replace the registry rows in one transaction with
`PRAGMA defer_foreign_keys` (the circular `artifacts ↔ revisions`
reference).

## 11. Open questions

1. **Canvas vs Studio:** creation canvas as the main workspace, or
   side-by-side with Studio for sequencing? *(Lean: side-by-side.)*
2. **Naming:** beat vs slide vs panel.
3. **Execution model:** user-typed ops instant + undo; LLM proposals gated by
   approve. *(Proposed, pending confirmation.)*
4. **Short-id format** for `c:<ref>` chips.
5. **Layered-mode cutout quality** acceptance for final output (Qwen-Image-2.1's alpha channel is subject-dependent).
6. **Native mask input** — **resolved by removal:** the mask/inpaint path was deleted with the Krea/SAM migration; `modify` is an instruction edit and there is no mask source.
7. **SQLite crate** — **resolved:** `rusqlite` (bundled), one writer
   connection + WAL (`crates/studio-core/src/persistence/`). The TOML `ProjectStore` and the
   stopgap JSON registry were removed with their imports; formats evolve
   via SQLite migrations.

## 12. Suggested implementation order

**Workspace restructuring: implemented.** Studio is `apps/studio`
(`storyfella-studio`), with GUI-free `crates/studio-core`
(`storyfella-studio-core`). `apps/cli` and `xtask` remain; `apps/player`
is a visual Dioxus desktop host. Language, compiler,
LSP and story runtime are implemented as defined in
[`architecture.md`](architecture.md#storyfella-workspace-boundaries).
Studio preview and Player share execution, resource checks and controls. Studio
now edits authoritative `.sf` files; §0 tracks remaining language, integration
and execution work.

**Existing feature status and implementation history.** Use §0 for new planning;
SF-PLAN-09 tracks reconciliation of remaining work here with the current direction.

**Atelier relocation: implemented.** `apps/atelier` (`storyfella-atelier`, binary
`atelier`) is the separate GPUI art app, moved from FineWineTTS with its current
sources, project/data identity and macOS renderer patch. Studio remains Dioxus.
**Atelier navigation and panel refinement (2026-10-25): implemented.** Tabs and
actions share one header; the logo opens Home, with whole-card project opening
and Library access. Composer setup actions live in Settings, reached from its
options menu. All inspectors use Activity's docked panel frame. Activity reports
use compact diagnostics and specific app-scoped Clipforge launch failures.
Workspace checks/tests/format, strict Atelier Clippy and packaging pass. Native
checks cover card opening, composer navigation, shared panels and Activity report
actions; broader accessibility, narrow-window and tab-hover acceptance remain.
See `DESIGN.md` for the recorded checks.
**Atelier creation workflow (2026-10-24): implemented; broader acceptance pending.**
Schema v2 gives generated compositions real artboards and keeps references
separate. Full-artboard/selection edits create pending layers with review actions;
separate-image selection generation creates a new artboard. The shared floating
composer has durable source/mode drafts and captured artboard destinations.
Layer gizmos and precise values extend existing undoable transforms. An app-wide
Library and import-capable reference picker replace the project Library tab and
generation rail. Open projects retain independent tab state and recover after
restart. Clipforge owns provider/credential setup, with launch and readiness
refresh in Atelier; a setup deep link remains a Clipforge dependency. Actual-provider
selection quality and complete native/first-user
acceptance remain. Workspace checks/tests/format and strict Atelier Clippy pass;
partial isolated macOS checks are recorded in
[creation workflow](atelier.md). This entry supersedes the
older Atelier surface descriptions below.
**Atelier visual refinement (2026-10-24): implemented; partial native acceptance.**
The drawing dock prioritizes layers with compact contextual controls and disclosed
precise properties. References collapse without empty preview space; their extra
tools and study controls stay off the default drawing surface. The composer uses
one options disclosure, concise source/destination metadata and ordered reference
chips. Isolated native checks verified review/undo, opacity and collapsed states;
workspace checks/tests/format, strict Clippy and packaging pass. Broader resize,
accessibility and first-time-user acceptance remain. See `DESIGN.md`.
**Atelier composer sizing (2026-10-25): implemented; native layout checked.**
Fixed-width dimensions share one wrapping options row with size/sampling and a
settings icon. Routine controls have more room for readable provider/aspect
choices; secondary actions fit their content and one readiness footer replaces
duplicate warnings. The exact final test bundle was checked with hosted/local
providers and numeric/prompt editing, without submitting generation. Workspace
checks/tests/format, strict Atelier Clippy and macOS packaging checks pass.
**Atelier project tabs (2026-10-25): implemented; native interactions checked.**
Tabs support drag/keyboard reordering and save-before-close, including inactive
tabs. Failed saves and newer edits keep tabs open; pending image jobs block close.
Display order and active identity recover independently, and closing all tabs
persists an empty session. Library stays available without project controls.
Native checks verified reordering, close/failure retention, prompt reopening,
last-tab close and ordered restart recovery. Workspace checks/tests/format,
strict Atelier Clippy and macOS packaging checks pass (60 passed, 13 ignored).
**Atelier Activity (2026-10-25): implemented; native fixture acceptance.**
One app-wide Activity panel replaces the queue-only panel and raw footer errors.
Short contextual hints open scoped reports with recovery actions, optional
read-only diagnostics and verified copying. Drawing uses one right inspector at
a time. Repeated attempts group; request retries retain their original project,
source and settings. Save failures remain actionable until successful recovery.
Native checks cover mocked 429s, save failures, details/copy/Escape and cross-tab
retry/recovery. Reports and queue history remain session-only. Workspace gates,
strict Atelier Clippy and packaging pass (63 passed, 14 ignored); real-provider,
VoiceOver and other-platform acceptance remain.
**Shared hosted image execution: implemented in Atelier.** Clipforge owns the
async adapters for Replicate, fal.ai, Alibaba and GPT Image, shared presets,
job polling/cancellation and result metadata. Atelier retains its document
boundary and reads shared credentials. **Art editing and reuse: implemented.** Full-resolution
originals, a cross-project library, zoomable inspection, version branches,
before/after, editable image layers with placement, durable masks and supported
multi-reference editing are available in Atelier. Approved originals and painted
canvases import into Studio's completed revisions through portable `.sfart`
packages. Studio's hosted-provider selection and a text/vision art assistant
remain **planned**; see [Atelier](atelier.md) for limits and verification.

**Compact Atelier workspace and shared provider credentials: implemented.**
Named icon actions and optional image details follow DESIGN.md. Hosted provider
keys persist in the shared `clipforge.toml` through Clipforge's atomic,
owner-only config writer. Credential editing now lives in Clipforge.
**Game-art creation briefs: implemented.** The new-image composer offers
editable starting points for characters, objects, backgrounds and narrative
illustrations, with canvas shapes matched to each asset.
**Iteration gaps: implemented and checked on macOS.** The real-provider bookshop
walkthrough demonstrates chained edits and sibling branches. Source-specific
prompt drafts recover after restart, saved version names retain identity, prompt
reuse is available, and the repeated native Open check passes. First-time-user
and cross-platform acceptance remain outstanding. The image editor now has an
on-demand floating variation composer, and submitting a request opens a
project-scoped Artboards view from the Projects list. New projects show a blank
canvas artboard and a compact image prompt visible by default with dropdowns for
provider, aspect ratio, resolution, and local sampling steps. Drawn and generated
artboards appear together with compact per-artboard activity summaries. Local Qwen Image 2.1
generation/editing and Krea 2 Turbo generation are connected through Clipforge;
actual Linux CUDA inference requires a run on that machine.
See [iteration acceptance](atelier.md).

**Generation queue: implemented.** The Atelier generation rail queues requests.
**Add to queue** captures the provider, model, prompt, width, height, sampling
steps and the applied LoRA adapters (each with its weight) as an immutable
snapshot and runs one request at a time on the shared image server. **Pause**
holds dispatch without interrupting a running image. Each queued/running row has
**Abort**, targeting only that request; aborted items remain in history and
**Clear completed** drops completed
history; the list labels each request's state. Size, steps and adapters are set
per request in the **Image settings** window, and Settings no longer duplicates
them. The queue has its own panel, opened from a header badge that counts
outstanding requests. **Generate from drawing** opens a compact panel that
auto-switches to a **box select** region when the canvas is dragged, then
rasterizes the area and queues it. See [Atelier](atelier.md). Live multi-request
validation against native Qwen passed on macOS (2026-10-31; SF-PLAN-27 above);
hosted-provider queue acceptance remains pending. The queue's ordering, pause, individual
abort, history-clearing and region-clamping rules are unit-tested. Isolated mock
UI acceptance (2026-10-30) verified that aborting a queued request preserves the
running one, prevents the queued submission and keeps both outcomes in history;
the running request was then aborted separately. Native GPU sampling may finish
after Atelier stops waiting. Workspace checks/tests/format, strict Atelier
Clippy and macOS packaging pass; no real inference was submitted for this check.

Configurable request lifetime (2026-10-30): app-wide Settings is reachable from
the Atelier menu and ⌘,/Ctrl+, on every page. The saved generation wait limit
defaults to 30 minutes (1–1440 configurable), replaces the fixed five-minute
native deadline, and is captured per queued request. Hosted and ComfyUI waits
use the same setting; native startup remains separately bounded. Mock native
tests cover short deadlines and delayed completion with a longer allowance;
settings tests cover migration, persistence and provider switching. Timeouts
do not imply native GPU cancellation. Isolated native UI verification opened
Settings with ⌘, from Projects before opening a document and saved a 45-minute
limit. Workspace tests/checks, strict Atelier/Agentkit Clippy, hosted-only
Agentkit and Clipforge all-target checks, and macOS packaging pass. No image
generation was submitted for these checks.

**Reference library scoping: implemented.** The Atelier reference library is the
shared collection, not every image ever made: generated images, edits and project
imports stay in their project, and only an import into the **All projects** tab
or an explicit **Add to reference library** action adds an image to the
collection. Project-only images are renamed and favorited locally, and the
version strip, before/after comparison and draft restore resolve project and
collection images together. See [Atelier](atelier.md).

1. **The API:** artifact domain model (artifact kinds, variants, scenes,
   beats — the registry), typed operation set + pipeline builder (closed
   vocabulary — `LoadImage`/`Checkpoint`/`Generate`/`Llm`; typed intermediates,
   static validation at `build()`, linear fail-fast stacks), and the
   slice-1 op compilers (create, variant, regenerate, compose, draft, modify).
   — **✅ implemented** (`docs/development.md`); the Qwen-Image-2.1 migration
   removed the mask/inpaint path and made `modify` a reference + grounded
   instruction. Live validation of `modify`/`stack propose` on Linux/CUDA pending
2. **The CLI (`svs`):** ops, `stack run`/`propose`, `--out` golden runs —
   drives and validates the API without the UI. — **✅ implemented**
3. **SQLite foundation:** schema + storage layer (§10) — `crates/studio-core/src/persistence/`
   (`ProjectDb`: versioned migrations via `schema_meta` (v1 §10 tables, v2
   `project_json` for the storyboard), WAL, full-replace snapshot writes,
   lossless round-trip); the CLI persists to `.svs-project.db`; the GUI
   opens/creates projects from the Projects screen and saves via autosave
   or Cmd/Ctrl+S. — **✅ implemented**
4. **The canvas:** the artifact workspace in two states — a grid at rest
   (variants stacked behind their base, in-flight work dimmed and badged in
   place) and an image-dominant detail pane with an inspector rail, whose
   every control is also a command. The tokenized command bar and the
   `Ctrl+K` palette are further views of the same typed vocabulary as the
   `svs` CLI (`/create`, `/variant`, `/regenerate`, `/modify`, `/set`);
   undo/redo via registry snapshots. Revisions carry a `RevisionOrigin` so
   canon and experiment are distinguishable, and `Promote` (`/describe`) is
   what makes an experiment canonical. The kind-grouped sidebar it replaced
   was deleted with it. See the consolidated UI notes in §0. Artifacts carry an optional default generation size
   (`/create … --size WxH`; per-kind defaults otherwise: character 512×768,
   object 768×768, environment/scene/beat 1024×576) that `regenerate`/
   `modify` honor. Visual `create`/`variant` generate the artifact's first
   image as part of the operation (variants reference the base's active
   image); a failed generation keeps the artifact with a failed revision,
   and `/regenerate` is the retry. `modify` runs unattended — an instruction
   edit has no checkpoint and no segmentation step. `/revert` re-activates the
   previous completed image (a Direct op — no render); the history keeps both,
   nothing is deleted, and the reverted-from revision can be re-activated from
   the detail pane. It is on the slash bar, the palette, and `svs op revert`.
   Free-form
   composer messages (no leading `/`) go to the LLM, which proposes an
   operation stack against the closed vocabulary; the proposal is shown in
   an approval modal with per-op toggles before any op runs (LLM proposals
   are gated, per the execution model). The legacy storyboard model was
   removed
   with it (`src/models/`, `src/timeline/`, schema v3 drops `project_json`).
   — **✅ implemented**; `c:` autocomplete and command syntax highlighting
   are implemented. Artboard pan/zoom remains deferred (§6).
5. **Generation quality controls:** configurable per-kind default sizes
   (`[generation.sizes]`), default step count (`[generation] steps`), and
   the Settings screen to edit them live (persisted to the platform `studio.toml`,
   applied to the next generation — canvas and CLI). The LLM expresses
   size/steps/adapters only when the user asks (the visual ops carry
   `size`/`steps`/`loras`; the `resize` op sets the default size and
   regenerates at it) — never invented. The planner discovers the registry
   and adapters via tool calling (`list_artifacts`/`list_loras`) with a
   strict schema on the final stack. `--name` on `variant` (explicit
   naming for variants). A Settings → Services toggle (`[runtime]
   prewarm`) starts the resident server when a project opens, so the first
   generation is not a multi-minute weight load. **Residency** is a launch
   profile per `sdcpp` provider (`resident` / `lowvram` = `--offload-to-cpu`),
   selectable in Settings → Services and validated at config load; the
   measured tradeoff (lowvram ≈9/16 GiB peak for +7%/+0.5% sampling; `--max-vram`
   rejected as too costly for edits) is in `docs/architecture.md`. The native
   server is owned by the machine's **shared lifecycle owner**
   (`clipforge_inference::control` over one shared state directory), the same one
   Atelier and the Clipforge apps drive, so Studio reuses a resident server rather
   than starting a second one; **Start** ensures the configured model is resident
   (replacing a server the owner started) and **Stop** stops the owner's server. A
   server holding the port that the owner did not start is reported as `Degraded`
   (the resident model is named) and stopped with `svs runtime stop --force` —
   identified by the served executable and listen port, not a `pkill` pattern.
   A grounding regression harness (`.svs-e2e/scripts/grounding_suite.py`) runs a
   fixed edit suite against one or more planner models and judges each with
   `svs eval --expect`, so planner quality is comparable by command.
   **Image assessment in the plans:** a driver `expect_vision` step attaches an
   artifact's current image to a question and asserts a substring gate, so a
   plan checks *what* was generated rather than only that an image exists. The
   image-carrying chat call lives in `clipforge-inference`
   (`chat::Client::complete_with_images`); storyfella's `crates/studio-core/src/vision.rs`
   (
   `VisionSession`) adapts config and images to it, and `svs eval` uses the same
   path.
   — **✅ implemented**
6. **LoRA registry:** project-level adapter registry (SQLite
   `lora_registry` table, round-tripped) with per-artifact bindings
   (`/attach-lora`, `/lora-scale`, `/lora-remove` — slash, CLI, and LLM
   vocabulary), config-level use-case bindings (`[lora]`:
   per-kind defaults, one multiplier), discovery
   from `lora_dir` (recursive — subfolders and symlinked model trees included; a
   refresh icon in Settings rescans), and auto-injection into
   every generation (artifact-bound first, then kind default;
   per-run `--lora <file>@<mult>` flags on
   the CLI). — **✅ implemented**
 7. **Error codes and reporting:** a central registry (`crates/studio-core/src/errors.rs` —
    `ErrorCode`, one stable `SVS-<DOMAIN>-<NNN>` per failure, with a short
    message and a remediation hint) that every `thiserror` type implements
    via `CodedError` (transparent variants delegate to the inner code).
    `CodedError::report` builds the `ReportedError` payload the UI consumes —
    a global error toast (code + short message) with a Details action
    opening the full report modal (error text, request context, hint, and
    Copy — button and click-on-text, via `document::eval` with the
    execCommand fallback). The report carries a structured `context` list
    (operation summary, generation prompt, model, size, steps, seed, job
    id, work dir) attached at the failure site — `PipelineError::Backend`
    keeps the `GenerateRequest` context from `CreativeBackend::generate`,
    and the canvas attaches op/seed/work-dir — so a failed generation is
    answerable from one log line (`report.log()`, the single log point,
    emitted by the surface right before showing the report). Wired through
    the canvas ops, free-form proposals, autosave, the Projects screen,
    Settings service/model errors, and the `svs` CLI op paths (which also
    init a tracing subscriber so reports actually log). — **✅ implemented**
8. **Studio integration:** scenes group the timeline; re-sync flows per
   mode (compose beats in the canvas, story sequencing). — **🔧 active**
   Writer/Canvas reconciliation is implemented: valid source order maps authored
   scene/beat keys to persisted UUIDs; invalid drafts retain the last association;
   removals detach artwork. Joint source/registry undo, pending catalog creation,
   source origins and backlinks are connected. Writer keyboard completion,
   exact caret navigation, explicit identity-preserving rename, formatting and
   compiler diagnostics are implemented. Fixed-snapshot visual preview is
   implemented; source changes require applying a new draft. Re-sync remains.
9. **Storyfella DSL (in-repo):** `storyfella-language` (lexer/parser/shared
   analysis) and `storyfella-compiler` (validation/lowering); DSL asset references
   and round-trip to canvas. `storyfella-runtime` owns story execution,
   state and timing, shared by Studio preview and standalone Player.
   — **🔧 first slice implemented.** `storyfella-language` provides lossless
   tokens, a recovering typed AST, source positions, diagnostics, symbol/reference
   analysis and shared completion/navigation. Writer highlighting, scope and asset
   references now consume it. See [language.md](language.md) for the syntax.
   `storyfella-compiler` now lowers versioned, source-mapped plans with sequential
   scene fallthrough, terminal-flow validation and immutable asset readiness;
   `svs story check`/`compile` expose it. See [compiler.md](compiler.md).
   Studio opens/saves authoritative `.sf` files beside project databases.
   Schema v9 exports existing Writer drafts before removing the source column;
   disk conflicts, copy/reload and project pair moves are covered. See
   [data-model.md](data-model.md). Schema v10 persists source/registry associations.
   The shared headless runtime and CLI trace are implemented: typed events,
   manual advance/choice/jump/end/pause/restart, bounded automatic transitions,
   imported-plan validation and exact-plan save/restore. See [runtime.md](runtime.md).
   Studio preview and standalone visual Player share checked resources and
   Dioxus controls: still visuals, text/choices, pause/restart, save/restore and
   source snapshots/Writer return. The 22-step fixture-backed desktop plan checks
   actual execution. Both hosts now use a minimal widescreen stage with
   location artwork, softly shaded text and choices, and secondary controls
   in the options panel. See [runtime.md](runtime.md). Portable image paths,
   packaging/export and expressive media/voice remain planned.
10. **LSP tooling:** `storyfella-lsp` exposes shared language analysis to
    external editors. — **🔧 single-document tooling implemented.** Stdio, full-document
    sync, UTF-16, versioned diagnostics, completion, definitions, references and
    scene/beat symbols, resolved rename, conservative formatting and semantic
    tokens are supported. Explicit catalog export/refresh supplies shared
    compiler diagnostics for unsaved buffers. Workspace imports remain planned.
11. **Self-contained distribution (no user-side dependencies):** one
   downloadable artifact per platform (AppImage/`.deb` on Linux, `.app`
   bundle on macOS, installer/portable zip on Windows) containing the app
   binary and a release-built `sd-server` (CUDA or CPU flavor). The Rust
   spawn/supervision/readiness architecture is unchanged; the only code change
   is resolving `sd-server` relative to the bundle root instead of the
   config-relative path. Model weights stay out of
   any bundle (multi-GiB, licensed) and keep provisioning through
   `model_setup` — or, in the app, Settings → Services → "Model
   files", which drives the same `ModelStore` download with byte-level
   progress. The pinned manifest was verified against the live Hugging Face
   repo (2026-10-01): the three `Comfy-Org/Qwen-Image-2.1` remote paths,
   sizes, and SHA-256 digests all match. Explicitly out: embedding sd.cpp into the Rust process —
   the C API exists, but in-process inference would rework the entire
   runtime-lifecycle machinery for no user-facing gain. **Bundling mechanism
   (verified 2026-09-30):** the CLI itself bundles Linux AppImages —
   `dx bundle --platform desktop --package-types appimage` (run from
   `apps/studio`) — so no extra crate is needed; it lays out an `.AppDir` and
   drives `linuxdeploy`, which it downloads into `~/.local/share/.dx/tools`
   (the only networked step, run with `APPIMAGE_EXTRACT_AND_RUN=1` so FUSE is
   not required). `Dioxus.toml` now
   carries the bundle identity (`[application] name`, `[bundle] identifier`/
   `publisher`/`category`/descriptions); `[bundle] external_bin` and
   `[bundle] resources` are the hooks for the `sd-server`
   payload. Still missing for a shippable bundle: an icon (PNG) and that
   payload, plus resolving it relative to the bundle root. — **⏳ planned**

12. **Studio UI/UX remediation:** shared interaction, visual and accessibility
    rules and the 2026-10-07 app-wide audit are documented in
    [`DESIGN.md`](../DESIGN.md). All 25 findings now have implementation changes:
    direct error reports, shared native dialogs, draft/Quit protection, retained
    command input, truthful settings/runtime feedback, accessible controls,
    readable styling, useful recents/filters and safe preview replacement.
    Model storage now defaults to OS-local Storyfella data in all builds, with
    Change location/Open folder controls and explicit restart feedback. App
    settings live in `studio.toml` and the shared provider registry in
    `clipforge.toml`; old user settings migrate once while preserving originals
    and existing model files.
    The 2026-10-10 workspace pass adds stable icon navigation, visual art-action
    forms using shared typed operations, contextual palette actions, a separate
    command dock and quieter inspector/project surfaces. It fixes command
    clearing and repeated artwork pickers found during live macOS acceptance.
    The dock is now a compact searchable action entry: contextual forms and
    artwork opening via arrows/Enter, explicit assistant requests, and opt-in
    slash argument completion. Native selects match textboxes; field focus
    uses an inset treatment. Rapid Enter dispatch reads current input.
    Explicit edit queries now explain missing images/selections and offer
    generation for the selected artwork before editing, instead of falling
    through to the assistant. Missing/available-image paths passed live checks.
    Writer rename opens a focused form; Problems starts collapsed with
    navigable, filtered diagnostics. Quit uses styled save/discard controls and
    distinguishes transient action searches from unsent requests. Isolated
    macOS checks verified rename/undo, filtering/jumps, request protection and
    description persistence through Save and quit.
    §9 records changes, regression evidence and remaining cross-platform,
    assistive-technology and failure-injection coverage. — **✅ remediation implemented; broader manual acceptance tracked in DESIGN.md**

## 13. Implementation handoff

Design status: the canvas (roadmap item 4) replaced the legacy storyboard
model — the GUI is the artifact registry; the `svs` CLI shares the same
`ArtifactRegistry`/`ProjectDb` core (`docs/development.md`). Work follows
§12 — items 5 (generation quality controls), 6 (LoRA registry), and 7
(error codes + reporting) are implemented; Studio integration (item 8) was
deferred by user reprioritization in favor of generation quality, the LoRA
registry, and UX fixes.

Text and vision are now capability-routed providers: the single `[llm]` section
is gone, replaced by `text` and `vision` capabilities resolved from the shared
`clipforge.toml` (so a key stays out of git) — so a remote, text-only planner no
longer blinds `svs eval`. The
`clipforge-inference` runtime is generalized to a shared native-server
supervisor, and a `llamacpp` provider can start a managed `llama-server` on
demand; weights are never bundled. Settings → **Models** gives each chat
capability an editable provider, base URL, model and API key (switching
`[defaults] text`/`vision` in `clipforge.toml`, and rebuilding the planner client
in place), so pointing the app at a remote or local server is a click and a field
edit rather than a file edit; the same page adds and removes providers. Settings
→ **Services** carries the image engine and the managed `llama-server` files.

### Entry points (existing code)

- `crates/studio-core/src/registry/` — artifact kinds, variants, scenes/beats, revisions,
  drafts; the typed operation set + pipeline builder; the slash
  parser (`slash.rs`) the canvas composer uses; `CreativeBackend` (live
  backend shared by the CLI, the canvas, and Settings).
- `apps/studio/src/state/mod.rs` — AppState: registry, selection, undo/redo stacks.
- `crates/studio-core/src/persistence/mod.rs` — `ProjectDb` (§10 schema; v3 dropped the legacy
  `project_json` column, v8 dropped the `masks` table).
- The shared `clipforge-inference` crate (`../clipforge/crates/inference`) —
  `sd::Client`, the typed client for the
  native `sd-server` `sdcpp` API; `GenerateRequest` is the wire shape. Also owns
  `sd::Runtime` (the resident process), `sd::qwen_image_2_1()` (the served profile),
  and `ModelStore` (provisioning). storyfella depends on it as a path dependency.
- `crates/studio-core/src/runtime/creative_runtime.rs` — `CreativeRuntime`, the adapter over
  the shared runtime + client (readiness, job wait, asset import) the pipeline
  builder wraps; `health.rs` probes the dependent services.
- `crates/studio-core/src/llm/` — LLM endpoint client; the operation registry is the
  planner business logic it will serve.

### Open decisions to settle first (with the user)

1. Execution model: user-typed ops instant + undo; LLM proposals gated by
   approval — proposed, unconfirmed.
2. Approval granularity: whole stack vs per-step checkpoints (draft-text
   approval is the remaining checkpoint).
3. Short-id format for `c:` references.
4. Native mask support for Krea — **removed** with the Krea/SAM migration;
   there is no mask source and no composite fallback.
5. Deferred with SQLite: crate choice (`rusqlite` vs `sqlx`). —
   **resolved:** `rusqlite` (bundled, WAL, one writer connection).

### Pitfalls

- Qwen-Image-2.1 is the only generation model (native, via
  stable-diffusion.cpp); Krea 2 and the SAM/segmentation path are gone.
- Undo is state restore, never pipeline re-execution.
- VLLM vocabulary is closed; LLM steps are soft dependencies.
- The builder is substrate: linear, fail-fast, no branching/DAG in slice 1.
- Storyfella is built from scratch in this repo; `../storyfella` is reference
  material only.

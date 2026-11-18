# Studio and Atelier responsibilities

SF-PLAN-01 decision record · 2026-11-01, updated for the SF-PLAN-07 cutover.
The table records migration ownership and acceptance; implementation evidence is
in the corresponding roadmap milestones.
[ROADMAP](ROADMAP.md#0-current-direction-and-actionable-backlog) owns delivery status.

Studio owns authored story meaning, source editing, references, asset availability,
compilation and Preview. Atelier owns how artwork is made. Both use the same
project and asset identities through SF-PLAN-02–06. Player consumes saved story
plans and resources without requiring either editor or an inference service.

## Current surfaces and destination

| Current workflow / implementation | Destination | Migration acceptance |
| --- | --- | --- |
| Projects: create/open/save, recents, source conflict handling (`ui/projects.rs`, `commands.rs`, `story.rs`) | Studio retains story projects; Atelier opens the same folder and keeps independent visual documents | Existing source/database pairs remain readable; moving a populated shared folder preserves identity and links in both apps. |
| Writer definitions, unresolved references, descriptions, identity-preserving rename, scene/beat associations (`ui/writer.rs`, `studio-core::authoring`) | Studio; shared catalog records definitions independently of images | Define an item without inference or an image; rename preserves identity and attached visual work; invalid drafts cannot remove catalog items. |
| Writer Rewrite/Expand and review (`ui/writing_*`, `studio-core::workflows`) | Studio | Accepted text follows normal source reconciliation and one undo; rejected/pending proposals create no shared requirements. |
| Assets gallery, filters, variant grouping, selection (`ui/assets.rs`, `ui/gallery.rs`) | Studio retains lightweight catalog browsing/previews/readiness; Atelier owns production browsing and artboards | Missing artwork is distinguishable from invalid source; Studio can find base/variation previews and open the intended item in Atelier. Atelier shows both story requirements and independent visual items. |
| New artifact / variant forms (`ui/art_actions.rs`); `/create`, `/variant` | Split story definitions from image creation: Studio defines requirements without rendering; Atelier creates items and their artwork independently | Defining a character/variation never submits a model call. Atelier-created items become referenceable in Studio without automatic DSL insertion. Current operations stay available until the replacement works. |
| Generate, regenerate, edit, resize (`ui/inspector.rs`); `/regenerate`, `/modify`, `/resize` | Atelier composer, queue and artboards | Open the same asset/variation with relevant story context; existing saved artwork remains usable during generation/failure/cancellation. Saving accepted canvas artwork updates that asset. |
| Canon description (`ui/inspector.rs`); `/describe` | Studio owns story-facing requirements; Atelier owns visual prompts and production notes | Changing a render prompt does not rewrite story meaning. Changing a requirement preserves saved artwork and lets the creator decide whether to revise it. |
| Sizes, steps, seed, adapters (`ui/inspector.rs`, generation Settings); `/set`, `/attach-lora`, `/lora-scale`, `/lora-remove` | Atelier production settings | Existing generation provenance and adapter bindings survive migration; model-specific production controls leave Studio only after Atelier can use the retained data. |
| Revision history, activate/revert, experiments, provenance (`ui/assets.rs`); `/revert` | Atelier owns artwork history/selection; Studio reads the selected saved revision and pinned Preview bindings | Saving a new revision preserves older revisions and exact compiled bindings. Pending/hidden candidates never become story artwork. |
| Beat composition / layers (`registry::ops::Compose`) | Studio owns authored staging and asset references; Atelier owns flattened illustrations and visual layer editing | Runtime staging stays typed and source-mapped. Saving an illustration does not flatten or overwrite authored character placement. |
| Artifact-scoped text drafts (`/draft`) | Studio writing tools | Preserve stored drafts during migration; source application stays explicit, compiler-checked and undoable. Remove superseded UI only when its supported use cases have a writing-tools destination. |
| Action entry, palette, shortcuts and log rerun (`ui/palette.rs`, `commands.rs`) | Studio keeps authoring/navigation actions; Atelier owns visual requests | Every entry point follows the ownership above, including keyboard and rerun paths. No hidden command remains that unexpectedly starts migrated generation. |
| Approved `.sfart` export/import (`studio-core::artwork`) | Shared-project ordinary save replaces the routine handoff; standalone exchange remains a separate explicit file workflow while used | Shared linked saves need no export/import, approval or publishing gate. Existing packages and project images are preserved during adoption. |
| Preview, diagnostics, pinned revisions, save/restore (`preview.rs`, compiler/runtime/playback-panel) | Studio and standalone Player | Both consume the same saved resources; no Atelier process or generation dependency. A newer image requires deliberate Preview replacement. |
| Service/provider setup and downloads (`ui/settings.rs`, `ai_config`, `paths`) | Clipforge owns shared provider credentials/setup; Agentkit implements inference contracts; apps retain appropriate request preferences | Preserve shared configuration, keys, deployment behavior and platform identities. Story authoring works with no image provider. Studio text-provider setup remains usable until an equivalent setup path exists. |
| `svs` operations and driver plans (`apps/cli`, `apps/studio/src/driver`) | Keep public CLI behavior stable during GUI migration; review all callers before removing shared operations | Existing CLI commands, error codes, environment overrides and deterministic plans still work. Update plans when GUI ownership changes; do not delete domain functionality merely because a button moves. |
| Atelier references, layers, masks, candidates, queue, tabs and recovery (`../storyfella-canvas/apps/atelier/src`) | Atelier | Shared links augment existing visual documents; standalone `.atelier` projects and the explicit shared reference library remain useful. Project images do not enter the global library automatically. |

## Shared ownership boundaries

The GUI-free application domain owns project/catalog validation and persistence.
Both editors call it; neither duplicates catalog identity or rewrites the other's
files. Agentkit receives inference/workflow inputs, not Storyfella project
ownership. The language, compiler, runtime and LSP remain independent of GUI and
inference services.

Story requirements, saved images and editable visual documents are separate:

- A requirement may exist without any canvas or image.
- A canvas may exist without being required by a story.
- A saved asset revision may remain in use after a requirement changes or is removed.
- Explicit variations are independent identities grouped under a base; do not
  expand outfits and expressions into an implicit Cartesian product.
- Removing source usage detaches a requirement; deleting an asset or artwork is
  a separate explicit operation. Source undo cannot erase visual work saved by
  another editor.

## Cutover rule

SF-PLAN-07 removes Studio's migrated image controls, visual command composer,
operation reruns, revision selection and approved-package import UI. It retains
source/catalog definitions, explicit variation definitions, names/descriptions,
saved previews/history, readiness and contextual opening. Existing images,
revisions, drafts, adapter metadata and provenance remain stored. Core/CLI visual
operations and Atelier's standalone `.sfart` export retain actual callers.
Studio keeps text-provider setup and machine service/file administration, but
removes request size/steps/LoRA controls and image prewarm on project open.
The earlier table's implementation paths describe the pre-cutover inventory;
removed Studio controls are no longer alternative entry points.

SF-PLAN-02 establishes portable project storage. SF-PLAN-03 establishes shared
identities. SF-PLAN-04–06 then supply concurrent editing, contextual opening and
ordinary-save refresh. Only SF-PLAN-07 removes Studio's migrated production
controls and unused implementations. Until then, preserve working controls and
existing user data; do not replace them with nonfunctional Atelier buttons.

Cutover acceptance exercises both directions: define in Studio → make/save in
Atelier → preview in Studio, and create in Atelier → reference in Studio. Repeat
with a variation, rename, unsaved source, failed save, reopening and folder
relocation. This document defines the acceptance contract; executable evidence
belongs to the corresponding roadmap milestones as they land.

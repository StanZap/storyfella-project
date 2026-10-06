# Story authoring UX plan

Start with the existing desktop app and a short guided story. The first pass
should answer whether writing, finding assets, and changing their appearance
feel connected. Use the feedback to shape scene synchronization and preview.

## 1. Experience the current app

Run from the workspace root:

```bash
cargo xtask plan config/plans/ui-tour.yaml --mode guide --log
```

Inside Studio, the same plan is **Plans → Story authoring walkthrough → Guide**.
Allow roughly two minutes. The guide creates a fresh **Keeper of the Low Light**
project and walks through:

1. Watch a new beat being typed, then read a scene and move between beats.
2. Compare resolved references with assets that still need to be created.
3. Explore the cast and variants on Canvas.
4. Inspect a character's canon, revision history, and generation controls.
5. Change generation settings and the description through real commands.
6. Browse the palette, then return to the editable `.sf` story.

The assistant card supports pause, resume, speed changes, and stop. Its
**Typing** slider adjusts visible typing from 1 to 200 characters per second
(the tour starts at 60), independently of narration speed. Pause
before editing or issuing your own commands. After the tour, edit dialogue,
inspect a variant, then reopen the saved project to try persistence.

This walkthrough needs no models. Pictures and revision history are sample
fixtures, including a stand-in landscape in the character detail. Rendering
is simulated and reuses that picture. Canon and setting changes are real.
Live image generation can be explored separately with `edit.yaml` when its
image backend is configured.

Evaluate whether you can identify the current scene and beat, understand which
assets are missing, reach a referenced asset, and distinguish canon changes
from image revisions. Note any step where you lose your place or need to
guess what to do next. The guide finishes in the Writer for hands-on review.

## 2. Connect an authored scene to Canvas

Next, synchronize scene and beat identities from `.sf` into the registry.
Make a referenced asset reachable from the current beat and let asset creation
resolve the reference immediately. Preserve artwork and revisions when source
is edited; show positioned diagnostics for invalid drafts.

Acceptance scenario: write a harbor beat using Mara and a lantern, create its
missing asset, revise the dialogue, save, and reopen. The scene order, beat
identities, and artwork stay consistent across Writer and Canvas.

## 3. Play the story

Build the shared story execution runtime, then expose it in Studio preview
and Player. Start with dialogue, thought, narration, scene fallthrough, choices,
`goto`, and `end`. Show the source position of the current beat in Studio.

Acceptance scenario: run a short two-scene story, choose a route, restart,
and follow the other route. Preview and Player produce the same sequence.
Report pending assets and unsupported directions before starting playback.

## 4. Give the scene its intended presentation

Add media placement, timing, and voice after the execution loop works.
Use artwork appropriate to each character and location so visual review can
judge the story rather than fixture images. Keep asset paths portable when
moving a project.

Acceptance scenario: preview and share a complete lighthouse scene with its
images and voice, then open the moved project with its media intact.

The first walkthrough is available now. Scene synchronization, playable
preview, and presentation are implementation milestones, not steps the current
guide pretends to execute.

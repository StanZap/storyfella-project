# Atelier iteration acceptance · 2026-10-10

The core image iteration loop works in the macOS app with real Replicate jobs.
The discovered draft-protection and version-identification gaps are corrected.
This is an operator walkthrough, not a first-time-user usability study or a
Linux CUDA acceptance run.

## Scenario and evidence

Use the forest-bookshop project in isolated `.svs-e2e/atelier-bookshop-demo` data.
The existing morning original and evening edit were generated through Atelier.
This pass used the rebuilt icon UI and app-data key storage, with the user entering
the Replicate key in Settings. Two further jobs completed:

- Evening → evening with a hanging bookshop sign.
- Morning → autumn morning, an independent branch from the original.

The saved `lanternleaf-iteration-proof.atelier` has four embedded references.
Metadata checks confirm two children of the morning original and one child of
the evening version. Both new results exported through the native UI as 1536 ×
1024 PNGs: `lanternleaf-evening-sign.png` and `lanternleaf-autumn-morning.png`.
Global library names were saved through Image details; project-local reference
names remain independent.

| Task | Observed result |
| --- | --- |
| Reopen existing project | Artwork and image layer recover. |
| Select original or edited version | One version-strip activation opens the source. |
| Edit again | Real hosted job completes and stores a new immutable original. |
| Branch from original | Autumn result retains morning as its parent. |
| Browse while generating | Original remains selected; the evening job keeps its evening source. |
| Compare with parent | Before / after opens directly and fits both images. |
| Edit failure | Missing-key error leaves prompt editable and source intact. |
| Save mask, switch versions, return | Draft reloads; Undo removes the test stroke. |
| Resize | Image actions, prompt and submission remain reachable at about 1060 × 732 logical pixels. |
| Name and export | Library names persist; full-resolution PNGs save to the isolated directory. |
| Save project | Four embedded references and their correct parent links persist. |
| Restart and native Open | Rebuilt app restores artwork, active edit source and unsent text. Native Open of the four-version proof project completes; library remains responsive. |
| Quit with unsent prompt | Evening-sign draft survives quit/restart without submission. Unit tests cover project/recovery writes and failed save retention. |
| Switch source with a prompt | Morning, evening-sign and New image restore independent drafts. Canvas undo/redo retains the latest prompt text. |
| Identify versions | Chips use saved names rather than list positions; accessible names/tooltips identify each parent. |
| Reuse prompt | Named icon restores the inspected original generation prompt in the editable field. |
| Save / Open while typing | Global actions and native File menu route ⌘S / ⌘O from the prompt editor; a portable draft-bearing project saves and reopens. |
| Local providers | Qwen exposes generation/editing; Krea switches to generation and disables editing. Missing local configuration preserves the source and draft. |

## Findings and corrections

1. **P1: preserve unsent edit drafts.** Persist drafts with their source and mode,
   recover them on restart and protect against failed writes. Switching versions
   should restore that version's draft. Test rapid typing, navigation, quit,
   restart and save failure. **Corrected:** source-bound project/recovery fields,
   observed native text, final capture before save/navigation/quit, restored
   source after startup, and draft preservation through canvas history.
2. **P2: stable version identity and lineage.** Hash order is not creation order.
   Replace positional numbers with durable identity and make the branch parent
   understandable without opening provenance. Test a chain plus sibling branches.
   **Corrected:** saved image names replace positional labels; parent names are
   included in tooltips and accessible labels. New edits receive a saved name.
3. **P2: zoom action mismatch, corrected.** The text control displayed Fit or the
   current percentage but always switched to actual size. It now says 100%;
   the separate fitted-view icon remains named Fit image.
4. **P2: prompt reuse.** Completed prompts live in provenance but have no direct
   reuse action. **Corrected:** the named reuse icon loads the version's prompt.
5. **Native Open retested.** The initial UI automation service timed out after
   selecting the saved proof project, and subsequent AX and screenshot calls
   timed out. App inventory still reported Atelier running and its log had no
   error. This does not distinguish an app stall from an automation failure.
   Resetting the stale automation binding restored access. The rebuilt native
   picker reopened the proof project and subsequent library navigation worked.

## Validation of this correction

Atelier's 37 non-device tests pass, including source-bound draft persistence,
save-failure retention, canvas history and both native architecture/capability
configurations. Strict Atelier Clippy and Storyfella workspace checks/tests pass.
Clipforge's required `cargo xtask check` passes; its public native-client loopback
tests cover Qwen generation/editing, Krea generation, source bytes, sampling,
results and cancellation. The relative-executable resolution fix is exercised
by Atelier's providers test and Clipforge's configured-path test.

The rebuilt native macOS app was used for independent prompts, source switching,
new/edit switching, prompt reuse, quit/restart recovery, project Open and local
provider selection. No additional paid job or real CUDA inference was submitted
in this correction pass. Setup instructions are in [Atelier](atelier.md#local-qwen-and-krea-on-linux-with-cuda).

The portable `lanternleaf-keyboard-proof.atelier` contains four references, an
unsent evening-sign draft and its active source identity. It was saved through
⌘S while the prompt owned focus and reopened through ⌘O. Save/Open/Save as now
use global actions; the old canvas-only shortcut handlers were removed.

## Repeatable acceptance gate

Reopen the proof project; select Evening; draft a localized change; compare;
submit; browse Morning while it runs; open the completed result; check its parent;
branch from Morning; save and export; quit and reopen. Results must retain
originals, source identity, names, prompts, masks and branch relationships.
Repeat with an unsent draft and injected storage/provider failure.

Before calling the UX good, have a first-time user complete that loop without
coaching and record wrong-source edits, lost text, recovery, and action discovery.
Keyboard-only, VoiceOver and Linux/Windows native acceptance remain outstanding.
No vision-model quality evaluation was performed in this pass; provider completion
and correct metadata do not by themselves establish artistic fidelity.

# Atelier Layers sidebar design specification

**Status:** Proposed design for implementation and review. This describes the Layers sidebar, not a redesign of the rest of Atelier. The generated images are visual references, not a source of feature requirements.

## Goal

Make it easy to find artwork, understand what is selected and what will receive paint, and perform the next relevant action without scanning unrelated controls. A user should be able to distinguish a layer, a group and a multi-selection at a glance.

## Design decisions

- The sidebar has two regions: **Layers** and a **selection inspector**. The layer list stays above the inspector in every state.
- A row's selection and its paint-target status are independent. Selecting an image or group must not change the paint target.
- The inspector describes the current selection. A group selected as a whole is one item; its children are individual items when editing the group.
- A transform changes artwork, not the artboard or viewport. Position and size describe the selection's axis-aligned bounds in artboard pixels, including after rotation. Position can be negative or outside the artboard.
- Numeric edits commit individually on Enter or focus loss, with Escape to cancel and undo for committed changes. The current shared **Apply values** and separate name checkmark are removed. Rotation remains a *relative* operation with a **Rotate** action; it is never presented as the current absolute angle.
- Aligning items relative to each other and positioning the selection as a whole on the artboard are distinct operations. The UI must name the target and scope.
- No logos, large repeated thumbnails, decorative badges, or invented Merge Down/Rasterize actions are added.

## Layout and visual system

Target the current approximately 360 px sidebar width and the application's supported minimum window size. Use the existing application sans-serif typeface and color palette: neutral surfaces and text, with wine used for selection, focus and intentional emphasis. Use the same row height, control heights, icon size, alignment and spacing throughout. Text and controls must remain legible at normal app scale; selection, hover, focus and disabled states must not depend on color alone.

```text
Layers                                  +   ×
┌────────────────────────────────────────────┐
│ [preview]  Portrait              eye  lock │  selected
│ [preview]  Sketch  · Paint       eye  lock │
│ ▾ [group]  Characters            eye  lock │
│    [preview] Hero                eye  lock │
└────────────────────────────────────────────┘
──────────────────────────────────────────────
Portrait                            Image

Transform
X [       ]  Y [       ]
W [       ]  H [       ]  [link]
Rotate by [       °]             [Rotate]
Opacity   [                 %]

Image
[Crop image]     [Image and versions]
▸ Arrange
```

This is a layout guide, not pixel art. Keep the layer list visible while inspecting properties: give the list a sensible bounded height and independent scrolling when needed; allow the inspector to scroll without hiding its heading. Do not nest competing scroll areas that trap wheel or keyboard input. On a short window, both regions remain reachable and no controls are clipped. Long names truncate in the row, expose the full name accessibly, and remain editable in the inspector.

## Layer list

**Header:** “Layers”, add-layer button and close button. Keep import available through a labeled menu or tooltip-accessible action near the header. Duplicate and destructive operations belong in the context menu; do not make the primary add button perform an ambiguous operation.

**Rows:** left-aligned thumbnail or recognizable type placeholder, name, then visibility and lock controls in fixed columns. A selected row has a subtle background and a non-color marker. A distinct active-row marker appears only when it conveys useful information within a multi-selection; it must not imply that the active row is the paint target. The complete row is the selection target; its buttons perform their own action without changing selection. Row positions must not shift when selected, hovered, hidden or locked. Preserve drag-to-reorder with an explicit insertion indicator and keyboard-accessible reorder actions in the context menu.

**Paint target:** show a small brush marker and accessible “Paint target” label on the target row, including when it is not selected. The selected layer's inspector offers **Set as paint target** when eligible; the current target shows **Paint target** as status. Choosing a target is explicit and never occurs just because a layer is selected. Painting an image-backed layer remains supported when explicitly chosen. A pending candidate cannot be chosen until accepted. If there is no target, explain at the point of drawing that the selected paint layer will be used or a new paint layer created above the selected image, matching current behavior. Hidden or locked targets remain identifiable; drawing is blocked with an actionable message to show or unlock the target.

**Selection:** click selects one item; Shift-click selects a contiguous range; Command-click on macOS or Control-click elsewhere toggles an item. The same selection appears on canvas and in Layers. A group row selects the complete group; selecting a child enters child-editing mode. A compact **Done editing group** control returns to the group. Expansion only changes list visibility and does not change selection. When the inspector has no selection, retain the layer list and show “Select a layer to view its properties.”

**Groups:** show a disclosure control, a group icon and an indented child list. Group-level visibility and locking remain separate from child settings. Group opacity applies to the composited group, not individually to its children. No nested groups are introduced. Existing grouping requires at least two adjacent ungrouped layers so stacking is preserved; unavailable actions explain why. Ungroup is available for a whole selected group only and must preserve current appearance: show and unlock it and set group opacity to 100% first. Adding an adjacent layer and removing an edge child remain available where supported, without silently changing stacking. If older documents contain noncontiguous group members, preserve their rendering and explain any unavailable group operations rather than silently rearranging them.

## Inspector states

| Selection | Heading and controls | Important behavior |
| --- | --- | --- |
| None | “Select a layer to view its properties.” | No stale values or destructive actions. |
| Empty paint layer | Name, “Paint layer”, opacity, paint-target status; a short “Draw on this layer to enable transforms.” | No fabricated zero-size geometry. Layer opacity remains editable. |
| One paint layer with artwork | Name, “Paint layer”, transform, opacity, paint-target action, Arrange. | All strokes on that layer transform together. |
| One accepted image | Name, “Image”, transform, opacity, **Crop image**, **Image and versions**, Arrange. | Crop remains reversible; replacement continues through the existing image/version flow, with Fit and Fill explained there. |
| One whole group | Group name, “Group”, combined bounds, group opacity, Arrange, **Ungroup**. | No child opacity is implicitly changed. Constraints on ungrouping are visible. |
| Multiple items | “N items selected”, combined bounds, common or mixed opacity, contextual Group and Arrange. | A whole group counts as one item; a selected child counts as one item. Never treat different opacities as 0% or 100%. |
| Locked selection | Same values, read-only transform/opacity, a clear locked indicator and an **Unlock** path when possible. | Do not silently change only unlocked items. Group locks and child locks both apply. |
| Hidden selection | Identify the item as hidden and provide **Show**; retain access to properties where the operation is supported. | Hiding must not change selection or paint target. |
| Pending candidate | Prominent “Pending review · excluded from export” with **Accept**, **Save as reference**, **Hide/Show** and **Discard**. | Preserve the existing review flow and provenance; do not offer it as a paint target until accepted. Pending review is not represented as accepted artwork. |
| Mixed group/layer selection | Combined bounds and Arrange; no single group-opacity control. | Do not silently turn group opacity into child opacity. Multi-opacity editing is offered only when its targets can be unambiguously stated; otherwise explain the limitation. |

Show the selected item's name as the inspector heading. Rename via a clearly discoverable inline edit or context-menu action; commit a valid 1–128-character name on Enter or focus loss. Escape restores the original. Group names follow the same rule and preserve uniqueness constraints.

## Transform and opacity

- Show X/Y and W/H in two compact rows, with units visible. Link proportions beside W/H; show its on/off state accessibly. Changing one dimension with the link on updates the other proportionally around the selection's top-left anchor; direct canvas resize follows the existing handle and modifier behavior.
- Inputs show a sensible precision without silently changing geometry on focus or blur. Preserve full internal precision until a user commits a change. Accept finite numbers; width and height must be positive, opacity 0–100%. Invalid input keeps the document unchanged and shows an inline reason and the last valid value. Switching selection must not apply an invalid draft or trap focus.
- Enter or blur commits only the edited field as one undoable change. An unchanged value creates no undo entry. Selection changes and incoming asynchronous work must not overwrite a focused, uncommitted draft; cancel or resolve it explicitly. When bounds change due to drawing, canvas transform, undo or redo, update unfocused fields immediately.
- An opacity field shows the shared percentage; **Mixed** is a placeholder, not a dropdown value. Entering a valid value explicitly sets all eligible selected layers. A group selection edits group opacity, never its children.
- **Rotate by** is a signed relative angle, initially 0°. Activate **Rotate** or press Enter to apply once around the selection center; reset the input to 0° afterward. Escape cancels the draft. A future absolute rotation control requires a trustworthy model and is outside this spec.
- Snapping affects direct manipulation and can be turned on or off; exact numeric entries must be applied exactly, with no hidden rounding to a grid. This changes current numeric snapping behavior and requires implementation work.

## Arrange

Arrange is collapsed by default and exposed only when the selection has usable artwork bounds. Its expanded state has clearly labeled sections:

1. **Align items to each other** — visible for two or more selection units. Left, horizontal center, right, top, vertical center and bottom use the combined selection bounds as reference. Whole groups count as one unit.
2. **Distribute** — visible for three or more units. Distribute horizontal or vertical gaps within the current outer span. For two or more units, a separate exact-gap field and horizontal/vertical actions set a nonnegative pixel gap while retaining the first item's position.
3. **Position selection on artboard** — move the combined selection as one unit to the artboard's left, center, right, top, middle or bottom. Preserve relative positions; grouping is not required.
4. **Align each item to artboard** — advanced action for two or more units. Explicitly say that centering each item can overlap them; present a confirmation before executing a potentially overlapping center alignment, with a concise explanation. Other actions must not silently choose this scope.

Every icon has an accessible name and tooltip naming both direction and target. Aligning or distributing a selection containing an item without bounds is unavailable with a reason; do not skip that item. Locked selections cannot be transformed. Disabled actions should either have an adjacent explanation or be omitted when irrelevant. Avoid repeating similar icon rows without visible scope labels.

## Accessibility and interaction

- All row actions and inspector controls are keyboard reachable with a visible focus indicator. Arrow-key nudging applies only when the canvas has focus, never while a text field is being edited. Tooltips supplement, not replace, accessible names.
- Provide semantic selected, expanded/collapsed, visibility, lock and paint-target states to assistive technology. Name buttons by action and object, such as “Hide Portrait” or “Set Sketch as paint target.”
- Do not rely solely on hover to expose essential actions; touchpad and keyboard users need equivalent access. Avoid sudden reflow when a control becomes visible.
- Give errors next to the affected field or action and preserve the user's input. Announce important state changes, errors and candidate outcomes without interrupting unrelated editing.
- All document changes remain undoable under existing history rules. Do not overwrite the user's current selection when an asynchronous image import or generation result arrives.

## Acceptance criteria

1. At the supported minimum window size, list, inspector heading and all actions are reachable without clipping; long lists and long names remain usable.
2. A new empty paint layer, a drawn paint layer, an image, a whole group, a group child and a multiple selection each show the correct heading and controls. No empty layer shows fake geometry.
3. Select an image while a separate paint layer is targeted: the image is selected, the paint target remains unchanged and both states are identifiable. Repeat with a hidden or locked target and with a pending candidate.
4. Edit X, W and opacity by keyboard, cancel a draft, enter invalid input, switch selection, and undo. Only intentional valid commits change the document; numeric positions do not snap.
5. Draw on the selected paint layer, transform it on canvas, undo and redo: unfocused inspector values track the actual artwork bounds without reselection.
6. For three separate images, distribute horizontally, then position the selection at artboard center and bottom; their relative positions remain intact. Align-each-to-center warns before overlapping them.
7. Select a group as a whole and then edit a child. Group opacity stays distinct from child opacity; grouping/ungrouping restrictions are explained and stacking does not change unexpectedly.
8. Review a generated candidate, crop an accepted image, and import an image while changing selection; existing review, crop, provenance and asynchronous destination behavior remains intact.
9. Operate the list and inspector without a mouse and inspect selected, locked, hidden, focus and pending states with assistive technology. Check contrast at normal and disabled states.

## Implementation notes

The inspector commits numeric edits on Enter or focus loss and applies canvas snapping only to direct manipulation. Layers supports range and toggle selection. Background artwork imports preserve selection and paint target. Use the native app for visual and keyboard acceptance and keep the candidate review and export behavior intact.

# Calibration

## Photo workflow

1. In the control panel pick a **Pattern** (Diagnostic, Alignment grid, Flat gray, White) and toggle
   **Blend ramps** as needed.
2. Photograph the wall from the audience position: one shot of the whole wall, plus close-ups of
   each seam. Hold the camera steady and avoid zoom. A phone is fine. You can keep the photos in a
   `calib/` folder in the repo, which is git-ignored.
3. Open **Calibration editor…** and adjust each projector's quad until the patterns coincide in the
   overlaps. Use "Wall (ideal)" preview as a background if you like.
4. Repeat with a new round of photos until the seams disappear.

Before adjusting geometry, check the box mapping with the Diagnostic pattern: if the tiles appear in
the wrong order enable `reverse_order`; if the content inside each tile is flipped, enable `mirror`.

## Reading the Alignment pattern

- Projector 1 is tinted red, 2 green, 3 blue; an overlap shows the mix (yellow / cyan).
- A white grid every 100 canvas px, a thick line every 500 px; the yellow cross is the canvas centre.
- Dots near the top-middle of each slice give the projector number (1, 2, 3 dots).
- A bright coloured bar marks the exact edge of each projector's slice (drawn unblended).
- In an overlap, both projectors' grid lines must coincide; doubled lines mean misalignment.

## Diagnostic pattern

- Pure RGB per projector (1 = red, 2 = green, 3 = blue), with numbered grids and rulers.
- Seam ring and cross targets at each overlap centre.
- Blend ramps are off for this pattern, so both projectors stay at full intensity. Aligned overlaps
  add up to yellow or cyan; misaligned lines stay as separate colours.
- Use it to check box mapping (tile order, mirroring) and to measure the real overlap of each seam.

## Calibration editor

- A wall canvas at the `canvas` aspect with a margin so spilled corners stay reachable. Toggle
  **Show preview** for the live Wall preview as background.
- Each projector is a coloured quad: drag a **corner**, an **edge midpoint** (moves both ends) or the
  **interior** (moves the whole quad). The off-screen part is hatched and the spill is shown.
- Mouse wheel zooms, right- or middle-drag pans, **Fit** resets the view.
- Arrow keys nudge the selection (Shift ×10, Ctrl ×0.1); Tab cycles projectors; Ctrl+Z / Ctrl+Y
  undo and redo. Corner coordinates can also be typed, with a readout in metres from `wall_m`.
- Edits go into the live config and autosave like everything else.

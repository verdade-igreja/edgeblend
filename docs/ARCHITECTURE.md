# Architecture

## Pipeline

```
source display ──► DXGI Desktop Duplication ──► D3D11 pixel shader ──► borderless topmost window
 (the canvas)        (GPU, no CPU copy)          (blend.hlsl)           on the output display
                                                                        └─► video-wall box ─► N projectors
```

Windows renders a desktop on the **source** display (the canvas, typically a wide virtual display).
edgeblend captures it with DXGI Desktop Duplication, runs a pixel shader that cuts the canvas into one
image per projector and applies the blend, and presents the result in a borderless, topmost window that
covers the **output** display. That display feeds a video-wall box or splitter, which slices the signal
across the projectors.

Source and output must be on the **same GPU**: the captured texture is sampled directly by the shader,
so both displays need to belong to the same adapter. edgeblend checks the adapter LUIDs and refuses to
start otherwise. (Cross-GPU support is not implemented.)

## Crate layout

| Crate | Role |
| --- | --- |
| `edgeblend-config` | `Config`, `Pattern`, `Preview`, per-projector geometry; TOML load/save and validation |
| `edgeblend-geom` | Pure math: homographies and the even-split seed quads (host-portable, unit-tested) |
| `edgeblend-gpu` | DXGI display enumeration, D3D11 renderer (capture, swapchain, preview readback), `blend.hlsl` (hot-reloaded when it changes on disk) |
| `edgeblend-engine` | The blend thread: output window, global hotkeys, frame loop, pacing, recovery |
| `edgeblend-ui` | egui control panel, calibration editor, tray icon |
| `edgeblend` | Binary: CLI flags, DPI awareness, single instance, watchdog, log file |

The config autosaves to `%APPDATA%\edgeblend\blend.toml`; `blend.example.toml` documents every key.

## Geometry model

- **Wall space** is normalized `q ∈ [0,1]²` over the lit screen (x left to right, y top to bottom).
  `canvas` (pixels) defines the content coordinates; `wall_m` (metres) is only used for readouts and
  the editor aspect.
- Each projector is a **quad**: four corners (TL, TR, BR, BL) giving where its image lands on the wall.
  Corners may lie outside `[0,1]` when the image spills past the screen.
- Each projector has a homography from its local `uv ∈ [0,1]²` to wall space (and the inverse), in
  `edgeblend-geom`.
- Rendering, per output pixel: output pixel → projector `uv` → wall point `q` → content at `q`.
  If `q` is outside `[0,1]²` the pixel is black, which hides spill.
- **Blend** is distance-weighted in each projector's own `uv`: `a_j = min(u, 1−u)^curve` when the
  point is inside projector `j`, else 0, and `w_i = a_i / Σ a_j`. Weights sum to 1 everywhere, so
  ramps span the real overlap of each seam automatically. The colour is scaled by `w^(1/gamma)` so
  the light output sums to a constant.
- The output signal is split into N equal tiles (one per projector). `reverse_order` reverses the
  tile order and `mirror` flips the whole signal horizontally, for boxes that do either.
- Initial quads come from an even horizontal split with `overlap_frac`; fine-tune them in the
  calibration editor.

## Frame pacing

- The swapchain is flip-model and **frame-latency waitable**, with `SetMaximumFrameLatency(1)`. The
  loop waits on the swapchain's waitable object before each frame, so it never queues frames ahead of
  the display.
- After the wait, `AcquireNextFrame` is called with a short timeout (`CAPTURE_WAIT`, a few ms in
  `edgeblend-engine`) so a source frame that lands just after the wait is still shown this refresh.
  Mouse-only updates are ignored. Alt+Enter is disabled.
- The status line reports, per second:
  - **repeated**: presents with no new source image. Normal on a still desktop; only read it while
    video plays.
  - **skipped**: source images that were never shown (from `AccumulatedFrames`).
  - **missed vblanks**: refreshes where the output got no new frame from edgeblend (from
    `GetFrameStatistics`). This is the in-app equivalent of PresentMon's hitches.
  - **Hz**: source refresh rate → output refresh rate.

### Measuring with PresentMon

Run PresentMon from an **administrator** shell while video plays on the wall:

```
PresentMon.exe --timed 12 --terminate_after_timed --output_file pm.csv
```

Look at the rows for the edgeblend swapchain: frames longer than ~25 ms, frames never displayed, and
the present mode.

- **Composed: Flip**: the window is composed by DWM. It inherits DWM's timing and frames can be
  replaced before they are shown.
- **Hardware: Independent Flip**: the swapchain scans out directly, bypassing DWM composition. This is
  the best case. If you stay on "Composed", look for anything overlapping the output display (other
  topmost windows, the taskbar, overlays).

### Why not exclusive fullscreen

DXGI leaves exclusive fullscreen whenever the window loses focus, and the operator normally works on
another display, so it would keep dropping out. Instead the window covers the whole output display
exactly and uses a flip-model swapchain, which lets Windows promote it to independent flip.

### Refresh-rate drift

If the source and output run at slightly different rates (for example 60 Hz vs 59.95 Hz) they drift
and an occasional frame is repeated or skipped. Compare the Hz shown in the status line. Virtual
display drivers often accept only integer refresh rates, so the practical fix is usually to set the
output (box) mode to the same integer rate rather than the virtual display to a fractional one.

## Recovery

The engine rebuilds the output window and the renderer after a GPU device loss (reset, driver
update), on `WM_DISPLAYCHANGE` (resolution, refresh rate, display added or removed) and when the
output display is missing. While the display is absent it retries every 2 seconds.

## Background app

- The release build uses the GUI subsystem (no console). Closing the panel hides it to the **tray
  icon**; the tray menu has Show panel, Start/Stop blending and Quit (the only way to exit).
- **Single instance**: a second launch brings the existing panel to the front.
- **`--watchdog`** runs the app as a child and restarts it after a crash (backoff from 2 s up to
  60 s). It exits when the app is quit normally.
- Fatal errors, panics and watchdog restarts are written to `%APPDATA%\edgeblend\edgeblend.log`.
- **Autostart** is a per-user Scheduled Task at logon (`tools/autostart/install-autostart.ps1`),
  not a Windows service. Services run in session 0, which has no user desktop; Desktop Duplication
  captures the logged-on user's desktop and the blend window must live on it.

## Video-wall boxes

Many boxes slice the input into N equal tiles, one per output. Some reverse the output order
(tile 1 appears on the last projector): use `reverse_order`. Others mirror the picture: use
`mirror`. The calibration Diagnostic pattern (see [CALIBRATION.md](CALIBRATION.md)) makes it easy to
tell which one your box does, because a mirror flips the content inside each tile while a reversal
only swaps the tiles.

Note that each projector only receives `input width / N` pixels, so the effective wall resolution is
limited by the box input mode.

## Decisions

- **Capture and re-render** rather than vendor warp/blend APIs. Vendor APIs are tied to one GPU
  brand and to fullscreen exclusive applications; capturing the desktop works on any GPU and with
  any application.
- **Calibration by photo loop**: the app shows patterns, you photograph the wall, and you adjust the
  quads in the editor. Automated camera-based calibration is a possible future step.
- **Logon task, not a service** (see above).
- **No exclusive fullscreen** (see above).

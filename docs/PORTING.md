# Porting plan: macOS and Ubuntu

Status: planned, not started. Windows remains the reference platform.

## Where the code stands

About 70% of the code is Windows-specific. `edgeblend-config`, `edgeblend-geom` and most of the
egui panel and calibration editor are already portable. The Windows-only parts:

| Area | Windows implementation | File(s) |
| --- | --- | --- |
| Display enumeration | DXGI adapters and outputs | `edgeblend-gpu/src/display.rs` |
| Capture | DXGI Desktop Duplication | `edgeblend-gpu/src/renderer.rs` |
| Rendering | D3D11 + `blend.hlsl` | `edgeblend-gpu/src/renderer.rs`, `blend.hlsl` |
| Output window, hotkeys, cursor | Win32 | `edgeblend-engine/src/lib.rs` |
| Tray icon | `Shell_NotifyIconW` | `edgeblend-ui/src/tray.rs` |
| Single instance, console, watchdog | Named mutex, `AttachConsole`, `CREATE_NO_WINDOW` | `edgeblend/src/*.rs` |
| Autostart, installer | Scheduled Task, Inno Setup | `tools/autostart`, `installer/` |

## Approach

Split platform code from portable code. Each OS provides capture, output and display
enumeration; the blend maths, shader, config, UI and engine loop are shared.

```
              ┌─ Windows: DXGI Desktop Duplication ─┐
capture  ─────┼─ macOS:   ScreenCaptureKit          ┼──► GPU texture ──► blend shader ──► output window
              └─ Ubuntu:  PipeWire (portal) / X11   ┘                   (one WGSL source)  (per platform)
```

- The blend shader is written once in **WGSL**.
- **Windows keeps the D3D11 renderer.** The HLSL it needs is generated from the WGSL by `naga` at
  build time.
- **macOS and Linux use a `wgpu` renderer**, on Metal and Vulkan.

## Decisions

1. **macOS first**, then Ubuntu. ScreenCaptureKit is simpler than Wayland capture, and a Mac is
   enough to develop and test against a second display.
2. **Unsigned macOS builds first.** Signing and notarization (Apple Developer Program, $99/year)
   are added later. Until then users open the app with right-click → Open, or
   `xattr -dr com.apple.quarantine edgeblend.app`. Known cost: macOS may ask for the Screen
   Recording permission again after each update of an unsigned app.
3. **Windows stays on D3D11** for now. Reasons, compared with `wgpu` on Windows:
   - `wgpu` has no D3D11 backend; it uses D3D12 (or Vulkan). Desktop Duplication returns D3D11
     textures, so every frame would need D3D11 → D3D12 interop through a shared handle and a fence.
     That means unsafe glue code, more synchronisation, and possibly an extra GPU copy. The D3D11
     path samples the captured texture directly.
   - The frame-pacing controls edgeblend relies on (frame-latency waitable object,
     `GetFrameStatistics` for missed vblanks, `MakeWindowAssociation`) are only reachable through
     `wgpu`'s unsafe HAL layer.
   - Flip-model swapchains and independent-flip eligibility would be the same either way.
   - The render cost (about 1 ms of GPU time per frame) is too small for `wgpu`'s overhead to
     matter.
   - `wgpu` has breaking API changes in most releases; D3D11 is stable.

   With a single WGSL shader, moving Windows to `wgpu` later stays cheap if it ever pays off.

## Phase 0: refactor on Windows, no behaviour change

1. **New crate `edgeblend-platform`** with three interfaces, implemented first for Windows with
   the current code:
   - `Displays`: enumerate displays with name, position, size, GPU and refresh rate.
   - `Capture`: the latest source frame as a GPU texture, plus new/skipped frame counters.
   - `Output`: a borderless fullscreen surface on a chosen display, with a wait-for-present-slot
     call and presentation statistics.
2. **Split `edgeblend-engine`** into a portable loop (pacing, counters, recovery, preview) and
   per-platform window and hotkey code.
3. **Port `blend.hlsl` to WGSL.** Generate the HLSL with `naga` in `build.rs`. Check the result
   against the current renderer by comparing preview readbacks pixel by pixel, for every pattern.
4. **Cross-platform helpers:**
   - tray: `tray-icon` crate;
   - hotkeys: `global-hotkey` crate;
   - single instance: a lock file;
   - watchdog: portable process spawning.
5. **CI:** a Linux job running the portable tests and `cargo check --target x86_64-pc-windows-msvc`.

**Done when:** the Windows build behaves as before, with the same PresentMon results.

## Phase 1: `wgpu` renderer

- **New crate `edgeblend-render-wgpu`** with the same features as the D3D11 renderer: blend,
  patterns, preview readback, shader hot-reload.
- **Develop against a synthetic source first** (a test pattern), so it doesn't depend on any
  capture backend.

**Done when:** on macOS and Linux the patterns render to a window, and preview readbacks match the
Windows renderer.

## Phase 2: macOS (13 or later)

| Piece | Plan |
| --- | --- |
| Capture | ScreenCaptureKit `SCStream` on the source display. Frames arrive as IOSurface-backed buffers and become Metal textures without a CPU copy. Needs the Screen Recording permission. |
| Output | Borderless window at `NSScreenSaverWindowLevel` (above the menu bar and Dock) on the output screen. Pacing through `CAMetalLayer` and `CADisplayLink`. |
| Canvas | macOS has no built-in virtual display. Document a third-party virtual display tool or a dummy HDMI plug. Later option: a built-in virtual display via `CGVirtualDisplay`. |
| Hotkeys | `global-hotkey` (needs the Accessibility permission). |
| Tray | `tray-icon` in the menu bar. |
| Autostart + watchdog | A LaunchAgent with `KeepAlive` covers both. |
| Packaging | `.app` bundle in a `.dmg`, unsigned (see Decisions). |

**Risks:**
- ScreenCaptureKit frame timing and latency compared with Desktop Duplication.
- Permission prompts: Screen Recording, Accessibility.

## Phase 3: Ubuntu (22.04 and 24.04, Wayland and X11)

| Piece | Plan |
| --- | --- |
| Capture (Wayland, default) | xdg-desktop-portal ScreenCast + PipeWire. The user approves once in a system dialog; a restore token avoids repeating it. Frames arrive as DMA-BUFs. Zero-copy import into Vulkan needs external-memory work, so start with a CPU-copy fallback and measure it. |
| Capture (X11) | XShm or XComposite as a simpler fallback. |
| Output | Borderless fullscreen on the chosen monitor through `winit`. Wayland has no "always on top"; fullscreen replaces it. GNOME can scan out a fullscreen window directly, the equivalent of independent flip. |
| Canvas | X11: add a virtual screen with `xrandr`. Wayland/GNOME: hard. Document a dummy HDMI plug or a second real output, and investigate GNOME's headless virtual monitor. |
| Hotkeys | No general global hotkeys on Wayland. Use the portal GlobalShortcuts API where the desktop supports it; otherwise only the panel buttons work. |
| Tray | `tray-icon` via AppIndicator (shipped with Ubuntu). |
| Autostart + watchdog | `systemd --user` service with `Restart=on-failure`, or a `.desktop` file in `~/.config/autostart`. |
| Packaging | `.deb` (via `cargo-deb`) and AppImage. |

**Risks:**
- Frame pacing through the Wayland compositor.
- With the CPU-copy fallback, a 5760-wide canvas at 60 fps is about 1.5 GB/s of memory traffic.
- Behaviour differs between NVIDIA and Mesa drivers.

## Phase 4: release pipeline

- **Build matrix:** `windows-latest`, `macos-14` (Apple Silicon; Intel or universal binary later)
  and `ubuntu-22.04`.
- **Release assets:** `edgeblend-setup.exe`, `edgeblend.dmg`, `edgeblend_amd64.deb` and
  `edgeblend.AppImage`, each also under a fixed name for `releases/latest/download/...`, plus
  `SHA256SUMS.txt`.
- **macOS signing and notarization:** CI steps that run only when signing secrets are configured.

## Test hardware

- **macOS:** a Mac with a second display, a projector, or a video-wall box.
- **Ubuntu:** a machine with a GPU. Test both Wayland and X11 sessions, ideally on NVIDIA and on
  AMD or Intel.
- **Final check per platform:** a video-wall box or at least two projectors.

## Order and rough size

| Phase | Size | Shippable on its own |
| --- | --- | --- |
| 0. Refactor + WGSL | Medium | Yes: Windows unchanged, cleaner code |
| 1. `wgpu` renderer | Medium | Partly (patterns only) |
| 2. macOS | Medium to large | Yes |
| 3. Ubuntu | Large (mostly Wayland capture) | Yes |
| 4. CI/release | Small | Alongside 2 and 3 |

# edgeblend

Whole-desktop projector edge blending for Windows, written in Rust (D3D11 + DXGI Desktop Duplication).

Most edge-blending lives inside one application (for example presentation software with app-limited
blending). edgeblend works on the **whole desktop**, so any Windows app can be shown across blended
projectors.

## How it works

Windows renders a desktop on a "source" display (the canvas). edgeblend captures it on the GPU, runs
it through a blend shader and shows the result in a borderless, topmost window on the "output"
display. The output feeds a video-wall box (or splitter) that drives N projectors. See
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Features

- Blends 1-6 projectors left to right, with a distance-weighted blend that follows each seam's real overlap
- Per-projector quad geometry (keystone, spill masking) edited visually in a calibration editor
- Calibration patterns (Diagnostic, Alignment grid, flat gray, white) and live previews
- Options for video-wall boxes that reverse the output order or mirror the picture
- GPU-only capture, frame pacing with on-screen stats (repeated / skipped / missed vblanks)
- Control panel with tray icon, single instance, crash watchdog, logon autostart
- Recovers from GPU resets, display changes and unplugged displays

## Requirements

- Windows 10 or 11
- Source and output displays on the **same GPU**
- A video-wall box or splitter feeding the N projectors
- Optionally a virtual display as the canvas, so your main monitor stays free:
  see [tools/virtual-display](tools/virtual-display) and [docs/VIRTUAL-DISPLAY.md](docs/VIRTUAL-DISPLAY.md)

## Install

Download [`edgeblend-setup.exe`](https://github.com/verdade-igreja/edgeblend/releases/latest/download/edgeblend-setup.exe)
from the GitHub Releases page and run it. The install is per-user and needs no admin rights (the app
goes to `%LOCALAPPDATA%\Programs\edgeblend`). Tick "Start edgeblend automatically when I log in" to
register the logon task. The config in `%APPDATA%\edgeblend\blend.toml` is never touched, not even by
the uninstaller.

The installer is unsigned, so Windows SmartScreen will warn: click "More info", then "Run anyway".
`SHA256SUMS.txt` on the release lets you verify the download.

## Quick start

1. Run `edgeblend --list` (from a terminal) to see display names, sizes and adapters.
2. Start edgeblend and choose the **Source** (canvas) and **Output** (box) displays in the panel.
3. Set the number of projectors, overlap, gamma and curve. Enable **Reverse order** or **Mirror
   output** if your box needs it.
4. Pick a calibration pattern and open **Calibration editor…** to align the projector quads; see
   [docs/CALIBRATION.md](docs/CALIBRATION.md).
5. Press Start. Settings apply live and autosave to `%APPDATA%\edgeblend\blend.toml`
   (see [blend.example.toml](blend.example.toml)).

```
edgeblend [config.toml] [--start] [--headless] [--list] [--watchdog]
  (no flags)  open the control panel
  --start     open the control panel and start blending immediately
  --headless  blend without UI, using the config file (Ctrl+Alt+Q quits)
  --list      list displays
  --watchdog  restart automatically after a crash (exits when the app is quit normally)
```

Release builds have no console; `--list` and `--help` print when run from a terminal.

Global hotkeys: `Ctrl+Alt+T` test pattern, `Ctrl+Alt+C` cursor, `Ctrl+Alt+Q` stop blending.

## Tray and background behaviour

Closing the panel (X) hides it to the tray icon. Left-click the icon to show the panel; the menu has
Show panel, Start/Stop blending and Quit (the only way to exit). A second launch brings the existing
panel to the front. Fatal errors, panics and watchdog restarts are logged to
`%APPDATA%\edgeblend\edgeblend.log`.

## Autostart

The installer can register a per-user logon Scheduled Task running
`edgeblend.exe --watchdog --start`. You can also manage it by hand with
`tools\autostart\install-autostart.ps1 -Apply` / `-Uninstall -Apply`
(see [tools/autostart/README.md](tools/autostart/README.md)). It is a Scheduled Task and not a
service because services run in session 0, which has no desktop to capture.

## Limitations

- Source and output must be on the same GPU.
- No black-level compensation yet.
- Per-projector tile rectangles are not configurable; the output is split into N equal tiles.
- Known limitations and ideas are tracked in the [GitHub issues](https://github.com/verdade-igreja/edgeblend/issues).

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Calibration](docs/CALIBRATION.md)
- [Virtual display](docs/VIRTUAL-DISPLAY.md)
- [Porting plan (macOS, Ubuntu)](docs/PORTING.md)
- [Contributing](CONTRIBUTING.md)

## License

[MIT](LICENSE)

## Inspiration

- Paul Bourke's edge-blending notes
- NVIDIA Warp & Blend API / Fly Elise-ng
- OBS display capture

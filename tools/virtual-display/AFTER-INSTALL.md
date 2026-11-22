# After installing the virtual display

Follow this once `setup.ps1 -Apply` (or Virtual Driver Control) has installed **Virtual Display Driver** and written `C:\VirtualDisplayDriver\vdd_settings.xml`.

## 1. Confirm Windows sees the monitor

1. Open **Settings → System → Display**.
2. You should see a new display (often labeled like a generic / MTT virtual monitor) in addition to your main monitor and the video-wall box output.
3. If it is missing: Device Manager → **Display adapters** → find **Virtual Display Driver** (or similar) → Disable → Enable. Or reboot.
4. If the resolution list has no 5760×1080: confirm `C:\VirtualDisplayDriver\vdd_settings.xml` lists that mode first, then reload the driver again.

## 2. Set 5760×1080 (60 Hz, or 30 Hz if needed)

1. In Display settings, select the **virtual** monitor.
2. Set resolution to **5760 × 1080**.
3. Prefer **60 Hz**. If the mode is missing, unstable, or DWM stutters, pick **30 Hz** (also listed in the XML written by `setup.ps1`).
4. Leave **Scale** at **100%** (important for DXGI desktop coordinates and for matching content pixels).

## 3. Arrange the desktop

Goal: operator stays on the main monitor; show content lives on the virtual canvas; mouse rarely wanders onto the projector box.

Suggested layout (left → right):

| Main monitor (operator) | Virtual 5760×1080 (source / canvas, example size) | Box (output) |
| --- | --- | --- |

Or put the **box far away** (e.g. above/below or far right) so it is hard to hit accidentally. edgeblend will cover the box with a borderless topmost window while blending; still avoid leaving the cursor there when stopped.

Do **not** make the virtual display the only display if you still need your main monitor for the control panel.

## 4. Verify DXGI adapter binding

From the repo (release or `cargo run --release`):

```text
edgeblend --list
```

Check:

- The virtual monitor appears (note its DXGI name, e.g. `\\.\DISPLAY3`).
- Its **adapter** string is the same GPU as the box output.
- The **adapter LUID** matches the box output’s LUID.

If the virtual monitor is on a different adapter than the box, edgeblend will refuse to start (same-GPU check). Fix by setting the GPU friendly name in `vdd_settings.xml` to the exact DXGI description of your GPU, reload the driver, and list again. `setup.ps1` defaults `GpuFriendlyName` to `default` (the driver picks the GPU).

## 5. Point edgeblend at the canvas

1. Open the control panel (or edit `%APPDATA%\edgeblend\blend.toml`).
2. Set **Source** to the virtual display’s DXGI name from `--list`.
3. Set **Output** to the box display (unchanged).
4. Start blending; put your presentation app (or any app) fullscreen / maximized on the virtual monitor.

## 6. If source and output are on different adapters

edgeblend today requires matching adapter LUIDs. If the virtual output stays on a separate IDD/WARP adapter despite GPU selection:

1. Prefer fixing driver GPU binding (above) — that is the right fix.
2. Otherwise the app would need **cross-adapter capture**: duplicate on the source adapter, copy/share the texture to the output GPU (e.g. DXGI shared handles / `IDXGIResource1::CreateSharedHandle`, or a CPU staging readback — slower), then render the blend window on the output adapter. Cross-GPU source/output is not implemented; see the GitHub issues.

## Quick regress checklist

- [ ] Virtual monitor visible in Settings
- [ ] 5760×1080 @ 60 (or 30) at 100% scaling
- [ ] Arranged away from easy mouse-over on the box
- [ ] `edgeblend --list` → same adapter as box
- [ ] Config `source` = virtual, `output` = box
- [ ] Content app on the virtual canvas; main monitor free for operator UI

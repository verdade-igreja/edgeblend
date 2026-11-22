# Virtual display options for edgeblend

Notes on a dedicated wide virtual canvas (example: **5760×1080 @ 60 Hz**, fallback 30 Hz) on Windows 10/11, captured via DXGI Desktop Duplication on the same GPU as the video-wall box output. edgeblend refuses to start if source and output adapter LUIDs differ (`crates/edgeblend-engine`).

**Note:** `tools/virtual-display/` only contains scripting. Drivers are *not* installed by the dry-run default of `setup.ps1`.

---

## Why DXGI adapter binding matters

An Indirect Display Driver (IddCx) creates a virtual monitor. Separately, `IddCxAdapterSetRenderAdapter` tells Windows **which GPU should render DWM’s desktop into that monitor’s swapchain**.

Per Microsoft’s IddCx 1.4 docs, when a preferred render adapter is set, **DX enumerates the indirect monitor on that GPU for DWM** (“Enum for DWM” column). Desktop Duplication then works the normal way: create a D3D11 device on that same adapter and call `DuplicateOutput` on the virtual output.

Sources:

- <https://learn.microsoft.com/en-us/windows-hardware/drivers/display/iddcx1.4-updates-for-console-and-remote-idds>
- <https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/iddcx/nf-iddcx-iddcxadaptersetrenderadapter>

Practical confirmation: Sunshine/Moonlight hosts routinely DXGI-capture Virtual Display Driver and Parsec VDD monitors when the virtual display is bound to the GPU Sunshine uses. Failures usually mean the virtual output landed on the wrong adapter (iGPU vs discrete), not that IDD is unduplicatable.

Caveat: one app (BetterCast) reported `DuplicateOutput` failing on some IDD setups and fell back to GDI; treat same-adapter binding as mandatory for edgeblend, and verify with `edgeblend --list` after install.

---

## Option A — Virtual Display Driver (VirtualDrivers / MikeTheTech)

| Field | Detail |
| --- | --- |
| Repo | <https://github.com/VirtualDrivers/Virtual-Display-Driver> |
| License | **MIT** (project LICENSE) |
| Maintenance | **Active** (releases through 2025; large community; companion “Virtual Driver Control” app) |
| Latest checked | As of writing, tag **25.7.23** (2025-07-23) — “Beta: Virtual Driver Control”; check the releases page for newer versions |
| IddCx | Claims **1.10**; WHQL/SignPath-**signed** driver packages |
| Custom 5760×1080@60? | **Yes** — arbitrary modes via `vdd_settings.xml` (wiki + issues confirm custom WxH; first listed mode is preferred). Ultra-wide examples in the wild include 5120×2160, 5760×2400. |
| Config | File: `C:\VirtualDisplayDriver\vdd_settings.xml` (override path via `HKLM\SOFTWARE\MikeTheTech\VirtualDisplayDriver\VDDPATH`). Optional GPU friendly name / PCI bus → LUID. Companion GUI can edit XML. |
| Install | Signed driver via **Virtual Driver Control** GUI, `winget install VirtualDrivers.Virtual-Display-Driver`, or community `silent-install.ps1` (nefcon + `MttVDD.inf` / HWID `Root\MttVDD`). **Admin required.** |
| Uninstall | VDC “Uninstall”, or `nefconw remove "Root\MttVDD"` (admin). |
| Test-signing | **Not required** for the signed release packages. |
| AMD notes | Adrenalin often **does not** list virtual displays for FSR/overlay (by design — no physical port). DXGI/Windows Settings still see the mode. Uninstall VDD before major GPU driver updates to avoid scrambled display priority. |
| DXGI / Desktop Duplication | Driver calls `IddCxAdapterSetRenderAdapter` with the configured GPU’s LUID. Set `<gpu><friendlyname>` to your GPU’s DXGI description (or leave `default` on a single discrete GPU). Expect the virtual output to appear under the same adapter as the box in `edgeblend --list`, with the same LUID. Duplication from a D3D11 device on that adapter is the intended path (Sunshine does this). |

Config wiki: <https://github.com/VirtualDrivers/Virtual-Display-Driver/wiki/How-to-configure-the-driver>  
Silent install reference: <https://github.com/VirtualDrivers/Virtual-Display-Driver/blob/master/Community%20Scripts/silent-install.ps1>  
GPU selection write-up: <https://deepwiki.com/VirtualDrivers/Virtual-Display-Driver/2.5-gpu-selection-system>

---

## Option B — Parsec Virtual Display Driver (+ nomi-san control app)

| Field | Detail |
| --- | --- |
| Driver | Closed-source **Parsec** binary (`mm.inf` / HWID `Root\Parsec\VDA`), signed by Parsec |
| Control app / API | <https://github.com/nomi-san/parsec-vdd> — MIT for the *wrapper*; driver itself remains Parsec’s |
| Maintenance | Driver last public drop **0.45** (builds.parsec.app); control app release **v0.45.1** (2024-04). Still widely used, but slower cadence than Option A. |
| Custom 5760×1080@60? | **Yes, with limits** — up to **5** custom modes via registry `HKLM\SOFTWARE\Parsec\vdd\{0..4}` DWORD `width` / `height` / `hz`. Presets max out around 4K; 5760×1080 must be custom. |
| Config | Registry only for modes; parent GPU by **vendor ID** only (`PreferredRenderAdapterVendorId` = `1002` for AMD) — cannot pick among multiple AMD GPUs by LUID. |
| Install | `parsec-vdd-0.45.0.0.exe /S` or nefcon create-device-node + install-driver. **Admin.** Displays need a keeper app that pings the driver (~1 s timeout if not polled). |
| Uninstall | Parsec setup / nefcon remove `Root\Parsec\VDA`. |
| Test-signing | Not required (Parsec-signed). |
| AMD notes | Vendor-ID parent GPU selection works for a single AMD card. Dual-AMD / same-vendor multi-GPU cannot pin a specific LUID (author points users at open-source VDD for that). |
| DXGI / Desktop Duplication | Same IddCx model: prefer AMD render adapter → output should enumerate for DWM on that GPU. Works with Sunshine when bound correctly. Keeper process required so the virtual plug stays alive. |

Advanced config: <https://support.parsec.app/hc/en-us/articles/32361359271444-VDD-Advanced-Configuration>  
Specs: <https://github.com/nomi-san/parsec-vdd/blob/main/docs/PARSEC_VDD_SPECS.md>  
GPU LUID limitation: <https://github.com/nomi-san/parsec-vdd/issues/47>

---

## Option C — virtual-display-rs (MolotovCherry)

| Field | Detail |
| --- | --- |
| Repo | <https://github.com/MolotovCherry/virtual-display-rs> |
| License | **MIT** |
| Maintenance | **Stale for production** — latest stable tag **v0.3.1** (2023-10); README points at newer “dev” builds. |
| Custom 5760×1080@60? | **Yes** via control app (per-monitor resolution lists). |
| Config | Control app / its saved config; disable+re-enable monitor after mode changes. |
| Install | Self-signed cert must be installed **before** the driver; **test-signing / trusting a private cert**. Admin. |
| Uninstall | Control app / PnP remove. |
| AMD / DXGI | IddCx 1.5; less documentation on `IddCxAdapterSetRenderAdapter` polish than Option A. Unsigned → higher friction on Win11. |

---

## Others (brief)

| Project | Why not primary |
| --- | --- |
| Microsoft IndirectDisplay sample | Sample only, unsigned, not productized. |
| RustDesk IddDriver | Older IddCx 1.2, unsigned, remote-desktop focused. |
| usbmmidd_v2 (Amyuni) | Not IddCx; limited/custom mode story; gaming/capture weaker. |
| Apollo / SudoVDA | Tied to Sunshine streaming host lifecycle — wrong product shape for a always-on blend canvas. |

---

## Recommendation

**Use VirtualDrivers Virtual Display Driver (Option A), driver package 25.7.23 (or newer), configured via `vdd_settings.xml`.**

Reasons:

1. **Custom ultra-wide is first-class** — put only `5760×1080@60` (+ `@30` fallback) in XML; no 5-slot registry cap.
2. **GPU binding by name/LUID** — critical so the canvas shares the GPU adapter LUID with the box output (edgeblend’s hard check).
3. **Signed + MIT + actively maintained** — no test-signing; better than Parsec’s closed driver + ping keeper, and better than virtual-display-rs’s stale unsigned releases.
4. **DXGI Desktop Duplication path is proven** in Sunshine setups once the render GPU is correct.
5. **Scriptable** — nefcon silent install matches what `setup.ps1 -Apply` automates.

`setup.ps1` in this folder targets Option A.

---

## Troubleshooting

- Confirm after install that `edgeblend --list` shows the virtual monitor on the same adapter, with the **same LUID**, as the box output.
- If Windows lists the IDD adapter separately and the output is *not* under your GPU, set `<friendlyname>` to the exact DXGI adapter string and reload the driver.
- Very wide modes can stress DWM/GPU memory; if 60 Hz is unstable, keep the 30 Hz fallback mode in XML and select it in Settings.
- AMD Adrenalin ignoring the virtual display is expected and unrelated to DXGI capture.

use anyhow::{anyhow, Result};
use std::time::{Duration, Instant, SystemTime};
use windows::core::{s, Interface, PCSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::Fxc::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Threading::WaitForSingleObjectEx;

use edgeblend_config::Config;
use edgeblend_geom::{even_split, invert, square_to_quad, Mat3};

/// Built-in copy of the shader, used when the source file is not on disk.
const SHADER: &str = include_str!("blend.hlsl");
/// Watched for live shader reload while developing.
const SHADER_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/blend.hlsl");

fn shader_mtime() -> Option<SystemTime> {
    std::fs::metadata(SHADER_PATH).and_then(|m| m.modified()).ok()
}

fn build_shaders(device: &ID3D11Device, src: &str) -> Result<(ID3D11VertexShader, ID3D11PixelShader)> {
    let vsb = compile(src, s!("vs_main"), s!("vs_5_0"))?;
    let psb = compile(src, s!("ps_main"), s!("ps_5_0"))?;
    unsafe {
        let mut vs = None;
        device.CreateVertexShader(blob(&vsb), None::<&ID3D11ClassLinkage>, Some(&mut vs))?;
        let mut ps = None;
        device.CreatePixelShader(blob(&psb), None::<&ID3D11ClassLinkage>, Some(&mut ps))?;
        Ok((vs.unwrap(), ps.unwrap()))
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Params {
    n: f32,
    gamma: f32,
    curve: f32,
    pattern: f32,
    view: f32,
    mirror: f32,
    reverse_order: f32,
    blend_on: f32,
    show_cursor: f32,
    canvas_w: f32,
    canvas_h: f32,
    src_w: f32,
    src_h: f32,
    cursor: [f32; 2],
    tile_w: f32,
    /// Bit i = projector i blacked out (stored as float; exact for small ints).
    mute_mask: f32,
    _pad: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct GeometryBuf {
    hf: [[f32; 4]; 18],
    hi: [[f32; 4]; 18],
    seams: [[f32; 4]; 5],
}

impl Default for GeometryBuf {
    fn default() -> Self {
        Self {
            hf: [[0.0; 4]; 18],
            hi: [[0.0; 4]; 18],
            seams: [[0.0; 4]; 5],
        }
    }
}

fn pack_mat(rows: &mut [[f32; 4]], base: usize, m: &Mat3) {
    for r in 0..3 {
        rows[base + r] = [m[r][0], m[r][1], m[r][2], 0.0];
    }
}

fn identity() -> Mat3 {
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
}

struct Frame {
    tex: ID3D11Texture2D,
    srv: ID3D11ShaderResourceView,
    w: u32,
    h: u32,
    format: DXGI_FORMAT,
}

/// What one `capture` call got from Desktop Duplication.
#[derive(Debug, Clone, Copy, Default)]
pub struct CaptureResult {
    /// A new desktop image was copied (mouse-only updates don't count).
    pub new_image: bool,
    /// Desktop images the source presented since the previous capture; >1 = some skipped.
    pub accumulated: u32,
}

/// The device, swapchain or output went away (driver reset, display mode change,
/// unplug). The caller should drop the renderer and build a new one.
#[derive(Debug)]
pub struct DeviceLost(pub String);

impl std::fmt::Display for DeviceLost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GPU device lost: {}", self.0)
    }
}

impl std::error::Error for DeviceLost {}

fn is_device_lost(code: windows::core::HRESULT) -> bool {
    code == DXGI_ERROR_DEVICE_REMOVED || code == DXGI_ERROR_DEVICE_RESET || code == DXGI_ERROR_DEVICE_HUNG
}

/// Output-side frame statistics between two `present_stats` calls.
#[derive(Debug, Clone, Copy)]
pub struct PresentStats {
    /// Vblanks on which the screen showed no new frame from us (a hitch on the wall).
    pub missed_vblanks: u32,
    /// Measured refresh rate of the output display.
    pub output_hz: f64,
}

/// CPU-side RGBA snapshot from `render_preview` for the control-panel UI.
#[derive(Clone)]
pub struct PreviewImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub seq: u64,
}

pub struct Renderer {
    device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    source: IDXGIOutput1,
    dup: Option<IDXGIOutputDuplication>,
    swap: IDXGISwapChain2,
    /// Signalled when the swapchain can take another frame without queueing (latency 1).
    latency_wait: HANDLE,
    last_stats: Option<DXGI_FRAME_STATISTICS>,
    source_hz: Option<f64>,
    rtv: ID3D11RenderTargetView,
    out_w: u32,
    out_h: u32,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    cbuf_params: ID3D11Buffer,
    cbuf_geom: ID3D11Buffer,
    frame: Option<Frame>,
    fallback_src: (u32, u32),
    shader_mtime: Option<SystemTime>,
    preview_rt: Option<ID3D11Texture2D>,
    preview_rtv: Option<ID3D11RenderTargetView>,
    preview_staging: Option<ID3D11Texture2D>,
    preview_w: u32,
    preview_h: u32,
    preview_seq: u64,
}

impl Renderer {
    pub fn new(
        adapter: &IDXGIAdapter1,
        source: &IDXGIOutput,
        source_size: (u32, u32),
        hwnd: HWND,
        out_w: u32,
        out_h: u32,
    ) -> Result<Self> {
        unsafe {
            let mut device = None;
            let mut ctx = None;
            D3D11CreateDevice(
                adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut ctx),
            )?;
            let device: ID3D11Device = device.ok_or_else(|| anyhow!("no D3D11 device"))?;
            let ctx = ctx.ok_or_else(|| anyhow!("no D3D11 context"))?;

            let factory: IDXGIFactory2 = adapter.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: out_w,
                Height: out_h,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                // Lets us wait for the right moment to render instead of queueing frames
                // behind vsync (stale frames, then a skip when the queue drains).
                Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32,
                ..Default::default()
            };
            let swap: IDXGISwapChain2 =
                factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None::<&IDXGIOutput>)?.cast()?;
            // Alt+Enter would try exclusive fullscreen on a window that never has focus.
            let _ = factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);
            swap.SetMaximumFrameLatency(1)?;
            let latency_wait = swap.GetFrameLatencyWaitableObject();
            let bb: ID3D11Texture2D = swap.GetBuffer(0)?;
            let mut rtv = None;
            device.CreateRenderTargetView(&bb, None, Some(&mut rtv))?;

            let shader_mtime = shader_mtime();
            let (vs, ps) = match std::fs::read_to_string(SHADER_PATH) {
                Ok(src) => build_shaders(&device, &src).or_else(|e| {
                    eprintln!("{e:#}; using built-in shader");
                    build_shaders(&device, SHADER)
                })?,
                Err(_) => build_shaders(&device, SHADER)?,
            };

            let sdesc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                ComparisonFunc: D3D11_COMPARISON_NEVER,
                MaxLOD: f32::MAX,
                ..Default::default()
            };
            let mut sampler = None;
            device.CreateSamplerState(&sdesc, Some(&mut sampler))?;

            let mut cbuf_params = None;
            device.CreateBuffer(
                &D3D11_BUFFER_DESC {
                    ByteWidth: std::mem::size_of::<Params>() as u32,
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut cbuf_params),
            )?;
            let mut cbuf_geom = None;
            device.CreateBuffer(
                &D3D11_BUFFER_DESC {
                    ByteWidth: std::mem::size_of::<GeometryBuf>() as u32,
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut cbuf_geom),
            )?;

            Ok(Self {
                device,
                ctx,
                source: source.cast()?,
                dup: None,
                swap,
                latency_wait,
                last_stats: None,
                source_hz: None,
                rtv: rtv.unwrap(),
                out_w,
                out_h,
                vs,
                ps,
                sampler: sampler.unwrap(),
                cbuf_params: cbuf_params.unwrap(),
                cbuf_geom: cbuf_geom.unwrap(),
                frame: None,
                fallback_src: source_size,
                shader_mtime,
                preview_rt: None,
                preview_rtv: None,
                preview_staging: None,
                preview_w: 0,
                preview_h: 0,
                preview_seq: 0,
            })
        }
    }

    /// Recompile the shader if src/blend.hlsl changed on disk; keeps the old one on error.
    pub fn reload_shader_if_changed(&mut self) {
        let m = shader_mtime();
        if m.is_none() || m == self.shader_mtime {
            return;
        }
        self.shader_mtime = m;
        let result = std::fs::read_to_string(SHADER_PATH)
            .map_err(anyhow::Error::from)
            .and_then(|src| build_shaders(&self.device, &src));
        match result {
            Ok((vs, ps)) => {
                self.vs = vs;
                self.ps = ps;
                println!("shader reloaded");
            }
            Err(e) => eprintln!("shader reload failed, keeping previous: {e:#}"),
        }
    }

    pub fn capture_size(&self) -> Option<(u32, u32)> {
        self.frame.as_ref().map(|f| (f.w, f.h))
    }

    /// Block until the swapchain can take a frame without queueing behind vsync
    /// (at most ~one refresh). Call once per frame, before `capture` and `render`.
    pub fn wait_for_present_slot(&self) {
        // A timeout (display asleep, window occluded) is harmless: Present still paces us.
        unsafe {
            WaitForSingleObjectEx(self.latency_wait, 100, true);
        }
    }

    /// Refresh rate of the source display as Desktop Duplication reports it.
    pub fn source_hz(&self) -> Option<f64> {
        self.source_hz
    }

    /// Grab the newest desktop image, waiting up to `wait` for one to arrive so a source
    /// frame that lands just after we start is still shown this refresh (not repeated now
    /// and skipped next time).
    pub fn capture(&mut self, wait: Duration) -> Result<CaptureResult> {
        let mut out = CaptureResult::default();
        if self.dup.is_none() {
            // Fails on the secure desktop (UAC, lock screen); retried next frame.
            match unsafe { self.source.DuplicateOutput(&self.device) } {
                Ok(d) => {
                    let desc = unsafe { d.GetDesc() };
                    let rr = desc.ModeDesc.RefreshRate;
                    self.source_hz = (rr.Denominator != 0).then(|| rr.Numerator as f64 / rr.Denominator as f64);
                    self.dup = Some(d);
                }
                Err(e) if is_device_lost(e.code()) => return Err(DeviceLost(format!("{e}")).into()),
                Err(_) => return Ok(out),
            }
        }
        let dup = self.dup.clone().unwrap();
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now()).as_millis() as u32;
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut res: Option<IDXGIResource> = None;
            match unsafe { dup.AcquireNextFrame(left, &mut info, &mut res) } {
                Ok(()) => {
                    // LastPresentTime == 0: only the mouse moved; keep waiting for an image.
                    let image = info.LastPresentTime != 0;
                    if image {
                        if let Some(tex) = res.and_then(|r| r.cast::<ID3D11Texture2D>().ok()) {
                            if let Err(e) = self.copy_frame(&tex) {
                                eprintln!("frame copy failed: {e}");
                            }
                        }
                        out.new_image = true;
                        out.accumulated += info.AccumulatedFrames;
                    }
                    unsafe {
                        let _ = dup.ReleaseFrame();
                    }
                    if image || left == 0 {
                        return Ok(out);
                    }
                }
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(out),
                Err(e) if is_device_lost(e.code()) => return Err(DeviceLost(format!("{e}")).into()),
                Err(e) => {
                    // ACCESS_LOST: mode change, fullscreen app, desktop switch. Recreate.
                    eprintln!("duplication lost ({:?}), recreating", e.code());
                    self.dup = None;
                    return Ok(out);
                }
            }
        }
    }

    fn copy_frame(&mut self, src: &ID3D11Texture2D) -> Result<()> {
        unsafe {
            let mut d = D3D11_TEXTURE2D_DESC::default();
            src.GetDesc(&mut d);
            let stale = match &self.frame {
                Some(f) => f.w != d.Width || f.h != d.Height || f.format != d.Format,
                None => true,
            };
            if stale {
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: d.Width,
                    Height: d.Height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: d.Format,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                    CPUAccessFlags: 0,
                    MiscFlags: 0,
                };
                let mut tex = None;
                self.device.CreateTexture2D(&desc, None, Some(&mut tex))?;
                let tex = tex.unwrap();
                let mut srv = None;
                self.device.CreateShaderResourceView(&tex, None, Some(&mut srv))?;
                println!("capturing {}x{} ({:?})", d.Width, d.Height, d.Format);
                self.frame = Some(Frame {
                    tex,
                    srv: srv.unwrap(),
                    w: d.Width,
                    h: d.Height,
                    format: d.Format,
                });
            }
            let f = self.frame.as_ref().unwrap();
            self.ctx.CopyResource(&f.tex, src);
        }
        Ok(())
    }

    fn build_params(&self, cfg: &Config, cursor_src: Option<[f32; 2]>, view: f32) -> Params {
        let (src_w, src_h) = match &self.frame {
            Some(f) => (f.w as f32, f.h as f32),
            None => (self.fallback_src.0 as f32, self.fallback_src.1 as f32),
        };
        let n = cfg.projectors.max(1) as f32;
        // Source-pixel cursor → wall q (source texture stretched over the wall).
        let cursor_q = cursor_src
            .map(|c| [c[0] / src_w.max(1.0), c[1] / src_h.max(1.0)])
            .unwrap_or([-1e6, -1e6]);
        Params {
            n,
            gamma: cfg.gamma,
            curve: cfg.curve,
            pattern: cfg.pattern.shader_id(),
            view,
            mirror: if cfg.mirror { 1.0 } else { 0.0 },
            reverse_order: if cfg.reverse_order { 1.0 } else { 0.0 },
            blend_on: if cfg.blend_ramps { 1.0 } else { 0.0 },
            show_cursor: if cfg.show_cursor && cursor_src.is_some() { 1.0 } else { 0.0 },
            canvas_w: cfg.canvas[0] as f32,
            canvas_h: cfg.canvas[1] as f32,
            src_w,
            src_h,
            cursor: cursor_q,
            tile_w: (self.out_w as f32 / n).max(1.0),
            mute_mask: (cfg.mute_mask & 0x3f) as f32,
            _pad: [0.0; 3],
        }
    }

    fn build_geometry(&self, cfg: &Config) -> GeometryBuf {
        let mut g = GeometryBuf::default();
        let n = cfg.projectors.max(1) as usize;
        for (i, proj) in cfg.geometry.iter().take(n).enumerate() {
            let hf = square_to_quad(&proj.corners);
            let hi = invert(&hf).unwrap_or_else(identity);
            pack_mat(&mut g.hf, i * 3, &hf);
            pack_mat(&mut g.hi, i * 3, &hi);
        }
        // Seam k target rings: FIXED wall points (nominal overlap centres of an even split),
        // independent of the edited geometry. If they followed the quads, nudging one
        // projector would also move the neighbour's ring (in the opposite direction on the
        // wall), which makes alignment feel inverted.
        let nominal = even_split(n, cfg.overlap_frac);
        for k in 0..n.saturating_sub(1).min(5) {
            let right_edge = nominal[k][1][0];
            let left_edge = nominal[k + 1][0][0];
            g.seams[k] = [(left_edge + right_edge) * 0.5, 0.5, 0.0, 0.0];
        }
        g
    }

    fn draw(&self, rtv: &ID3D11RenderTargetView, width: u32, height: u32, p: &Params, g: &GeometryBuf) {
        unsafe {
            let ctx = &self.ctx;
            ctx.UpdateSubresource(&self.cbuf_params, 0, None, p as *const _ as *const _, 0, 0);
            ctx.UpdateSubresource(&self.cbuf_geom, 0, None, g as *const _ as *const _, 0, 0);
            ctx.ClearRenderTargetView(rtv, &[0.0, 0.0, 0.0, 1.0]);
            ctx.OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);
            ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: width as f32,
                Height: height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            ctx.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShader(&self.ps, None);
            let srv = self.frame.as_ref().map(|f| f.srv.clone());
            ctx.PSSetShaderResources(0, Some(&[srv]));
            ctx.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.cbuf_params.clone()), Some(self.cbuf_geom.clone())]));
            ctx.Draw(3, 0);
        }
    }

    /// Draw the blended output and present (vsync-paced on the output display).
    /// Returns a `DeviceLost` error when the renderer must be rebuilt.
    pub fn render(&self, cfg: &Config, cursor: Option<[f32; 2]>) -> Result<()> {
        let p = self.build_params(cfg, cursor, 0.0);
        let g = self.build_geometry(cfg);
        self.draw(&self.rtv, self.out_w, self.out_h, &p, &g);
        let hr = unsafe { self.swap.Present(1, DXGI_PRESENT(0)) };
        if is_device_lost(hr) {
            let reason = unsafe { self.device.GetDeviceRemovedReason() };
            return Err(DeviceLost(format!("{hr:?} (reason {reason:?})")).into());
        }
        // DXGI_STATUS_OCCLUDED and friends are success codes; nothing to do.
        hr.ok()?;
        Ok(())
    }

    /// Hitches and refresh rate on the output since the previous call (call ~once a second).
    /// None until two samples exist, or when DXGI can't give statistics right now.
    pub fn present_stats(&mut self) -> Option<PresentStats> {
        let mut now = DXGI_FRAME_STATISTICS::default();
        if unsafe { self.swap.GetFrameStatistics(&mut now) }.is_err() {
            self.last_stats = None;
            return None;
        }
        let prev = self.last_stats.replace(now)?;
        let presents = now.PresentCount.wrapping_sub(prev.PresentCount);
        let vblanks = now.SyncRefreshCount.wrapping_sub(prev.SyncRefreshCount);
        let qpc = now.SyncQPCTime.wrapping_sub(prev.SyncQPCTime) as f64;
        let mut freq = 0i64;
        unsafe {
            let _ = windows::Win32::System::Performance::QueryPerformanceFrequency(&mut freq);
        }
        if vblanks == 0 || qpc <= 0.0 || freq <= 0 {
            return None;
        }
        Some(PresentStats {
            missed_vblanks: vblanks.saturating_sub(presents),
            output_hz: vblanks as f64 * freq as f64 / qpc,
        })
    }

    fn ensure_preview_targets(&mut self, width: u32, height: u32) -> Result<()> {
        if self.preview_w == width && self.preview_h == height && self.preview_rt.is_some() {
            return Ok(());
        }
        unsafe {
            let rt_desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut rt = None;
            self.device.CreateTexture2D(&rt_desc, None, Some(&mut rt))?;
            let rt = rt.unwrap();
            let mut rtv = None;
            self.device.CreateRenderTargetView(&rt, None, Some(&mut rtv))?;

            let st_desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut staging = None;
            self.device.CreateTexture2D(&st_desc, None, Some(&mut staging))?;

            self.preview_rt = Some(rt);
            self.preview_rtv = rtv;
            self.preview_staging = staging;
            self.preview_w = width;
            self.preview_h = height;
        }
        Ok(())
    }

    /// Render a panel preview into a CPU RGBA buffer (same shader as the real output).
    pub fn render_preview(
        &mut self,
        cfg: &Config,
        cursor: Option<[f32; 2]>,
        wall: bool,
        width: u32,
    ) -> Result<PreviewImage> {
        let aspect = if wall {
            let cw = cfg.canvas[0].max(1) as f32;
            let ch = cfg.canvas[1].max(1) as f32;
            ch / cw
        } else {
            self.out_h as f32 / (self.out_w as f32).max(1.0)
        };
        let height = ((width as f32) * aspect).round().max(1.0) as u32;
        let width = width.max(1);

        self.ensure_preview_targets(width, height)?;
        let p = self.build_params(cfg, cursor, if wall { 1.0 } else { 0.0 });
        let g = self.build_geometry(cfg);
        let rtv = self.preview_rtv.as_ref().unwrap().clone();
        self.draw(&rtv, width, height, &p, &g);

        let rt = self.preview_rt.as_ref().unwrap();
        let staging = self.preview_staging.as_ref().unwrap();
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        unsafe {
            self.ctx.CopyResource(staging, rt);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.ctx.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let src = mapped.pData as *const u8;
            let pitch = mapped.RowPitch as usize;
            for y in 0..height as usize {
                let row = src.add(y * pitch);
                for x in 0..width as usize {
                    let si = x * 4;
                    let di = (y * width as usize + x) * 4;
                    // BGRA → RGBA, force opaque alpha.
                    rgba[di] = *row.add(si + 2);
                    rgba[di + 1] = *row.add(si + 1);
                    rgba[di + 2] = *row.add(si);
                    rgba[di + 3] = 255;
                }
            }
            self.ctx.Unmap(staging, 0);
        }

        self.preview_seq = self.preview_seq.wrapping_add(1);
        Ok(PreviewImage {
            width,
            height,
            rgba,
            seq: self.preview_seq,
        })
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.latency_wait);
        }
    }
}

fn compile(src: &str, entry: PCSTR, target: PCSTR) -> Result<ID3DBlob> {
    unsafe {
        let mut code = None;
        let mut err = None;
        let r = D3DCompile(
            src.as_ptr() as *const _,
            src.len(),
            s!("blend.hlsl"),
            None,
            None::<&ID3DInclude>,
            entry,
            target,
            D3DCOMPILE_OPTIMIZATION_LEVEL3,
            0,
            &mut code,
            Some(&mut err),
        );
        if let Err(e) = r {
            let msg = err
                .map(|b: ID3DBlob| String::from_utf8_lossy(blob(&b)).into_owned())
                .unwrap_or_default();
            return Err(anyhow!("shader compile failed: {e} {msg}"));
        }
        code.ok_or_else(|| anyhow!("shader compile produced no code"))
    }
}

fn blob(b: &ID3DBlob) -> &[u8] {
    unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use windows::core::s;

    #[test]
    fn cbuffer_sizes_are_16_byte_aligned() {
        assert_eq!(std::mem::size_of::<Params>() % 16, 0);
        assert_eq!(std::mem::size_of::<GeometryBuf>() % 16, 0);
        assert_eq!(std::mem::size_of::<Params>(), 80);
    }

    /// Guards against `[unroll]` on projector loops, which made D3DCompile take minutes.
    #[test]
    fn blend_shader_compiles_quickly() {
        let limit = Duration::from_secs(10);

        let t0 = Instant::now();
        compile(SHADER, s!("vs_main"), s!("vs_5_0")).expect("vs_main compile failed");
        let vs_dt = t0.elapsed();
        assert!(
            vs_dt < limit,
            "vs_main took {vs_dt:?} (limit {limit:?}); check for [unroll] on projector loops"
        );

        let t1 = Instant::now();
        compile(SHADER, s!("ps_main"), s!("ps_5_0")).expect("ps_main compile failed");
        let ps_dt = t1.elapsed();
        assert!(
            ps_dt < limit,
            "ps_main took {ps_dt:?} (limit {limit:?}); check for [unroll] on projector loops"
        );
    }
}

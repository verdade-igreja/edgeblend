//! The blending loop: owns the output window and the D3D11 renderer.
//! Runs on its own thread when driven by the UI, or on the main thread headless.

use anyhow::{anyhow, bail, Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use edgeblend_config::{Config, Preview};
use edgeblend_gpu::display;
use edgeblend_gpu::{DeviceLost, PreviewImage, Renderer};

const HK_QUIT: i32 = 1;
const HK_TEST: i32 = 2;
const HK_CURSOR: i32 = 3;

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub running: bool,
    pub info: String,
    pub error: Option<String>,
}

/// A running blend thread. Dropping it stops the thread and waits for it.
pub struct Engine {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn start(
        cfg: Arc<Mutex<Config>>,
        status: Arc<Mutex<Status>>,
        preview: Arc<Mutex<Option<PreviewImage>>>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = std::thread::Builder::new()
            .name("blend".into())
            .spawn(move || {
                *status.lock().unwrap() = Status { running: true, info: "starting…".into(), error: None };
                let result = run(&cfg, &status, &stop2, &preview);
                let mut st = status.lock().unwrap();
                st.running = false;
                match result {
                    Ok(()) => st.info = "stopped".into(),
                    Err(e) => {
                        st.info = "stopped (error)".into();
                        st.error = Some(format!("{e:#}"));
                    }
                }
            })
            .expect("spawn blend thread");
        Self { stop, handle: Some(handle) }
    }

    pub fn is_running(&self) -> bool {
        self.handle.as_ref().is_some_and(|h| !h.is_finished())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// After the swapchain is ready for a frame, how long to wait for a fresh source image.
/// Long enough to catch a frame that lands just after we start, short enough to leave
/// time to render before the next vblank (rendering takes ~1 ms).
const CAPTURE_WAIT: Duration = Duration::from_millis(6);
/// Pause before rebuilding after a device loss, display change or startup error.
const RETRY_DELAY: Duration = Duration::from_secs(2);

/// Set by the output window on WM_DISPLAYCHANGE (resolution, refresh, display added/removed).
static DISPLAY_CHANGED: AtomicBool = AtomicBool::new(false);

enum SessionEnd {
    Quit,
    Rebuild(&'static str),
}

/// Blend until `stop` is set or Ctrl+Alt+Q is pressed. Reads `cfg` every frame.
/// Survives GPU resets, display mode changes and a missing display by rebuilding the
/// window and renderer, so a running blend never stays dead.
pub fn run(
    cfg: &Mutex<Config>,
    status: &Mutex<Status>,
    stop: &AtomicBool,
    preview: &Mutex<Option<PreviewImage>>,
) -> Result<()> {
    cfg.lock().unwrap().clone().validate()?;
    let mut last_err = String::new();
    while !stop.load(Ordering::Relaxed) {
        match session(cfg, status, stop, preview) {
            Ok(SessionEnd::Quit) => return Ok(()),
            Ok(SessionEnd::Rebuild(why)) => {
                eprintln!("{why}; rebuilding output");
                status.lock().unwrap().info = format!("{why}; restarting output…");
            }
            Err(e) => {
                let msg = format!("{e:#}");
                if msg != last_err {
                    eprintln!("{msg}; retrying every {}s", RETRY_DELAY.as_secs());
                    last_err = msg.clone();
                }
                let mut st = status.lock().unwrap();
                st.info = "waiting to retry…".into();
                st.error = Some(msg);
            }
        }
        let until = Instant::now() + RETRY_DELAY;
        while Instant::now() < until && !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    Ok(())
}

/// Per-second counters for the status line.
#[derive(Default)]
struct Counters {
    frames: u32,
    /// Presents that showed no new source image (the previous one again).
    repeated: u32,
    /// Source images that were never shown (more than one arrived between presents).
    skipped: u32,
}

/// One window + renderer lifetime.
fn session(
    cfg: &Mutex<Config>,
    status: &Mutex<Status>,
    stop: &AtomicBool,
    preview: &Mutex<Option<PreviewImage>>,
) -> Result<SessionEnd> {
    let c = cfg.lock().unwrap().clone();
    let displays = display::enumerate()?;
    let find = |name: &str| {
        displays
            .iter()
            .find(|d| d.name.eq_ignore_ascii_case(name))
            .with_context(|| format!("display {name} not found"))
    };
    let src = find(&c.source)?;
    let out = find(&c.output)?;
    if src.adapter_luid != out.adapter_luid {
        bail!("source ({}) and output ({}) are on different GPUs; not supported yet", src.adapter_name, out.adapter_name);
    }

    let hwnd = unsafe { create_window(out.rect)? };
    DISPLAY_CHANGED.store(false, Ordering::SeqCst);
    let result = (|| -> Result<SessionEnd> {
        unsafe {
            let mods = MOD_CONTROL | MOD_ALT | MOD_NOREPEAT;
            for (id, key) in [(HK_QUIT, b'Q'), (HK_TEST, b'T'), (HK_CURSOR, b'C')] {
                if RegisterHotKey(hwnd, id, mods, key as u32).is_err() {
                    eprintln!("warning: could not register Ctrl+Alt+{}", key as char);
                }
            }
        }
        let mut r = Renderer::new(&src.adapter, &src.output, (src.width(), src.height()), hwnd, out.width(), out.height())?;
        let label = |n: &str| n.trim_start_matches(r"\\.\").to_string();
        let mut n = Counters::default();
        let mut first_frame = true;
        let mut tick = Instant::now();
        let mut shader_check = Instant::now();
        let mut preview_tick = Instant::now();
        let mut preview_err_logged = false;

        while !stop.load(Ordering::Relaxed) {
            // Render right after the previous frame reaches the screen, not one frame early.
            r.wait_for_present_slot();
            unsafe {
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    match msg.message {
                        WM_QUIT => return Ok(SessionEnd::Quit),
                        WM_HOTKEY => {
                            let mut c = cfg.lock().unwrap();
                            match msg.wParam.0 as i32 {
                                HK_QUIT => return Ok(SessionEnd::Quit),
                                HK_TEST => c.pattern = c.pattern.next(),
                                HK_CURSOR => c.show_cursor = !c.show_cursor,
                                _ => {}
                            }
                        }
                        _ => {
                            let _ = TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }
                }
            }
            if DISPLAY_CHANGED.swap(false, Ordering::SeqCst) {
                return Ok(SessionEnd::Rebuild("display configuration changed"));
            }

            if shader_check.elapsed() > Duration::from_millis(500) {
                shader_check = Instant::now();
                r.reload_shader_if_changed();
            }

            let cap = match r.capture(CAPTURE_WAIT) {
                Ok(cap) => cap,
                Err(e) if e.is::<DeviceLost>() => return Ok(SessionEnd::Rebuild("GPU device lost")),
                Err(e) => return Err(e),
            };
            n.frames += 1;
            if !cap.new_image {
                n.repeated += 1;
            }
            n.skipped += cap.accumulated.saturating_sub(1);

            let c = {
                let mut c = cfg.lock().unwrap().clone();
                c.ensure_geometry();
                c
            };
            let cursor = if c.show_cursor { cursor_in(src.rect) } else { None };
            match r.render(&c, cursor) {
                Ok(()) => {}
                Err(e) if e.is::<DeviceLost>() => {
                    eprintln!("{e:#}");
                    return Ok(SessionEnd::Rebuild("GPU device lost"));
                }
                Err(e) => return Err(e),
            }
            if first_frame {
                first_frame = false;
                status.lock().unwrap().error = None;
            }

            if c.preview != Preview::Off && preview_tick.elapsed() >= Duration::from_millis(200) {
                preview_tick = Instant::now();
                let wall = c.preview == Preview::Wall;
                match r.render_preview(&c, cursor, wall, 640) {
                    Ok(img) => {
                        *preview.lock().unwrap() = Some(img);
                        preview_err_logged = false;
                    }
                    Err(e) => {
                        if !preview_err_logged {
                            eprintln!("preview render failed: {e:#}");
                            preview_err_logged = true;
                        }
                    }
                }
            }

            let dt = tick.elapsed();
            if dt >= Duration::from_secs(1) {
                let fps = n.frames as f32 / dt.as_secs_f32();
                let size = r.capture_size().map_or("waiting for desktop…".into(), |(w, h)| format!("capturing {w}x{h}"));
                let mut line = format!(
                    "{} → {} · {size} · {fps:.0} fps · repeated {} · skipped {}",
                    label(&src.name),
                    label(&out.name),
                    n.repeated,
                    n.skipped
                );
                if let Some(ps) = r.present_stats() {
                    line += &format!(" · missed vblanks {}", ps.missed_vblanks);
                    let src_hz = r.source_hz().map_or("?".into(), |hz| format!("{hz:.2}"));
                    line += &format!(" · {src_hz} → {:.2} Hz", ps.output_hz);
                }
                status.lock().unwrap().info = line;
                n = Counters::default();
                tick = Instant::now();
            }
        }
        Ok(SessionEnd::Quit)
    })();

    unsafe {
        for id in [HK_QUIT, HK_TEST, HK_CURSOR] {
            let _ = UnregisterHotKey(hwnd, id);
        }
        let _ = DestroyWindow(hwnd);
    }
    result
}

/// Cursor position relative to `rect`, if visible and inside it.
fn cursor_in(rect: RECT) -> Option<[f32; 2]> {
    let mut ci = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
    unsafe { GetCursorInfo(&mut ci).ok()? };
    if ci.flags.0 & CURSOR_SHOWING.0 == 0 {
        return None;
    }
    let p = ci.ptScreenPos;
    if p.x < rect.left || p.x >= rect.right || p.y < rect.top || p.y >= rect.bottom {
        return None;
    }
    Some([(p.x - rect.left) as f32, (p.y - rect.top) as f32])
}

unsafe fn create_window(r: RECT) -> Result<HWND> {
    let hinst = GetModuleHandleW(None)?;
    let class = w!("EdgeBlendOutput");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: hinst.into(),
        lpszClassName: class,
        ..Default::default()
    };
    RegisterClassW(&wc); // fails harmlessly if already registered by a previous start
    CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        class,
        w!("edgeblend output"),
        WS_POPUP | WS_VISIBLE,
        r.left,
        r.top,
        r.right - r.left,
        r.bottom - r.top,
        None,
        None,
        hinst,
        None,
    )
    .map_err(|e| anyhow!("creating output window: {e}"))
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            // Hide the hardware cursor over the projector output.
            WM_SETCURSOR => {
                SetCursor(None);
                LRESULT(1)
            }
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_DISPLAYCHANGE => {
                DISPLAY_CHANGED.store(true, Ordering::SeqCst);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

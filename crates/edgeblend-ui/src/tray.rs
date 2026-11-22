//! Notification-area icon on its own thread (raw Win32). Left click shows the panel, right
//! click opens a menu. The panel window is shown/hidden with Win32 directly because eframe
//! stops calling `update` while its window is hidden.

use eframe::egui;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::blender::Blender;

/// Must match the title set in `run`.
pub const PANEL_TITLE: PCWSTR = w!("edgeblend");

const WM_TRAY: u32 = WM_APP + 1;
const ID_SHOW: usize = 1;
const ID_TOGGLE: usize = 2;
const ID_QUIT: usize = 3;
const ID_REMOTE: usize = 4;
const ICON_ID: u32 = 1;

struct TrayState {
    blender: Arc<Blender>,
    ctx: egui::Context,
}

static STATE: OnceLock<TrayState> = OnceLock::new();
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

fn panel_hwnd() -> Option<HWND> {
    unsafe { FindWindowW(PCWSTR::null(), PANEL_TITLE).ok() }
}

/// Show (and restore, if minimized) the panel window. Also used by a second instance.
pub fn show_panel() {
    let Some(hwnd) = panel_hwnd() else { return };
    unsafe {
        let cmd = if IsIconic(hwnd).as_bool() { SW_RESTORE } else { SW_SHOW };
        let _ = ShowWindow(hwnd, cmd);
        let _ = SetForegroundWindow(hwnd);
    }
    if let Some(s) = STATE.get() {
        s.ctx.request_repaint();
    }
}

pub fn hide_panel() {
    if let Some(hwnd) = panel_hwnd() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

/// Start the tray thread. Returns true once its window exists (the icon itself is retried
/// on "TaskbarCreated", so it may legitimately be missing at logon before explorer is up).
pub fn spawn(ctx: egui::Context, blender: Arc<Blender>) -> bool {
    if STATE.set(TrayState { blender, ctx }).is_err() {
        return false;
    }
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("tray".into()).spawn(move || unsafe {
        match create_window() {
            Some(hwnd) => {
                let _ = add_icon(hwnd);
                let _ = tx.send(true);
                message_loop();
            }
            None => {
                let _ = tx.send(false);
            }
        }
    });
    spawned.is_ok() && rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap_or(false)
}

unsafe fn create_window() -> Option<HWND> {
    let hinstance = GetModuleHandleW(None).ok()?.into();
    let class = w!("edgeblend-tray");
    let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hinstance, lpszClassName: class, ..Default::default() };
    if RegisterClassW(&wc) == 0 {
        return None;
    }
    // Hidden top-level (not message-only) window: message-only windows miss the
    // "TaskbarCreated" broadcast.
    TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::SeqCst);
    CreateWindowExW(WS_EX_TOOLWINDOW, class, w!(""), WS_POPUP, 0, 0, 0, 0, None, None, hinstance, None).ok()
}

unsafe fn message_loop() {
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..Default::default()
    };
    for (dst, src) in nid.szTip.iter_mut().zip("edgeblend".encode_utf16()) {
        *dst = src;
    }
    nid
}

unsafe fn add_icon(hwnd: HWND) -> bool {
    let mut nid = icon_data(hwnd);
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = LoadIconW(None, IDI_APPLICATION).unwrap_or_default(); // shared stock icon: no destroy
    Shell_NotifyIconW(NIM_ADD, &nid).as_bool()
}

unsafe fn remove_icon(hwnd: HWND) {
    let _ = Shell_NotifyIconW(NIM_DELETE, &icon_data(hwnd));
}

unsafe fn context_menu(hwnd: HWND, state: &TrayState) {
    let Ok(menu) = CreatePopupMenu() else { return };
    let toggle = if state.blender.running() { w!("Stop blending") } else { w!("Start blending") };
    let _ = AppendMenuW(menu, MF_STRING, ID_SHOW, w!("Show panel"));
    let _ = AppendMenuW(menu, MF_STRING, ID_TOGGLE, toggle);
    let remote = if state.blender.remote_info().is_some() { w!("Remote control (on)…") } else { w!("Remote control…") };
    let _ = AppendMenuW(menu, MF_STRING, ID_REMOTE, remote);
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, w!("Quit"));

    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    // Required so the menu closes when the user clicks elsewhere.
    let _ = SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_RETURNCMD, pt.x, pt.y, 0, hwnd, None);
    let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
    let _ = DestroyMenu(menu);

    match cmd.0 as usize {
        ID_SHOW => show_panel(),
        ID_TOGGLE => {
            if state.blender.running() {
                state.blender.stop();
            } else {
                state.blender.start();
            }
            state.ctx.request_repaint();
        }
        ID_REMOTE => {
            state.blender.start_remote(&state.ctx);
            state.blender.remote_window.store(true, Ordering::SeqCst);
            show_panel();
        }
        ID_QUIT => {
            state.blender.shutdown();
            remove_icon(hwnd);
            std::process::exit(0);
        }
        _ => {}
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_TRAY {
        match (lp.0 as u32) & 0xFFFF {
            WM_LBUTTONUP | WM_LBUTTONDBLCLK => show_panel(),
            WM_RBUTTONUP => {
                if let Some(state) = STATE.get() {
                    context_menu(hwnd, state);
                }
            }
            _ => {}
        }
        return LRESULT(0);
    }
    if msg == TASKBAR_CREATED.load(Ordering::SeqCst) && msg != 0 {
        let _ = add_icon(hwnd); // explorer restarted: icons are gone
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

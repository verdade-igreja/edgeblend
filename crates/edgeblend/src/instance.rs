//! Single-instance guard (named mutex) for the panel and the watchdog.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;

pub const PANEL: &str = "Local\\edgeblend-panel";
pub const WATCHDOG: &str = "Local\\edgeblend-watchdog";

/// Holds the mutex until dropped (or process exit).
pub struct Guard(HANDLE);

impl Drop for Guard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// `Some(guard)` if we are the first instance; `None` if another process holds the mutex.
pub fn acquire(name: &str) -> Option<Guard> {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let handle = CreateMutexW(None, false, PCWSTR(wide.as_ptr())).ok()?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(handle);
            return None;
        }
        Some(Guard(handle))
    }
}

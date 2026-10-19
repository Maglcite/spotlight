//! Windows-only window and console helpers.

use raw_window_handle::RawWindowHandle;
use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
};
use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, GetCursorPos, GetWindowRect, SWP_NOACTIVATE, SWP_NOSIZE,
    SWP_NOZORDER, SetForegroundWindow, SetWindowPos,
};

/// GUI-subsystem binaries have no console. Reuse the calling terminal's, if
/// any, so `search`, `ask`, `--help` and `--version` print where they were run.
pub fn attach_parent_console() {
    // SAFETY: plain Win32 call; failure just means there is no parent console.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// Called by a forwarding invocation, which owns the foreground because the
/// user just ran it, so the primary instance may bring its window forward.
pub fn allow_foreground_handoff() {
    // SAFETY: plain Win32 call without pointers.
    unsafe {
        AllowSetForegroundWindow(ASFW_ANY);
    }
}

/// Prepare a freshly opened launcher window: move it to the monitor under the
/// cursor, bring it to the front, and apply a native backdrop. Returns whether
/// a backdrop (acrylic, mica or blur) was applied.
pub fn prepare(window: &dyn iced::window::Window) -> bool {
    if let Some(hwnd) = hwnd(window) {
        center_on_cursor_monitor(hwnd);
        // SAFETY: `hwnd` is a live window owned by this thread's event loop.
        unsafe {
            SetForegroundWindow(hwnd);
        }
    }
    let tint = Some((16, 13, 24, 120));
    if let Err(error) = window_vibrancy::apply_acrylic(window, tint) {
        tracing::debug!("acrylic unavailable: {error}");
        if let Err(error) = window_vibrancy::apply_mica(window, Some(true)) {
            tracing::debug!("mica unavailable: {error}");
            if let Err(error) = window_vibrancy::apply_blur(window, tint) {
                tracing::debug!("blur unavailable: {error}");
                return false;
            }
        }
    }
    true
}

fn hwnd(window: &dyn iced::window::Window) -> Option<HWND> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as HWND),
        _ => None,
    }
}

/// iced centers new windows on the primary monitor; Spotlight should open
/// where the user is working.
fn center_on_cursor_monitor(hwnd: HWND) {
    let mut cursor = POINT { x: 0, y: 0 };
    let mut window = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        rcMonitor: window,
        rcWork: window,
        dwFlags: 0,
    };
    // SAFETY: every pointer refers to a properly sized local, and `cbSize`
    // is set as GetMonitorInfoW requires.
    unsafe {
        if GetCursorPos(&mut cursor) == 0 || GetWindowRect(hwnd, &mut window) == 0 {
            return;
        }
        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return;
        }
        let work = info.rcWork;
        let (x, y) = centered(
            (work.left, work.top, work.right, work.bottom),
            (window.right - window.left, window.bottom - window.top),
        );
        if (x, y) != (window.left, window.top) {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

/// Top-left corner that centers a `size` window in the `area` rectangle
/// (left, top, right, bottom), kept inside the area.
fn centered(area: (i32, i32, i32, i32), size: (i32, i32)) -> (i32, i32) {
    let (left, top, right, bottom) = area;
    (
        left + ((right - left - size.0) / 2).max(0),
        top + ((bottom - top - size.1) / 2).max(0),
    )
}

#[cfg(test)]
mod tests {
    use super::centered;

    #[test]
    fn windows_center_in_the_work_area() {
        assert_eq!(centered((0, 0, 1920, 1040), (740, 620)), (590, 210));
        assert_eq!(centered((-1280, 0, 0, 1024), (740, 620)), (-1010, 202));
        assert_eq!(centered((0, 0, 600, 500), (740, 620)), (0, 0));
    }
}

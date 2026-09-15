//! Win32 window management for the notch.
//!
//! Four things matter here, and all of them are things a normal window gets
//! wrong for this use case:
//!
//! 1. **Never steal focus.** `WS_EX_NOACTIVATE` plus `SWP_NOACTIVATE` on every
//!    move means typing in the IDE or terminal is never interrupted, even as
//!    the HUD resizes underneath the pointer.
//! 2. **Stay out of the way.** `WS_EX_TOOLWINDOW` (and clearing
//!    `WS_EX_APPWINDOW`) keeps the notch out of Alt+Tab and off the taskbar.
//! 3. **Stay on top.** `HWND_TOPMOST`, reasserted periodically because other
//!    topmost windows (and full-screen apps) can displace us.
//! 4. **Click-through when resting.** `WS_EX_TRANSPARENT` is toggled so the
//!    collapsed pill doesn't swallow clicks meant for whatever is underneath.

use anyhow::{anyhow, Result};

use codenotch_core::config::Edge;
use codenotch_core::layout::{place, Placement, WorkArea};

use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, POINT, RECT, TRUE, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_SYSTEMBACKDROP_TYPE,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DWM_SYSTEMBACKDROP_TYPE,
};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
    PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetCursorPos, GetForegroundWindow, GetWindowLongPtrW,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    SetForegroundWindow, SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    GWL_EXSTYLE, HWND_TOPMOST, LWA_ALPHA, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_RESTORE,
    WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};

use super::{Backdrop, WindowHandle};

/// Win11 build 22621 names these; older SDK headers don't have the constants.
const DWMSBT_NONE: i32 = 1;
const DWMSBT_MAINWINDOW: i32 = 2; // Mica
const DWMSBT_TRANSIENTWINDOW: i32 = 3; // Acrylic

fn hwnd(handle: WindowHandle) -> HWND {
    HWND(handle as *mut core::ffi::c_void)
}

/// Apply the extended styles that make this window behave like a HUD.
///
/// Safe to call repeatedly; each call re-reads the current style bits so it
/// composes with whatever the webview host has set.
pub fn apply_hud_chrome(handle: WindowHandle, click_through: bool) -> Result<()> {
    let hwnd = hwnd(handle);

    // SAFETY: `hwnd` comes from Tauri and is valid for the window's lifetime.
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;

        let mut style = current
            | WS_EX_TOOLWINDOW.0   // out of Alt+Tab and the taskbar
            | WS_EX_NOACTIVATE.0   // clicking never activates us
            | WS_EX_LAYERED.0; // required for per-window alpha
        style &= !WS_EX_APPWINDOW.0; // undo any "real app window" marking

        if click_through {
            style |= WS_EX_TRANSPARENT.0;
        } else {
            style &= !WS_EX_TRANSPARENT.0;
        }

        if style != current {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style as isize);
        }

        // WS_EX_LAYERED windows start fully transparent until an alpha is set.
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
    }

    set_topmost(handle)?;
    Ok(())
}

/// Toggle click-through without disturbing the other style bits.
pub fn set_click_through(handle: WindowHandle, enabled: bool) -> Result<()> {
    let hwnd = hwnd(handle);
    // SAFETY: see `apply_hud_chrome`.
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let style = if enabled {
            current | WS_EX_TRANSPARENT.0
        } else {
            current & !WS_EX_TRANSPARENT.0
        };
        if style != current {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style as isize);
        }
    }
    Ok(())
}

/// Re-assert topmost z-order without moving, resizing or activating.
pub fn set_topmost(handle: WindowHandle) -> Result<()> {
    // SAFETY: flags guarantee position/size arguments are ignored.
    unsafe {
        SetWindowPos(
            hwnd(handle),
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )?;
    }
    Ok(())
}

/// Move and resize in physical pixels, without activating the window.
///
/// This is the call that makes hover-expand feel right: the window can grow
/// under the pointer while the user keeps typing somewhere else.
pub fn move_no_activate(handle: WindowHandle, placement: Placement) -> Result<()> {
    // SAFETY: `hwnd` is valid; SWP_NOACTIVATE keeps focus where it is.
    unsafe {
        SetWindowPos(
            hwnd(handle),
            Some(HWND_TOPMOST),
            placement.x,
            placement.y,
            placement.width,
            placement.height,
            SWP_NOACTIVATE,
        )?;
    }
    Ok(())
}

/// Ask DWM for rounded corners, a dark frame, an accent border and a backdrop.
///
/// Every call is best-effort: these attributes are Windows 11 era, and on
/// Windows 10 they simply return an error we ignore rather than failing setup.
pub fn apply_appearance(handle: WindowHandle, backdrop: Backdrop, accent: Option<u32>) {
    let hwnd = hwnd(handle);

    // SAFETY: each call passes a correctly sized value for its attribute.
    unsafe {
        let dark: BOOL = TRUE;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const _ as *const _,
            std::mem::size_of::<BOOL>() as u32,
        );

        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const _,
            std::mem::size_of::<i32>() as u32,
        );

        if let Some(rgb) = accent {
            // DWM wants COLORREF (0x00BBGGRR), not the 0xRRGGBB we store.
            let colour = COLORREF(swap_rgb(rgb));
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_BORDER_COLOR,
                &colour as *const _ as *const _,
                std::mem::size_of::<COLORREF>() as u32,
            );
        }

        // `Inherit` deliberately leaves the backdrop alone; see `Backdrop`.
        if let Some(kind) = match backdrop {
            Backdrop::Inherit => None,
            Backdrop::Acrylic => Some(DWMSBT_TRANSIENTWINDOW),
            Backdrop::Mica => Some(DWMSBT_MAINWINDOW),
            Backdrop::None => Some(DWMSBT_NONE),
        } {
            let value = DWM_SYSTEMBACKDROP_TYPE(kind);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                &value as *const _ as *const _,
                std::mem::size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
            );
        }
    }
}

/// `0xRRGGBB` -> `0x00BBGGRR`, the byte order COLORREF uses.
pub fn swap_rgb(rgb: u32) -> u32 {
    let r = (rgb >> 16) & 0xFF;
    let g = (rgb >> 8) & 0xFF;
    let b = rgb & 0xFF;
    (b << 16) | (g << 8) | r
}

/// Parse `#rrggbb` into `0xRRGGBB`.
pub fn parse_hex_colour(hex: &str) -> Option<u32> {
    let hex = hex.trim().strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

/// Enumerate monitor work areas in the order Windows reports them.
///
/// The work area excludes the taskbar, which is what "docked near the taskbar"
/// has to mean if the notch isn't going to sit underneath it.
pub fn monitors() -> Vec<WorkArea> {
    let mut found: Vec<WorkArea> = Vec::new();

    unsafe extern "system" fn callback(
        monitor: HMONITOR,
        _dc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        // SAFETY: `data` is the &mut Vec we passed to EnumDisplayMonitors, which
        // outlives the enumeration.
        let out = unsafe { &mut *(data.0 as *mut Vec<WorkArea>) };

        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: `info.cbSize` is set as the API requires.
        if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            let work = info.rcWork;
            out.push(WorkArea::new(
                work.left,
                work.top,
                work.right - work.left,
                work.bottom - work.top,
                // Per-monitor DPI is resolved later against the actual window;
                // 1.0 keeps this a pure geometry query.
                1.0,
            ));
        }
        TRUE
    }

    // SAFETY: `found` outlives the synchronous enumeration below.
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(callback),
            LPARAM(&mut found as *mut Vec<WorkArea> as isize),
        );
    }
    found
}

/// The primary monitor's work area: the one containing the origin.
pub fn primary_work_area() -> Option<WorkArea> {
    let all = monitors();
    all.iter()
        .find(|m| m.x <= 0 && m.y <= 0 && m.x + m.width > 0 && m.y + m.height > 0)
        .copied()
        .or_else(|| all.first().copied())
}

/// Work area for a monitor index, falling back to the primary.
pub fn work_area_for(index: Option<usize>) -> Option<WorkArea> {
    match index {
        Some(i) => monitors().get(i).copied().or_else(primary_work_area),
        None => primary_work_area(),
    }
}

/// The DPI scale currently applied to this window (1.0 = 96 DPI).
pub fn window_scale(handle: WindowHandle) -> f64 {
    // SAFETY: GetDpiForWindow returns 0 for an invalid handle, handled below.
    let dpi = unsafe { GetDpiForWindow(hwnd(handle)) };
    if dpi == 0 {
        1.0
    } else {
        dpi as f64 / 96.0
    }
}

/// Compute where the HUD belongs and move it there, in one call.
pub fn dock(
    handle: WindowHandle,
    monitor: Option<usize>,
    edge: Edge,
    offset: f32,
    margin: f64,
    logical_width: f64,
    logical_height: f64,
) -> Result<Placement> {
    let mut area =
        work_area_for(monitor).ok_or_else(|| anyhow!("no monitors reported a work area"))?;
    area.scale = window_scale(handle);

    let placement = place(area, edge, offset, margin, logical_width, logical_height);
    move_no_activate(handle, placement)?;
    Ok(placement)
}

/// Where the mouse is, in physical screen pixels.
///
/// A click-through window (`WS_EX_TRANSPARENT`) receives no mouse messages at
/// all -- not even hover -- so the webview cannot detect the pointer arriving.
/// Polling the cursor is what lets the notch stay click-through while resting
/// and still open when you move onto it.
pub fn cursor_pos() -> Option<(i32, i32)> {
    let mut point = POINT::default();
    // SAFETY: `point` is a plain out-parameter owned by this frame.
    unsafe { GetCursorPos(&mut point).ok()? };
    Some((point.x, point.y))
}

/// Basename of a process's executable, e.g. `Cursor.exe`.
fn process_image_name(pid: u32) -> Option<String> {
    // SAFETY: the handle is closed by its Owned wrapper when it drops.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;

        let mut buffer = [0u16; 260];
        let mut len = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
        .is_ok();

        if !ok {
            return None;
        }
        let full = String::from_utf16_lossy(&buffer[..len as usize]);
        full.rsplit(['\\', '/']).next().map(str::to_string)
    }
}

/// A candidate window found while searching for a provider's UI.
struct Candidate {
    hwnd: HWND,
    title: String,
    image: String,
}

/// Find visible top-level windows whose executable matches `process_names`.
fn find_windows(process_names: &[&str]) -> Vec<Candidate> {
    struct Search {
        wanted: Vec<String>,
        found: Vec<Candidate>,
    }

    unsafe extern "system" fn callback(window: HWND, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the &mut Search passed to EnumWindows.
        let search = unsafe { &mut *(data.0 as *mut Search) };

        // SAFETY: `window` is valid for the duration of the callback.
        unsafe {
            if !IsWindowVisible(window).as_bool() {
                return TRUE;
            }
            let len = GetWindowTextLengthW(window);
            if len <= 0 {
                return TRUE; // untitled windows are tool/host windows, not the UI
            }

            let mut buffer = vec![0u16; len as usize + 1];
            let written = GetWindowTextW(window, &mut buffer);
            let title = String::from_utf16_lossy(&buffer[..written as usize]);

            let mut pid = 0u32;
            GetWindowThreadProcessId(window, Some(&mut pid));
            let Some(image) = process_image_name(pid) else {
                return TRUE;
            };

            if search.wanted.iter().any(|w| w.eq_ignore_ascii_case(&image)) {
                search.found.push(Candidate {
                    hwnd: window,
                    title,
                    image,
                });
            }
        }
        TRUE
    }

    let mut search = Search {
        wanted: process_names.iter().map(|s| s.to_string()).collect(),
        found: Vec::new(),
    };

    // SAFETY: `search` outlives the synchronous enumeration.
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(&mut search as *mut Search as isize));
    }
    search.found
}

/// Bring a window to the foreground.
///
/// Windows only lets the *foreground* thread set the foreground window, so the
/// usual trick is to attach our input queue to the current foreground thread
/// for the duration of the call. Without it `SetForegroundWindow` silently
/// no-ops and the taskbar button just flashes.
fn focus_window(target: HWND) -> bool {
    // SAFETY: every handle is validated by the API; the attach is undone below.
    unsafe {
        if IsIconic(target).as_bool() {
            let _ = ShowWindow(target, SW_RESTORE);
        }

        let foreground = GetForegroundWindow();
        let foreground_thread = GetWindowThreadProcessId(foreground, None);
        let our_thread = GetCurrentThreadId();

        let attached = foreground_thread != 0
            && foreground_thread != our_thread
            && AttachThreadInput(our_thread, foreground_thread, true).as_bool();

        let _ = BringWindowToTop(target);
        let ok = SetForegroundWindow(target).as_bool();
        let _ = SetFocus(Some(target));

        if attached {
            let _ = AttachThreadInput(our_thread, foreground_thread, false);
        }
        ok
    }
}

/// Bring a provider's window to the front.
///
/// `title_hint` (typically the project folder) picks between several windows of
/// the same application, so clicking a card lands on the right project rather
/// than an arbitrary editor window.
pub fn focus_provider_window(process_names: &[&str], title_hint: Option<&str>) -> Result<bool> {
    let candidates = find_windows(process_names);
    if candidates.is_empty() {
        return Ok(false);
    }

    let chosen = title_hint
        .and_then(|hint| {
            let hint = hint.to_lowercase();
            candidates
                .iter()
                .find(|c| c.title.to_lowercase().contains(&hint))
        })
        // Otherwise prefer the earliest-listed executable, which is the order
        // the caller ranked them in.
        .or_else(|| {
            process_names.iter().find_map(|name| {
                candidates
                    .iter()
                    .find(|c| c.image.eq_ignore_ascii_case(name))
            })
        })
        .or_else(|| candidates.first());

    Ok(chosen.is_some_and(|c| focus_window(c.hwnd)))
}

/// Registry path for per-user startup entries.
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "CodeNotch";

/// Whether CodeNotch is registered to start at sign-in.
pub fn launch_at_login() -> bool {
    use windows::Win32::System::Registry::{
        RegCloseKey, RegGetValueW, HKEY, HKEY_CURRENT_USER, RRF_RT_REG_SZ,
    };

    // SAFETY: buffer size is passed and updated by the API.
    unsafe {
        let mut key = HKEY::default();
        let sub = wide(RUN_KEY);
        if windows::Win32::System::Registry::RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(sub.as_ptr()),
            None,
            windows::Win32::System::Registry::KEY_READ,
            &mut key,
        )
        .is_err()
        {
            return false;
        }

        let name = wide(RUN_VALUE);
        let mut size = 0u32;
        let present = RegGetValueW(
            key,
            None,
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
        .is_ok();

        let _ = RegCloseKey(key);
        present && size > 0
    }
}

/// Add or remove the startup entry.
pub fn set_launch_at_login(enabled: bool) -> Result<()> {
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
        REG_SZ,
    };

    let exe = std::env::current_exe()?;
    // Quote the path: Run entries are parsed as command lines, and
    // `C:\Program Files\...` would otherwise split at the space.
    let command = format!("\"{}\"", exe.display());

    // SAFETY: handles are closed on every path; buffers are NUL-terminated.
    unsafe {
        let mut key = HKEY::default();
        let sub = wide(RUN_KEY);
        windows::Win32::System::Registry::RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(sub.as_ptr()),
            None,
            KEY_SET_VALUE,
            &mut key,
        )
        .ok()?;

        let name = wide(RUN_VALUE);
        let result = if enabled {
            let value = wide(&command);
            let bytes = std::slice::from_raw_parts(
                value.as_ptr() as *const u8,
                value.len() * std::mem::size_of::<u16>(),
            );
            RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes)).ok()
        } else {
            // Removing an entry that isn't there is a success, not a failure.
            let _ = RegDeleteValueW(key, PCWSTR(name.as_ptr()));
            Ok(())
        };

        let _ = RegCloseKey(key);
        result?;
    }
    Ok(())
}

/// NUL-terminated UTF-16, as every `W` API expects.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Nudge the window so DWM repaints the backdrop after a style change.
pub fn refresh_frame(handle: WindowHandle) {
    use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_THEMECHANGED};
    // SAFETY: a theme-changed notification carries no pointer payload.
    unsafe {
        let _ = SendMessageW(
            hwnd(handle),
            WM_THEMECHANGED,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        );
    }
}

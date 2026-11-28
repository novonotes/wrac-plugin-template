use wrac_clap_adapter::interface::GuiSize;

use crate::window::StoredParentWindow;

/// Queries the native parent window's effective DPI scale on Windows.
///
/// The controller inspects the parent HWND where the host cannot be relied on for the scale:
/// some Windows hosts (notably Cubase 10 VST3) report integer scale factors even on
/// fractional-DPI monitors, and AAX has no content-scale callback at all.
#[cfg(windows)]
pub(super) fn parent_window_scale(parent: Option<StoredParentWindow>) -> Option<f64> {
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::HWND;

    type GetDpiForWindowFn = unsafe extern "system" fn(HWND) -> u32;

    let hwnd = parent?.win32_hwnd()? as HWND;
    if hwnd.is_null() {
        return None;
    }

    static GET_DPI_FOR_WINDOW: OnceLock<Option<GetDpiForWindowFn>> = OnceLock::new();
    if let Some(get_dpi_for_window) = *GET_DPI_FOR_WINDOW
        .get_or_init(|| unsafe { load_windows_proc(b"user32.dll\0", b"GetDpiForWindow\0") })
    {
        let window_dpi = unsafe { get_dpi_for_window(hwnd) };
        if window_dpi > 0 {
            return Some(window_dpi as f64 / 96.0);
        }
    }

    monitor_scale_for_parent(parent)
}

/// Queries the DPI scale of the monitor that currently shows the parent window on Windows.
///
/// Unlike the window DPI, this follows the window as soon as it is dragged onto another
/// monitor, before the host reports the new content scale. Returns `None` when the scale
/// cannot be determined; a process that is not per-monitor DPI aware always sees the same
/// value, so monitor changes stay invisible there.
#[cfg(windows)]
pub(super) fn monitor_scale_for_parent(parent: Option<StoredParentWindow>) -> Option<f64> {
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Gdi::{
        HMONITOR, MONITOR_DEFAULTTONEAREST, MonitorFromWindow,
    };
    use windows_sys::Win32::UI::HiDpi::{MDT_EFFECTIVE_DPI, MONITOR_DPI_TYPE};

    type GetDpiForMonitorFn =
        unsafe extern "system" fn(HMONITOR, MONITOR_DPI_TYPE, *mut u32, *mut u32) -> i32;

    let hwnd = parent?.win32_hwnd()? as HWND;
    if hwnd.is_null() {
        return None;
    }

    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_null() {
        return None;
    }

    static GET_DPI_FOR_MONITOR: OnceLock<Option<GetDpiForMonitorFn>> = OnceLock::new();
    let get_dpi_for_monitor = (*GET_DPI_FOR_MONITOR
        .get_or_init(|| unsafe { load_windows_proc(b"shcore.dll\0", b"GetDpiForMonitor\0") }))?;

    let mut dpi_x = 0u32;
    let mut dpi_y = 0u32;
    let result = unsafe {
        get_dpi_for_monitor(
            monitor,
            MDT_EFFECTIVE_DPI,
            &mut dpi_x as *mut u32,
            &mut dpi_y as *mut u32,
        )
    };
    if result == 0 && dpi_x > 0 {
        Some(dpi_x as f64 / 96.0)
    } else {
        None
    }
}

#[cfg(windows)]
unsafe fn load_windows_proc<T>(module_name: &[u8], proc_name: &[u8]) -> Option<T>
where
    T: Copy,
{
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

    let module = unsafe { LoadLibraryA(module_name.as_ptr()) };
    if module.is_null() {
        return None;
    }
    let proc = unsafe { GetProcAddress(module, proc_name.as_ptr()) }?;
    Some(unsafe { std::mem::transmute_copy(&proc) })
}

#[cfg(not(windows))]
pub(super) fn parent_window_scale(_parent: Option<StoredParentWindow>) -> Option<f64> {
    None
}

#[cfg(not(windows))]
pub(super) fn monitor_scale_for_parent(_parent: Option<StoredParentWindow>) -> Option<f64> {
    None
}

/// Decides whether a host resize already applied a monitor DPI change to the frame.
///
/// When the editor window lands on a monitor with a different DPI, REAPER first delivers a
/// frame already multiplied by the DPI ratio and only then reports the new scale. Reading
/// that frame at the old scale would record a logical size that is too large (or too
/// small), and the scale report arriving afterwards would apply the ratio once more.
///
/// `monitor_scale_ratio` is the current monitor scale divided by the monitor scale at the
/// time `scale` was reported. Returns the scale to adopt when `requested` is closer to
/// `previous` scaled by that ratio than to `previous` itself, and `None` when the resize
/// is unrelated to the monitor change.
pub(super) fn scale_implied_by_host_resize(
    previous: GuiSize,
    requested: GuiSize,
    scale: f64,
    monitor_scale_ratio: f64,
) -> Option<f64> {
    let distance_from_requested = |width: f64, height: f64| {
        (requested.width as f64 - width).abs() + (requested.height as f64 - height).abs()
    };
    let unscaled = distance_from_requested(previous.width as f64, previous.height as f64);
    let rescaled = distance_from_requested(
        previous.width as f64 * monitor_scale_ratio,
        previous.height as f64 * monitor_scale_ratio,
    );
    (rescaled < unscaled).then_some(scale * monitor_scale_ratio)
}

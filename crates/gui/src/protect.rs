//! Screen-capture protection.
//!
//! Windows 10 2004+: `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` makes the window
//! appear black (or vanish) in screenshots, recordings and screen sharing.
//! Linux (X11/Wayland) and macOS offer no reliable equivalent, so we only warn.

#[cfg(windows)]
pub fn exclude_from_capture(handle: &impl raw_window_handle::HasWindowHandle) -> bool {
    use raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
    };
    let Ok(h) = handle.window_handle() else {
        return false;
    };
    match h.as_raw() {
        RawWindowHandle::Win32(w) => {
            let hwnd = w.hwnd.get() as *mut core::ffi::c_void;
            // SAFETY: hwnd is the live top-level window owned by this process.
            unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) != 0 }
        }
        _ => false,
    }
}

#[cfg(not(windows))]
pub fn exclude_from_capture(_handle: &impl raw_window_handle::HasWindowHandle) -> bool {
    false
}

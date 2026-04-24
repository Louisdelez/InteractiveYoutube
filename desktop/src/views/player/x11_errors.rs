//! Custom X11 error handler installed by the player on boot.
//!
//! Without this, Xlib's default handler prints the error and calls
//! `exit(1)` — which was exactly the SIGSEGV-looking crash we saw on
//! shutdown (BadWindow on X_DestroyWindow at serial ~112). At teardown
//! time the parent GPUI window may already be gone, so the X server has
//! auto-destroyed our child/sibling windows before our Drop impls run
//! their own XDestroyWindow. That's a benign race, not a real bug —
//! swallow it instead of aborting the process.
//!
//! Extracted from `views/player.rs` to keep that file focused on
//! PlayerView state + render.

#![cfg(target_os = "linux")]

static X11_ERROR_HANDLER_INSTALLED: std::sync::Once = std::sync::Once::new();

/// Set to `true` by the top-level Drop teardown path so every
/// subsequent X error is swallowed silently (we're going away anyway).
pub(crate) static X11_SHUTTING_DOWN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

unsafe extern "C" fn x11_error_handler(
    _display: *mut x11_dl::xlib::Display,
    event: *mut x11_dl::xlib::XErrorEvent,
) -> std::os::raw::c_int {
    if X11_SHUTTING_DOWN.load(std::sync::atomic::Ordering::Acquire) {
        return 0;
    }
    let ev = &*event;
    tracing::warn!(
        code = ev.error_code,
        request = ev.request_code,
        resource = format!("0x{:x}", ev.resourceid),
        "X11 error"
    );
    0
}

/// Register our custom handler exactly once per process. Subsequent
/// calls are no-ops (Once guard).
pub(crate) fn install_x11_error_handler(xlib: &x11_dl::xlib::Xlib) {
    X11_ERROR_HANDLER_INSTALLED.call_once(|| unsafe {
        (xlib.XSetErrorHandler)(Some(x11_error_handler));
    });
}

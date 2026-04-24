//! Instrumented wrappers for silent-error hotspots.
//!
//! Before this module, the codebase was sprinkled with ~50 `let _ =
//! mpv.set_property(...)` and `let _ = socket.emit(...)` calls that
//! swallowed any failure silently. In practice this means: if the mpv
//! subprocess hangs, the IPC socket closes, or mpv rejects a property
//! name, the UI keeps pretending everything is fine — then crashes
//! three seconds later with no stack.
//!
//! The `mpv_try!` and `emit_try!` macros keep the ergonomic terseness
//! of `let _ = …` but route failures to `tracing::warn!` with the
//! operation + context, so ops visibility exists even when we choose
//! not to propagate the error.
//!
//! Pure logging layer — no retry, no circuit breaker, no metrics
//! increment. Those are a worthwhile next step but out of scope here.

/// Run an expression that returns `Result<_, E>`; on error, log a
/// structured `warn!` with the operation label + the error, then
/// continue as if `let _ = expr`.
///
/// Usage:
///   mpv_try!(self.mpv.set_property("volume", 50), "set volume");
///   mpv_try!(self.mpv.command("loadfile", &[url]), "loadfile");
///
/// Accepts an optional `ctx: ...` extra field that shows up in the
/// log record alongside `op` + `err`.
#[macro_export]
macro_rules! mpv_try {
    ($expr:expr, $op:expr) => {
        match $expr {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!(op = $op, err = ?e, "mpv IPC call failed");
                None
            }
        }
    };
    ($expr:expr, $op:expr, $ctx:expr) => {
        match $expr {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!(op = $op, ctx = ?$ctx, err = ?e, "mpv IPC call failed");
                None
            }
        }
    };
}

/// Same as `mpv_try!` but for socket.io `emit` calls. Separate macro
/// so log records are distinguishable by `target`/`op` prefix.
#[macro_export]
macro_rules! emit_try {
    ($expr:expr, $event:expr) => {
        match $expr {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!(event = $event, err = ?e, "socket.io emit failed");
                None
            }
        }
    };
}

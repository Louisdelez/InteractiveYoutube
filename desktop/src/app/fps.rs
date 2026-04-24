//! FPS counter — rolling 1-second window of `render()` timestamps.
//! Factored out of `app.rs` so the 1.8k-LOC file has one less tiny
//! responsibility inside it.
//!
//! Usage from `AppView`:
//!
//! ```ignore
//!     // field:
//!     frame_times: FpsCounter,
//!
//!     // in render():
//!     self.frame_times.record();
//!     let fps = self.frame_times.current();
//!     FpsCounter::schedule_next_tick(cx);
//!     // …
//!     .child(div().child(format!("{} fps", fps)))
//! ```
//!
//! Counter is per-instance (Rc<RefCell<…>>) so clones share the same
//! window, but AppView doesn't need to clone it — all reads happen
//! during render on `&mut self`.

use gpui::AppContext;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Size hint for the rolling buffer. At 120 Hz we'd overshoot this
/// briefly but the prune-to-1s loop brings us right back.
const CAPACITY: usize = 128;

/// Tick period for the "keep FPS visible even when nothing changed"
/// forced re-render. 1 s is 5× cheaper than the old 200 ms hack.
const TICK_PERIOD: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub struct FpsCounter {
    times: Rc<RefCell<VecDeque<Instant>>>,
}

impl FpsCounter {
    pub fn new() -> Self {
        Self {
            times: Rc::new(RefCell::new(VecDeque::with_capacity(CAPACITY))),
        }
    }

    /// Call once per `render()`. Appends `now` to the window and prunes
    /// entries older than 1 second.
    pub fn record(&self) {
        let now = Instant::now();
        let cutoff = now - Duration::from_secs(1);
        let mut ft = self.times.borrow_mut();
        ft.push_back(now);
        while ft.front().map_or(false, |t| *t < cutoff) {
            ft.pop_front();
        }
    }

    /// Number of renders in the last second — exactly the displayed
    /// "XX fps" value in the topbar.
    pub fn current(&self) -> usize {
        self.times.borrow().len()
    }

    /// Schedule a 1 s timer that re-notifies the AppView so the FPS
    /// label keeps ticking during idle (nothing else would trigger
    /// a re-render otherwise). Call from inside `render()`.
    pub fn schedule_next_tick<T: 'static>(cx: &mut gpui::Context<T>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TICK_PERIOD).await;
            if let Some(e) = this.upgrade() {
                let _ = cx.update_entity(&e, |_, cx| cx.notify());
            }
        })
        .detach();
    }
}

impl Default for FpsCounter {
    fn default() -> Self {
        Self::new()
    }
}

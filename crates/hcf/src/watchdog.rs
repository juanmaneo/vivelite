//! A watchdog that force-exits the process when a hang case actually hangs.
//!
//! Upstream HCF arms a termination timer (default 120 s) because the whole
//! point of these workloads is that the driver may never return; the same
//! applies on the XR Elite. The exit is injectable so the timing logic is
//! testable without killing the test runner.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// An armed watchdog. Disarm it when the run finished on its own; dropping
/// the handle does **not** disarm it (the process exiting makes that
/// moot).
#[derive(Debug)]
pub struct Watchdog {
    armed: Arc<AtomicBool>,
}

impl Watchdog {
    /// Arms a watchdog that calls `fire` after `timeout`, unless disarmed
    /// first.
    ///
    /// The watchdog is fire-and-forget: `fire` is expected to terminate the
    /// process (the runner exits with [`crate::EXIT_HANG`]); if it returns,
    /// the thread simply ends.
    pub fn arm<F>(timeout: Duration, fire: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        let armed = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&armed);
        // The watchdog thread must outlive the returned handle: a hang
        // usually means the main thread never reaches a join.
        let _ = thread::Builder::new()
            .name("hcf-watchdog".into())
            .spawn(move || {
                thread::sleep(timeout);
                if flag.load(Ordering::SeqCst) {
                    fire();
                }
            });
        Self { armed }
    }

    /// Disarms the watchdog so `fire` will not run.
    pub fn disarm(&self) {
        self.armed.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use super::Watchdog;

    /// One shared closure body, so both tests exercise the same code.
    fn recorder(flag: &Arc<AtomicBool>) -> impl FnOnce() + Send + 'static {
        let seen = Arc::clone(flag);
        move || seen.store(true, Ordering::SeqCst)
    }

    #[test]
    fn fires_after_the_timeout() {
        let fired = Arc::new(AtomicBool::new(false));
        let _watchdog = Watchdog::arm(Duration::from_millis(5), recorder(&fired));
        thread::sleep(Duration::from_millis(60));
        assert!(fired.load(Ordering::SeqCst), "watchdog should have fired");
    }

    #[test]
    fn disarm_cancels_the_shot() {
        let fired = Arc::new(AtomicBool::new(false));
        let watchdog = Watchdog::arm(Duration::from_millis(5), recorder(&fired));
        watchdog.disarm();
        thread::sleep(Duration::from_millis(60));
        assert!(
            !fired.load(Ordering::SeqCst),
            "disarmed watchdog fired anyway"
        );
    }
}

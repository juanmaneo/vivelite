//! A watchdog that force-exits the process when a hang case actually hangs.
//!
//! Upstream HCF arms a termination timer (default 120 s) because the whole
//! point of these workloads is that the driver may never return; the same
//! applies on the XR Elite. The exit is injectable so the timing logic is
//! testable without killing the test runner.

use std::io;
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
    ///
    /// Fails if the watchdog thread cannot be started: an unarmed watchdog
    /// would let a real hang run forever.
    pub fn arm<F>(timeout: Duration, fire: F) -> io::Result<Self>
    where
        F: FnOnce() + Send + 'static,
    {
        Self::arm_with(timeout, fire, spawn_thread)
    }

    /// [`Watchdog::arm`] with the thread spawn injected, so the spawn-failure
    /// path is testable without exhausting OS resources.
    fn arm_with<F>(
        timeout: Duration,
        fire: F,
        spawn: fn(Box<dyn FnOnce() + Send>) -> io::Result<()>,
    ) -> io::Result<Self>
    where
        F: FnOnce() + Send + 'static,
    {
        let armed = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&armed);
        // The watchdog thread must outlive the returned handle: a hang
        // usually means the main thread never reaches a join.
        spawn(Box::new(move || {
            thread::sleep(timeout);
            if flag.load(Ordering::SeqCst) {
                fire();
            }
        }))?;
        Ok(Self { armed })
    }

    /// Disarms the watchdog so `fire` will not run.
    pub fn disarm(&self) {
        self.armed.store(false, Ordering::SeqCst);
    }
}

fn spawn_thread(body: Box<dyn FnOnce() + Send>) -> io::Result<()> {
    thread::Builder::new()
        .name("hcf-watchdog".into())
        .spawn(body)
        .map(drop)
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
        let _watchdog = Watchdog::arm(Duration::from_millis(5), recorder(&fired)).unwrap();
        thread::sleep(Duration::from_millis(60));
        assert!(fired.load(Ordering::SeqCst), "watchdog should have fired");
    }

    #[test]
    fn arm_reports_a_thread_that_cannot_start() {
        let fired = Arc::new(AtomicBool::new(false));
        let err = Watchdog::arm_with(Duration::from_millis(1), recorder(&fired), |_| {
            Err(std::io::Error::other("no threads"))
        })
        .unwrap_err();
        assert_eq!(err.to_string(), "no threads");
        assert!(!fired.load(Ordering::SeqCst), "fire ran without a thread");
    }

    #[test]
    fn spawn_thread_runs_the_body_on_a_real_thread() {
        let (tx, rx) = std::sync::mpsc::channel();
        super::spawn_thread(Box::new(move || tx.send(()).unwrap())).unwrap();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn arm_hands_the_spawn_its_body() {
        let fired = Arc::new(AtomicBool::new(false));
        let body_fired = Arc::clone(&fired);
        let watchdog = Watchdog::arm_with(
            Duration::from_millis(1),
            move || body_fired.store(true, Ordering::SeqCst),
            |body| {
                body();
                Ok(())
            },
        )
        .unwrap();
        assert!(fired.load(Ordering::SeqCst));
        watchdog.disarm();
    }

    #[test]
    fn disarm_cancels_the_shot() {
        let fired = Arc::new(AtomicBool::new(false));
        let watchdog = Watchdog::arm(Duration::from_millis(5), recorder(&fired)).unwrap();
        watchdog.disarm();
        thread::sleep(Duration::from_millis(60));
        assert!(
            !fired.load(Ordering::SeqCst),
            "disarmed watchdog fired anyway"
        );
    }
}

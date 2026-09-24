//! Automatic maintenance of the process-wide compiler caches.

use std::sync::atomic::{AtomicU8, Ordering};

use tinymist_std::time::Instant;

// Keep the working set touched since the preceding sweep. In comemo, a hit
// resets age to zero and a sweep increments age before retaining age <= this
// limit. One unused interval therefore survives; the next sweep releases it.
// This bounds retained generations, not bytes: one document can itself be large.
pub(super) const MAX_UNUSED_AGE: usize = 1;

static EVICTION: EvictionScheduler = EvictionScheduler::new();

/// Request a sweep without spawning one worker per completed compilation.
pub(super) fn schedule() {
    if EVICTION.request() {
        let queued_at = Instant::now();
        super::spawn_cpu(move || {
            log::debug!(
                "ProjectCompiler: automatic cache sweep queued for {:?}",
                queued_at.elapsed()
            );
            EVICTION.run(|| {
                let start = Instant::now();
                comemo::evict(MAX_UNUSED_AGE);
                log::debug!(
                    "ProjectCompiler: evict comemo cache in {:?} (max_age={MAX_UNUSED_AGE})",
                    start.elapsed()
                );
            });
        });
    }
}

const IDLE: u8 = 0;
const RUNNING: u8 = 1;
const PENDING: u8 = 2;

/// Owns one sweep worker and at most one pending follow-up sweep.
struct EvictionScheduler {
    state: AtomicU8,
}

impl EvictionScheduler {
    const fn new() -> Self {
        Self {
            state: AtomicU8::new(IDLE),
        }
    }

    /// Returns whether the caller must start the sole worker.
    fn request(&self) -> bool {
        self.state.swap(PENDING, Ordering::AcqRel) == IDLE
    }

    /// Drains coalesced requests, including ones arriving during a sweep.
    /// The caller must have reserved this worker through `request`.
    fn run(&self, mut evict: impl FnMut()) {
        loop {
            // Requests already pending are satisfied by this sweep. Requests
            // arriving after this swap reserve a follow-up instead.
            self.state.swap(RUNNING, Ordering::AcqRel);
            evict();

            // Release ownership atomically with checking for more work. A
            // racing request either keeps this worker alive or starts a new
            // worker after we have finished; it cannot be lost.
            if self
                .state
                .compare_exchange(RUNNING, IDLE, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::*;

    #[test]
    fn requests_before_worker_starts_share_one_sweep() {
        let scheduler = EvictionScheduler::new();
        assert!(scheduler.request());
        for _ in 0..32 {
            assert!(!scheduler.request());
        }
        let mut sweeps = 0;
        scheduler.run(|| sweeps += 1);
        assert_eq!(sweeps, 1);

        assert!(
            scheduler.request(),
            "the drained worker must release ownership"
        );
        scheduler.run(|| sweeps += 1);
        assert_eq!(sweeps, 2);
    }

    #[test]
    fn concurrent_requests_during_sweep_share_one_follow_up() {
        let scheduler = EvictionScheduler::new();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        assert!(scheduler.request());

        thread::scope(|scope| {
            let worker_scheduler = &scheduler;
            let worker = scope.spawn(move || {
                let mut sweeps = 0;
                worker_scheduler.run(|| {
                    sweeps += 1;
                    if sweeps == 1 {
                        started_tx.send(()).unwrap();
                        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                    }
                });
                sweeps
            });
            started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            let requesters: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| (0..32).filter(|_| scheduler.request()).count()))
                .collect();
            let extra_workers: usize = requesters
                .into_iter()
                .map(|requester| requester.join().unwrap())
                .sum();
            release_tx.send(()).unwrap();
            let sweeps = worker.join().unwrap();

            assert_eq!(extra_workers, 0, "a sweep must never start a second worker");
            assert_eq!(sweeps, 2, "a burst must produce just one follow-up sweep");
        });

        assert!(scheduler.request());
        scheduler.run(|| {});
    }
}

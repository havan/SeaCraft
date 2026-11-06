//! Order-preserving parallel map on scoped std threads (no thread pool).

/// Worker threads running now, across every `par_map` in the process.
static WORKERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A share of the process-wide budget of one worker per core, returned on drop
/// (a panic included). Concurrent calls split the budget instead of each starting
/// a thread per core, which multiplied under `cargo test` and two GUI jobs.
struct WorkerBudget {
    workers: usize,
}

impl WorkerBudget {
    fn claim(wanted: usize) -> Self {
        use std::sync::atomic::Ordering;
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        let wanted = wanted.min(cores);
        let mut busy = WORKERS.load(Ordering::SeqCst);
        let workers = loop {
            let workers = wanted.min(cores.saturating_sub(busy));
            match WORKERS.compare_exchange(busy, busy + workers, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => break workers,
                Err(now) => busy = now,
            }
        };
        Self { workers }
    }
}

impl Drop for WorkerBudget {
    fn drop(&mut self) {
        WORKERS.fetch_sub(self.workers, std::sync::atomic::Ordering::SeqCst);
    }
}

/// `items.iter().map(f).collect()` on the cores no other call is using, order kept.
/// Workers take one item at a time, so uneven costs still balance. Each item
/// is computed by exactly the same code as the serial map, so the results are
/// bit-for-bit those of `items.iter().map(f)`, however many workers it got.
///
/// A panic in `f` is re-raised on the calling thread with its original
/// payload, so a caller that catches it sees the real message.
pub(crate) fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let budget = WorkerBudget::claim(items.len());
    let threads = budget.workers;
    if threads <= 1 {
        drop(budget);
        return items.iter().map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let (next, f) = (&next, &f);
    let mut tagged: Vec<(usize, R)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(move || {
                    let mut out = Vec::new();
                    loop {
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(k) else { break };
                        out.push((k, f(item)));
                    }
                    out
                })
            })
            .collect();
        let mut all = Vec::with_capacity(items.len());
        let mut panic = None;
        for h in handles {
            match h.join() {
                Ok(part) => all.extend(part),
                Err(payload) => panic = panic.or(Some(payload)),
            }
        }
        if let Some(payload) = panic {
            std::panic::resume_unwind(payload);
        }
        all
    });
    drop(budget);
    tagged.sort_unstable_by_key(|(k, _)| *k);
    tagged.into_iter().map(|(_, r)| r).collect()
}

#[cfg(test)]
mod tests {
    use super::par_map;

    /// Concurrent calls share one budget: a caller that finds it spent runs
    /// serially instead of multiplying threads (tests running at once, two GUI jobs).
    #[test]
    fn concurrent_calls_share_the_core_budget() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        let callers = 4;
        let (now, peak) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let items: Vec<u32> = (0..40).collect();
        std::thread::scope(|s| {
            for _ in 0..callers {
                s.spawn(|| {
                    par_map(&items, |_| {
                        let running = now.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(running, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(5));
                        now.fetch_sub(1, Ordering::SeqCst);
                    });
                });
            }
        });
        let peak = peak.load(Ordering::SeqCst);
        assert!(peak <= cores + callers, "peak {peak} with {cores} cores");
    }

    #[test]
    fn keeps_the_order_of_the_input() {
        let items: Vec<u64> = (0..1000).collect();
        let out = par_map(&items, |&k| k * k + 1);
        let serial: Vec<u64> = items.iter().map(|&k| k * k + 1).collect();
        assert_eq!(out, serial);
    }

    #[test]
    fn a_panic_reaches_the_caller_with_its_message() {
        let items: Vec<u32> = (0..64).collect();
        let caught = std::panic::catch_unwind(|| {
            par_map(&items, |&k| {
                if k == 17 {
                    panic!("item seventeen failed");
                }
                k
            })
        })
        .expect_err("the panic propagates");
        let msg = caught
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| caught.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        assert!(msg.contains("item seventeen failed"), "{msg}");
    }
}

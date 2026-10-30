//! Order-preserving parallel map on scoped std threads (no thread pool).

/// `items.iter().map(f).collect()` on every available core, order kept.
/// Workers take one item at a time, so uneven costs still balance. Each item
/// is computed by exactly the same code as the serial map, so the results are
/// bit-for-bit those of `items.iter().map(f)`.
///
/// A panic in `f` is re-raised on the calling thread with its original
/// payload, so a caller that catches it sees the real message.
pub(crate) fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(items.len());
    if threads <= 1 {
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
    tagged.sort_unstable_by_key(|(k, _)| *k);
    tagged.into_iter().map(|(_, r)| r).collect()
}

#[cfg(test)]
mod tests {
    use super::par_map;

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

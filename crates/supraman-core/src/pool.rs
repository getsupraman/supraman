use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// `f` of each item, on up to `at_most` threads that each take the next item when done with one; the results in the
/// items' order. Blocking.
pub fn map<T: Send, R: Send>(items: Vec<T>, at_most: usize, f: impl Fn(T) -> R + Sync) -> Vec<R> {
    let total = items.len();
    let slots: Vec<Mutex<Option<T>>> = items.into_iter().map(|i| Mutex::new(Some(i))).collect();
    let next = AtomicUsize::new(0);
    let threads = at_most.clamp(1, total.max(1));
    let mut done: Vec<(usize, R)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let n = next.fetch_add(1, Ordering::Relaxed);
                        let Some(slot) = slots.get(n) else { break out };
                        let Some(item) = slot.lock().unwrap_or_else(|e| e.into_inner()).take() else { continue };
                        out.push((n, f(item)));
                    }
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))).collect()
    });
    done.sort_by_key(|(n, _)| *n);
    done.into_iter().map(|(_, r)| r).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Condvar;
    use std::time::Duration;

    #[test]
    fn at_most_n_run_at_once_and_the_results_keep_the_items_order() {
        let (running, more) = (Mutex::new(0usize), Condvar::new());
        let most = AtomicUsize::new(0);
        let out = map((0..12).collect(), 4, |n: u32| {
            let mut now = running.lock().unwrap();
            *now += 1;
            most.fetch_max(*now, Ordering::SeqCst);
            more.notify_all();
            let (mut now, _) = more.wait_timeout_while(now, Duration::from_millis(100), |now| *now <= 4).unwrap();
            *now -= 1;
            n * 2
        });
        assert_eq!(out, (0..12).map(|n| n * 2).collect::<Vec<_>>());
        assert_eq!(most.load(Ordering::SeqCst), 4, "each item waits for a fifth to run beside it");
        assert!(map(Vec::<u32>::new(), 4, |n| n).is_empty());
    }
}

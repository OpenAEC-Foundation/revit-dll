//! Work shared over the cores of the machine, with results in the order of the input.

/// `f` of every item, in order. The items are taken one by one by as many threads as the machine
/// has cores (at most 16), so items of very different cost do not leave a thread idle; a browser
/// build (no threads) and a short list run in place.
pub fn map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = if cfg!(target_arch = "wasm32") { 1 } else { std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16).min(items.len()) };
    if threads <= 1 {
        return items.iter().map(f).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let (next, f) = (&next, &f);
    let mut parts: Vec<(usize, R)> = std::thread::scope(|sc| {
        let jobs: Vec<_> = (0..threads)
            .map(|_| {
                sc.spawn(move || {
                    let mut done = Vec::new();
                    loop {
                        let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(item) = items.get(k) else { break };
                        done.push((k, f(item)));
                    }
                    done
                })
            })
            .collect();
        jobs.into_iter().flat_map(|j| j.join().expect("worker thread")).collect()
    });
    parts.sort_by_key(|(k, _)| *k);
    parts.into_iter().map(|(_, r)| r).collect()
}

/// `a` and `b` beside each other: `a` on a thread of its own, `b` here. A browser build (no
/// threads) runs them one after the other.
pub fn both<A: Send, B>(a: impl FnOnce() -> A + Send, b: impl FnOnce() -> B) -> (A, B) {
    if cfg!(target_arch = "wasm32") {
        return (a(), b());
    }
    std::thread::scope(|sc| {
        let a = sc.spawn(a);
        let b = b();
        (a.join().expect("worker thread"), b)
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn both_give_their_results() {
        assert_eq!(super::both(|| 1, || "two"), (1, "two"));
    }

    #[test]
    fn results_keep_the_order_of_the_items() {
        let items: Vec<u32> = (0..1000).collect();
        assert_eq!(super::map(&items, |v| v * 2), items.iter().map(|v| v * 2).collect::<Vec<_>>());
        assert!(super::map(&[] as &[u32], |v| *v).is_empty());
    }
}

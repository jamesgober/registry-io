//! Concurrent / multi-threaded tests for `SyncRegistry`.
//!
//! These exercise the lock-free read path under contention as well as
//! interleaved register/unregister against active readers.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use registry_io::SyncRegistry;

#[test]
fn many_threads_can_notify_simultaneously() {
    let registry = Arc::new(SyncRegistry::<u64>::new());
    let count = Arc::new(AtomicUsize::new(0));

    for _ in 0..4 {
        let c = Arc::clone(&count);
        let _ = registry.register(move |_| {
            let _ = c.fetch_add(1, Ordering::Relaxed);
        });
    }

    let threads = 16usize;
    let per_thread = 1000usize;
    let mut handles = Vec::new();
    for _ in 0..threads {
        let r = Arc::clone(&registry);
        handles.push(thread::spawn(move || {
            for _ in 0..per_thread {
                r.notify(&1);
            }
        }));
    }
    for h in handles {
        h.join().expect("worker did not panic");
    }

    assert_eq!(count.load(Ordering::Relaxed), threads * per_thread * 4);
}

#[test]
fn register_during_active_notify_does_not_corrupt_state() {
    let registry = Arc::new(SyncRegistry::<u32>::new());
    let baseline = Arc::new(AtomicUsize::new(0));

    for _ in 0..4 {
        let b = Arc::clone(&baseline);
        let _ = registry.register(move |_| {
            let _ = b.fetch_add(1, Ordering::Relaxed);
        });
    }

    let notifier_registry = Arc::clone(&registry);
    let notifier = thread::spawn(move || {
        for _ in 0..2000 {
            notifier_registry.notify(&1);
        }
    });

    let mutator_registry = Arc::clone(&registry);
    let mutator = thread::spawn(move || {
        let mut new_ids = Vec::new();
        for _ in 0..100 {
            new_ids.push(mutator_registry.register(|_| {}));
        }
        for id in new_ids {
            let _ = mutator_registry.unregister(id);
        }
    });

    notifier.join().expect("notifier ok");
    mutator.join().expect("mutator ok");

    // We don't assert an exact count because the number of handlers active
    // during each notify varies. We do assert state is consistent.
    assert_eq!(registry.handler_count(), 4);
}

#[test]
fn concurrent_register_produces_unique_ids() {
    let registry = Arc::new(SyncRegistry::<()>::new());
    let mut handles = Vec::new();
    for _ in 0..8 {
        let r = Arc::clone(&registry);
        handles.push(thread::spawn(move || {
            let mut local = Vec::with_capacity(500);
            for _ in 0..500 {
                local.push(r.register(|_| {}));
            }
            local
        }));
    }
    let mut all = Vec::new();
    for h in handles {
        let mut ids = h.join().expect("worker ok");
        all.append(&mut ids);
    }
    let unique: std::collections::HashSet<_> = all.iter().copied().collect();
    assert_eq!(unique.len(), all.len());
    assert_eq!(registry.handler_count(), 8 * 500);
}

#[test]
fn concurrent_unregister_is_idempotent_per_id() {
    let registry = Arc::new(SyncRegistry::<()>::new());
    let id = registry.register(|_| {});

    let mut threads = Vec::new();
    let success = Arc::new(AtomicUsize::new(0));
    for _ in 0..8 {
        let r = Arc::clone(&registry);
        let s = Arc::clone(&success);
        threads.push(thread::spawn(move || {
            if r.unregister(id) {
                let _ = s.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    for t in threads {
        t.join().expect("worker ok");
    }

    // Exactly one thread should have observed the successful removal.
    assert_eq!(success.load(Ordering::Relaxed), 1);
    assert_eq!(registry.handler_count(), 0);
}

#[test]
fn stress_register_unregister_cycle_keeps_count_stable() {
    let registry = Arc::new(SyncRegistry::<u32>::new());

    let mut handles = Vec::new();
    for _ in 0..4 {
        let r = Arc::clone(&registry);
        handles.push(thread::spawn(move || {
            for _ in 0..2_500 {
                let id = r.register(|_| {});
                assert!(r.unregister(id));
            }
        }));
    }
    for h in handles {
        h.join().expect("stress worker ok");
    }

    assert_eq!(registry.handler_count(), 0);
}

#[test]
fn notify_observes_consistent_snapshot_even_if_handlers_change() {
    use std::sync::Barrier;

    // While a notify is iterating, a concurrent clear must not retroactively
    // remove handlers from the in-flight notify's snapshot.
    let registry = Arc::new(SyncRegistry::<()>::new());
    let invocations = Arc::new(AtomicUsize::new(0));

    // Barrier rendezvous between the first handler and the clearing thread.
    let started = Arc::new(Barrier::new(2));

    // Handler 0: signal that notify is mid-iteration, then yield long
    // enough for the clear to land.
    let inv = Arc::clone(&invocations);
    let signal = Arc::clone(&started);
    let _ = registry.register(move |_| {
        signal.wait();
        // Give the clearing thread time to execute its store.
        for _ in 0..10_000 {
            std::hint::spin_loop();
        }
        let _ = inv.fetch_add(1, Ordering::Relaxed);
    });

    // Handlers 1..8: plain counters.
    for _ in 1..8 {
        let inv = Arc::clone(&invocations);
        let _ = registry.register(move |_| {
            let _ = inv.fetch_add(1, Ordering::Relaxed);
        });
    }

    let notifier_registry = Arc::clone(&registry);
    let notifier = thread::spawn(move || {
        notifier_registry.notify(&());
    });

    started.wait();
    registry.clear();

    notifier.join().expect("notifier ok");

    // Every handler in the original snapshot must have run, despite the
    // mid-flight clear.
    assert_eq!(invocations.load(Ordering::Relaxed), 8);
    assert_eq!(registry.handler_count(), 0);
}

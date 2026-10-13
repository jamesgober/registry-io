//! `unregister_and_wait`, `clear_and_wait`, `register_once`, and
//! `panic_count`.

#![cfg(feature = "sync")]
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Barrier, Once, OnceLock};
use std::thread;
use std::time::Duration;

use registry_io::{HandlerId, SyncRegistry};

/// Handler panics are intentional in several tests; keep the default hook
/// from printing them.
fn quiet_panics() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| std::panic::set_hook(Box::new(|_| {})));
}

/// Registers a handler that signals when it starts and then blocks until
/// released. Returns its id, the "entered" receiver, and the release flag.
fn blocking_handler(
    registry: &SyncRegistry<()>,
) -> (HandlerId, mpsc::Receiver<()>, Arc<AtomicBool>) {
    let (entered_tx, entered_rx) = mpsc::channel();
    let release = Arc::new(AtomicBool::new(false));
    let r = Arc::clone(&release);
    let entered_tx = std::sync::Mutex::new(entered_tx);
    let id = registry.register(move |_| {
        let _ = entered_tx.lock().unwrap().send(());
        while !r.load(Ordering::SeqCst) {
            thread::yield_now();
        }
    });
    (id, entered_rx, release)
}

#[test]
fn unregister_and_wait_blocks_until_in_flight_handler_finishes() {
    let registry = Arc::new(SyncRegistry::<()>::new());
    let (id, entered, release) = blocking_handler(&registry);

    let notifier = {
        let r = Arc::clone(&registry);
        thread::spawn(move || r.notify(&()))
    };
    entered.recv().unwrap();

    let waiter = {
        let r = Arc::clone(&registry);
        thread::spawn(move || r.unregister_and_wait(id))
    };
    thread::sleep(Duration::from_millis(30));
    assert!(!waiter.is_finished(), "must wait while the handler runs");
    assert!(!registry.contains(id), "removed before waiting");

    release.store(true, Ordering::SeqCst);
    assert!(waiter.join().unwrap());
    notifier.join().unwrap();
}

#[test]
fn unregister_and_wait_unknown_id_returns_false_immediately() {
    let registry: SyncRegistry<()> = SyncRegistry::new();
    let id = registry.register(|_| {});
    assert!(registry.unregister(id));
    assert!(!registry.unregister_and_wait(id));
}

#[test]
fn unregister_and_wait_from_inside_own_handler_does_not_deadlock() {
    let registry = Arc::new(SyncRegistry::<()>::new());
    let own_id: Arc<OnceLock<HandlerId>> = Arc::new(OnceLock::new());
    let runs = Arc::new(AtomicU32::new(0));
    let (r, me, n) = (
        Arc::downgrade(&registry),
        Arc::clone(&own_id),
        Arc::clone(&runs),
    );
    let id = registry.register(move |_| {
        let _ = n.fetch_add(1, Ordering::SeqCst);
        if let (Some(registry), Some(id)) = (r.upgrade(), me.get()) {
            // Removes itself; must not wait for its own running call.
            assert!(registry.unregister_and_wait(*id));
        }
    });
    own_id.set(id).unwrap();
    registry.notify(&());
    registry.notify(&());
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert!(registry.is_empty());
}

#[test]
fn clear_and_wait_blocks_until_in_flight_handler_finishes() {
    let registry = Arc::new(SyncRegistry::<()>::new());
    let (_id, entered, release) = blocking_handler(&registry);
    let notifier = {
        let r = Arc::clone(&registry);
        thread::spawn(move || r.notify(&()))
    };
    entered.recv().unwrap();
    let waiter = {
        let r = Arc::clone(&registry);
        thread::spawn(move || r.clear_and_wait())
    };
    thread::sleep(Duration::from_millis(30));
    assert!(!waiter.is_finished());
    release.store(true, Ordering::SeqCst);
    waiter.join().unwrap();
    notifier.join().unwrap();
    assert!(registry.is_empty());
}

#[test]
fn clear_and_wait_on_empty_registry_returns() {
    let registry: SyncRegistry<()> = SyncRegistry::new();
    registry.clear_and_wait();
}

#[test]
fn register_once_runs_exactly_once_under_concurrent_notifies() {
    let registry = Arc::new(SyncRegistry::<()>::new());
    let runs = Arc::new(AtomicU32::new(0));
    let n = Arc::clone(&runs);
    let id = registry.register_once(move |_| {
        let _ = n.fetch_add(1, Ordering::SeqCst);
    });

    let start = Arc::new(Barrier::new(16));
    let handles: Vec<_> = (0..16)
        .map(|_| {
            let (r, b) = (Arc::clone(&registry), Arc::clone(&start));
            thread::spawn(move || {
                let _ = b.wait();
                for _ in 0..100 {
                    r.notify(&());
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert!(!registry.contains(id));
}

#[test]
fn register_once_accepts_fn_once_and_moves_state_out() {
    let registry = Arc::new(SyncRegistry::<u32>::new());
    let (tx, rx) = mpsc::channel::<String>();
    let owned = String::from("moved into the handler");
    let _ = registry.register_once(move |n| {
        tx.send(format!("{owned} {n}")).unwrap();
    });
    registry.notify(&7);
    assert_eq!(rx.recv().unwrap(), "moved into the handler 7");
    // The handler (and its `tx`) is gone, so the channel closes.
    assert!(rx.recv().is_err());
}

#[test]
fn register_once_is_removed_even_when_it_panics() {
    quiet_panics();
    let registry = Arc::new(SyncRegistry::<()>::new());
    let id = registry.register_once(|_| panic!("once"));
    registry.notify(&());
    assert!(!registry.contains(id));
    assert_eq!(registry.panic_count(), 1);
    registry.notify(&());
    assert_eq!(registry.panic_count(), 1);
}

#[test]
fn register_once_does_not_keep_registry_alive() {
    let canary = Arc::new(());
    let registry = Arc::new(SyncRegistry::<()>::new());
    let c = Arc::clone(&canary);
    let _ = registry.register_once(move |_| drop(c));
    assert_eq!(Arc::strong_count(&canary), 2);
    drop(registry);
    assert_eq!(
        Arc::strong_count(&canary),
        1,
        "handler freed with the registry"
    );
}

#[test]
fn panic_count_counts_with_and_without_callback() {
    quiet_panics();
    let registry: SyncRegistry<()> = SyncRegistry::new();
    let _ = registry.register(|_| panic!("a"));
    let _ = registry.register(|_| {});
    registry.notify(&());
    assert_eq!(registry.panic_count(), 1);
    registry.on_panic(|_| {});
    registry.notify(&());
    assert_eq!(registry.panic_count(), 2);
}

/// Stress test for the `unregister_and_wait` guarantee.
///
/// Notifier threads dispatch continuously. Churn threads repeatedly
/// register a handler guarding a "resource", unregister it with
/// `unregister_and_wait`, and then close the resource. A handler that
/// runs after its resource is closed panics, and the registry counts it.
/// The guarantee holds only if that count stays at zero.
///
/// Runs for 1.5 s by default; set `REGISTRY_IO_SOAK_SECS` for a longer
/// soak.
#[test]
fn soak_unregister_and_wait_never_runs_handler_after_return() {
    quiet_panics();
    let secs: f64 = std::env::var("REGISTRY_IO_SOAK_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.5);
    let deadline = std::time::Instant::now() + Duration::from_secs_f64(secs);
    let registry = Arc::new(SyncRegistry::<u64>::new());
    let stop = Arc::new(AtomicBool::new(false));
    let dispatched = Arc::new(AtomicU64::new(0));

    let notifiers: Vec<_> = (0..6)
        .map(|_| {
            let (r, s, d) = (
                Arc::clone(&registry),
                Arc::clone(&stop),
                Arc::clone(&dispatched),
            );
            thread::spawn(move || {
                while !s.load(Ordering::Relaxed) {
                    r.notify(&1);
                    let _ = d.fetch_add(1, Ordering::Relaxed);
                }
            })
        })
        .collect();

    let churners: Vec<_> = (0..4)
        .map(|_| {
            let r = Arc::clone(&registry);
            thread::spawn(move || {
                let mut cycles = 0u64;
                while std::time::Instant::now() < deadline {
                    let closed = Arc::new(AtomicBool::new(false));
                    let c = Arc::clone(&closed);
                    let id = r.register(move |_| {
                        assert!(
                            !c.load(Ordering::SeqCst),
                            "handler ran after unregister_and_wait"
                        );
                    });
                    thread::yield_now();
                    assert!(r.unregister_and_wait(id));
                    closed.store(true, Ordering::SeqCst);
                    cycles += 1;
                }
                cycles
            })
        })
        .collect();

    let cycles: u64 = churners.into_iter().map(|h| h.join().unwrap()).sum();
    stop.store(true, Ordering::Relaxed);
    for h in notifiers {
        h.join().unwrap();
    }
    assert!(cycles > 100, "churn made progress: {cycles}");
    assert!(dispatched.load(Ordering::Relaxed) > 1000);
    assert_eq!(
        registry.panic_count(),
        0,
        "a handler ran after its unregister_and_wait returned"
    );
    assert!(registry.is_empty());
}

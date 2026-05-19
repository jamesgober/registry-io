//! Memory-leak verification via `Arc::strong_count` canary.
//!
//! The registry stores handler closures as
//! `Arc<dyn Fn(&E) + Send + Sync + 'static>`. The concern this test guards
//! against is: after a register / unregister cycle, the closure (and
//! anything it captured) must be dropped. If the registry forgot to drop
//! a handler — for instance by stashing it in some internal cache that
//! `unregister` doesn't touch — that captured state would leak.
//!
//! We exercise this by capturing a strong reference to a "canary" `Arc`
//! inside each registered handler, then driving 10 000 register /
//! unregister cycles. After the loop, the canary's strong count must be
//! within a small constant of its baseline. If even one closure leaked
//! per cycle we would see a strong count in the thousands.
//!
//! A small per-thread slack (`<= 4`) is permitted because `arc-swap`
//! retains a thread-local cache of the last-seen `Arc<Vec<...>>` to
//! make `load()` cheaper. That cached snapshot is reclaimed on the next
//! load that observes a different generation, but the test thread can
//! end its iteration with one snapshot still cached. The canary count
//! therefore bounds total leakage at "essentially constant", not "0".

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use registry_io::SyncRegistry;

#[test]
fn register_unregister_churn_does_not_leak_handler_closures() {
    let registry: SyncRegistry<()> = SyncRegistry::new();
    let canary: Arc<u64> = Arc::new(0xDEAD_BEEF);
    assert_eq!(Arc::strong_count(&canary), 1, "baseline canary count");

    const ITERATIONS: usize = 10_000;
    for _ in 0..ITERATIONS {
        let captured = Arc::clone(&canary);
        let id = registry.register(move |_| {
            // Keep the capture in the closure body so the compiler doesn't
            // elide it.
            let _ = &*captured;
        });
        assert!(
            registry.unregister(id),
            "unregister must find the just-registered id"
        );
    }

    // One final no-op operation to give arc-swap's TLS cache a chance to
    // observe the latest empty-vec generation. Without this, the test
    // thread may still hold one stale snapshot from inside the loop.
    let _ = registry.handler_count();

    let final_count = Arc::strong_count(&canary);
    assert!(
        final_count <= 4,
        "leaked handler closures: canary strong_count = {final_count} after {ITERATIONS} cycles"
    );
}

#[test]
fn clear_drops_all_handler_closures() {
    let registry: SyncRegistry<()> = SyncRegistry::new();
    let canary: Arc<u64> = Arc::new(0);

    for _ in 0..100 {
        let captured = Arc::clone(&canary);
        let _ = registry.register(move |_| {
            let _ = &*captured;
        });
    }
    // 100 captured Arcs + 1 outside-the-registry reference = at least 101.
    assert!(Arc::strong_count(&canary) >= 101);

    registry.clear();
    let _ = registry.handler_count(); // flush TLS cache

    let final_count = Arc::strong_count(&canary);
    assert!(
        final_count <= 4,
        "clear() failed to drop all handler closures: canary strong_count = {final_count}"
    );
}

#[test]
fn dropping_registry_releases_all_handler_closures() {
    let canary: Arc<u64> = Arc::new(0);
    {
        let registry: SyncRegistry<()> = SyncRegistry::new();
        for _ in 0..50 {
            let captured = Arc::clone(&canary);
            let _ = registry.register(move |_| {
                let _ = &*captured;
            });
        }
        assert!(Arc::strong_count(&canary) >= 51);
        // registry drops here.
    }

    let final_count = Arc::strong_count(&canary);
    assert_eq!(
        final_count, 1,
        "dropping the registry must release every handler closure (canary = {final_count})"
    );
}

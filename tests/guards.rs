//! RAII handler guard tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use registry_io::SyncRegistry;

#[test]
fn guard_unregisters_on_drop() {
    let registry = Arc::new(SyncRegistry::<u32>::new());
    assert_eq!(registry.handler_count(), 0);

    {
        let _guard = registry.register_guard(|_| {});
        assert_eq!(registry.handler_count(), 1);
    }

    assert_eq!(registry.handler_count(), 0);
}

#[test]
fn guard_with_priority_observes_priority_ordering() {
    use std::sync::Mutex;
    let registry = Arc::new(SyncRegistry::<()>::new());
    let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));

    let o = Arc::clone(&order);
    let _g1 = registry.register_guard_with_priority(0, move |_| o.lock().unwrap().push("mid"));
    let o = Arc::clone(&order);
    let _g2 = registry.register_guard_with_priority(100, move |_| o.lock().unwrap().push("first"));

    registry.notify(&());
    assert_eq!(order.lock().unwrap().as_slice(), &["first", "mid"]);
}

#[test]
fn guard_id_matches_registration() {
    let registry = Arc::new(SyncRegistry::<()>::new());
    let guard = registry.register_guard(|_| {});
    let id = guard.id();
    assert!(registry.contains(id));
    drop(guard);
    assert!(!registry.contains(id));
}

#[test]
fn forget_keeps_handler_registered() {
    let registry = Arc::new(SyncRegistry::<()>::new());
    let guard = registry.register_guard(|_| {});
    let id = guard.id();
    guard.forget();
    assert!(registry.contains(id));
    assert_eq!(registry.handler_count(), 1);

    assert!(registry.unregister(id));
    assert_eq!(registry.handler_count(), 0);
}

#[test]
fn dropping_registry_before_guard_is_safe() {
    let registry = Arc::new(SyncRegistry::<()>::new());
    let guard = registry.register_guard(|_| {});

    // Drop the strong ref. Guard now holds a stale weak ref.
    drop(registry);

    // Drop the guard. Its Drop should observe upgrade()=None and no-op.
    drop(guard);
}

#[test]
fn multiple_guards_unregister_independently() {
    let registry = Arc::new(SyncRegistry::<()>::new());
    let g1 = registry.register_guard(|_| {});
    let g2 = registry.register_guard(|_| {});
    let g3 = registry.register_guard(|_| {});

    assert_eq!(registry.handler_count(), 3);

    drop(g2);
    assert_eq!(registry.handler_count(), 2);

    drop(g1);
    assert_eq!(registry.handler_count(), 1);

    drop(g3);
    assert_eq!(registry.handler_count(), 0);
}

#[test]
fn guard_drop_does_not_fire_handler() {
    let registry = Arc::new(SyncRegistry::<()>::new());
    let count = Arc::new(AtomicUsize::new(0));
    let sink = Arc::clone(&count);
    let guard = registry.register_guard(move |_| {
        let _ = sink.fetch_add(1, Ordering::Relaxed);
    });

    drop(guard);
    registry.notify(&());
    assert_eq!(count.load(Ordering::Relaxed), 0);
}

#[test]
fn guard_debug_includes_id_and_alive_status() {
    let registry = Arc::new(SyncRegistry::<()>::new());
    let guard = registry.register_guard(|_| {});
    let s = format!("{guard:?}");
    assert!(s.contains("HandlerGuard"));
    assert!(s.contains("id"));
    assert!(s.contains("registry_alive"));
}

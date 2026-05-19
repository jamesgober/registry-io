//! `AsyncHandlerGuard` RAII tests.

#![cfg(feature = "async")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use registry_io::r#async::AsyncRegistry;

#[tokio::test]
async fn guard_unregisters_on_drop() {
    let registry = Arc::new(AsyncRegistry::<u32>::new());
    {
        let _guard = registry.register_guard(|_| async move {});
        assert_eq!(registry.handler_count(), 1);
    }
    assert_eq!(registry.handler_count(), 0);
}

#[tokio::test]
async fn guard_id_round_trip() {
    let registry = Arc::new(AsyncRegistry::<()>::new());
    let guard = registry.register_guard(|_| async move {});
    let id = guard.id();
    assert!(registry.contains(id));
    drop(guard);
    assert!(!registry.contains(id));
}

#[tokio::test]
async fn forget_keeps_handler_registered() {
    let registry = Arc::new(AsyncRegistry::<()>::new());
    let guard = registry.register_guard(|_| async move {});
    let id = guard.id();
    guard.forget();
    assert!(registry.contains(id));
    assert!(registry.unregister(id));
}

#[tokio::test]
async fn dropping_registry_before_guard_is_safe() {
    let registry = Arc::new(AsyncRegistry::<()>::new());
    let guard = registry.register_guard(|_| async move {});
    drop(registry);
    drop(guard);
}

#[tokio::test]
async fn guard_drop_prevents_handler_from_firing_next_time() {
    let registry = Arc::new(AsyncRegistry::<()>::new());
    let count = Arc::new(AtomicUsize::new(0));
    let sink = Arc::clone(&count);
    let guard = registry.register_guard(move |_| {
        let sink = Arc::clone(&sink);
        async move {
            let _ = sink.fetch_add(1, Ordering::Relaxed);
        }
    });

    drop(guard);
    registry.notify(&()).await;
    assert_eq!(count.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn multiple_guards_unregister_independently() {
    let registry = Arc::new(AsyncRegistry::<()>::new());
    let g1 = registry.register_guard(|_| async move {});
    let g2 = registry.register_guard(|_| async move {});
    let g3 = registry.register_guard(|_| async move {});

    assert_eq!(registry.handler_count(), 3);
    drop(g2);
    assert_eq!(registry.handler_count(), 2);
    drop(g1);
    assert_eq!(registry.handler_count(), 1);
    drop(g3);
    assert_eq!(registry.handler_count(), 0);
}

#[tokio::test]
async fn priority_guard_respects_priority_with_sequential_notify() {
    use std::sync::Mutex;
    let registry = Arc::new(AsyncRegistry::<()>::new());
    let log: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));

    let l = Arc::clone(&log);
    let _g1 = registry.register_guard_with_priority(0, move |_| {
        let l = Arc::clone(&l);
        async move {
            l.lock().unwrap().push("mid");
        }
    });
    let l = Arc::clone(&log);
    let _g2 = registry.register_guard_with_priority(100, move |_| {
        let l = Arc::clone(&l);
        async move {
            l.lock().unwrap().push("first");
        }
    });

    registry.notify_sequential(&()).await;
    assert_eq!(log.lock().unwrap().as_slice(), &["first", "mid"]);
}

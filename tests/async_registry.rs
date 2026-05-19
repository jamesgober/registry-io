//! Integration tests for `AsyncRegistry`.

#![cfg(feature = "async")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use registry_io::r#async::AsyncRegistry;

#[tokio::test]
async fn new_registry_is_empty() {
    let registry: AsyncRegistry<u32> = AsyncRegistry::new();
    assert!(registry.is_empty());
    assert_eq!(registry.handler_count(), 0);
}

#[tokio::test]
async fn default_matches_new() {
    let a: AsyncRegistry<u32> = AsyncRegistry::default();
    let b: AsyncRegistry<u32> = AsyncRegistry::new();
    assert_eq!(a.handler_count(), b.handler_count());
}

#[tokio::test]
async fn concurrent_notify_fires_every_handler_once() {
    let registry: AsyncRegistry<u32> = AsyncRegistry::new();
    let count = Arc::new(AtomicUsize::new(0));
    for _ in 0..10 {
        let sink = Arc::clone(&count);
        let _ = registry.register(move |_| {
            let sink = Arc::clone(&sink);
            async move {
                let _ = sink.fetch_add(1, Ordering::Relaxed);
            }
        });
    }
    registry.notify(&42).await;
    assert_eq!(count.load(Ordering::Relaxed), 10);

    registry.notify(&42).await;
    assert_eq!(count.load(Ordering::Relaxed), 20);
}

#[tokio::test]
async fn sequential_notify_fires_every_handler_once() {
    let registry: AsyncRegistry<u32> = AsyncRegistry::new();
    let count = Arc::new(AtomicUsize::new(0));
    for _ in 0..5 {
        let sink = Arc::clone(&count);
        let _ = registry.register(move |_| {
            let sink = Arc::clone(&sink);
            async move {
                let _ = sink.fetch_add(1, Ordering::Relaxed);
            }
        });
    }
    registry.notify_sequential(&0).await;
    assert_eq!(count.load(Ordering::Relaxed), 5);
}

#[tokio::test]
async fn unregister_removes_handler() {
    let registry: AsyncRegistry<()> = AsyncRegistry::new();
    let count = Arc::new(AtomicUsize::new(0));
    let sink = Arc::clone(&count);
    let id = registry.register(move |_| {
        let sink = Arc::clone(&sink);
        async move {
            let _ = sink.fetch_add(1, Ordering::Relaxed);
        }
    });

    assert!(registry.contains(id));
    assert!(registry.unregister(id));
    assert!(!registry.contains(id));
    assert!(!registry.unregister(id));

    registry.notify(&()).await;
    assert_eq!(count.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn clear_removes_every_handler() {
    let registry: AsyncRegistry<()> = AsyncRegistry::new();
    for _ in 0..8 {
        let _ = registry.register(|_| async move {});
    }
    assert_eq!(registry.handler_count(), 8);
    registry.clear();
    assert_eq!(registry.handler_count(), 0);
}

#[tokio::test]
async fn notify_passes_event_by_reference() {
    let registry: AsyncRegistry<String> = AsyncRegistry::new();
    let captured: Arc<std::sync::Mutex<String>> = Arc::new(std::sync::Mutex::new(String::new()));
    let sink = Arc::clone(&captured);
    let _ = registry.register(move |s: &String| {
        let sink = Arc::clone(&sink);
        let owned = s.clone();
        async move {
            sink.lock().unwrap().clone_from(&owned);
        }
    });

    let event = String::from("hello");
    registry.notify(&event).await;
    // Caller still owns the event.
    assert_eq!(event.as_str(), "hello");
    assert_eq!(captured.lock().unwrap().as_str(), "hello");
}

#[tokio::test]
async fn handler_count_after_mixed_operations() {
    let registry: AsyncRegistry<()> = AsyncRegistry::new();
    let mut ids = Vec::new();
    for _ in 0..50 {
        ids.push(registry.register(|_| async move {}));
    }
    for id in ids.iter().take(20) {
        assert!(registry.unregister(*id));
    }
    assert_eq!(registry.handler_count(), 30);
}

#[tokio::test]
async fn registry_is_send_and_sync() {
    fn require_send_sync<T: Send + Sync>() {}
    require_send_sync::<AsyncRegistry<u32>>();
    require_send_sync::<AsyncRegistry<String>>();
}

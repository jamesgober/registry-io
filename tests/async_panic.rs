//! Panic isolation for `AsyncRegistry`.
//!
//! Same guarantees as the sync side: a panic in one handler's future must
//! not stop sibling handlers nor unwind into the caller's `await`. The
//! `on_panic` callback observes per-handler failures.

#![cfg(feature = "async")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::panic::panic_any;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use registry_io::r#async::AsyncRegistry;

#[tokio::test]
async fn panic_in_handler_does_not_propagate_to_caller_concurrent() {
    let registry: AsyncRegistry<()> = AsyncRegistry::new();
    let _ = registry.register(|_| async move { panic!("boom") });
    // Must not unwind out of notify().await.
    registry.notify(&()).await;
}

#[tokio::test]
async fn panic_in_handler_does_not_propagate_to_caller_sequential() {
    let registry: AsyncRegistry<()> = AsyncRegistry::new();
    let _ = registry.register(|_| async move { panic!("boom") });
    registry.notify_sequential(&()).await;
}

#[tokio::test]
async fn panicking_handler_does_not_break_siblings_concurrent() {
    let registry: AsyncRegistry<()> = AsyncRegistry::new();
    let count = Arc::new(AtomicUsize::new(0));

    let c = Arc::clone(&count);
    let _ = registry.register(move |_| {
        let c = Arc::clone(&c);
        async move {
            let _ = c.fetch_add(1, Ordering::Relaxed);
        }
    });
    let _ = registry.register(|_| async move { panic!("middle dies") });
    let c = Arc::clone(&count);
    let _ = registry.register(move |_| {
        let c = Arc::clone(&c);
        async move {
            let _ = c.fetch_add(10, Ordering::Relaxed);
        }
    });

    registry.notify(&()).await;
    assert_eq!(count.load(Ordering::Relaxed), 11);
}

#[tokio::test]
async fn panicking_handler_does_not_break_siblings_sequential() {
    let registry: AsyncRegistry<()> = AsyncRegistry::new();
    let count = Arc::new(AtomicUsize::new(0));

    let c = Arc::clone(&count);
    let _ = registry.register(move |_| {
        let c = Arc::clone(&c);
        async move {
            let _ = c.fetch_add(1, Ordering::Relaxed);
        }
    });
    let _ = registry.register(|_| async move { panic!("middle dies") });
    let c = Arc::clone(&count);
    let _ = registry.register(move |_| {
        let c = Arc::clone(&c);
        async move {
            let _ = c.fetch_add(10, Ordering::Relaxed);
        }
    });

    registry.notify_sequential(&()).await;
    assert_eq!(count.load(Ordering::Relaxed), 11);
}

#[tokio::test]
async fn on_panic_observes_handler_id_and_message() {
    let registry: AsyncRegistry<()> = AsyncRegistry::new();
    let captured: Arc<Mutex<Vec<(u64, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    registry.on_panic(move |info| {
        sink.lock().unwrap().push((
            info.handler_id().as_u64(),
            info.message().unwrap_or("<opaque>").to_owned(),
        ));
    });

    let id = registry.register(|_| async move { panic!("first failure") });
    registry.notify_sequential(&()).await;

    let log = captured.lock().unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].0, id.as_u64());
    assert_eq!(log[0].1, "first failure");
}

#[tokio::test]
async fn on_panic_fires_once_per_panicking_handler_concurrent() {
    let registry: AsyncRegistry<()> = AsyncRegistry::new();
    let count = Arc::new(AtomicUsize::new(0));
    let sink = Arc::clone(&count);
    registry.on_panic(move |_| {
        let _ = sink.fetch_add(1, Ordering::Relaxed);
    });

    let _ = registry.register(|_| async move { panic!("a") });
    let _ = registry.register(|_| async move {});
    let _ = registry.register(|_| async move { panic!("b") });
    let _ = registry.register(|_| async move { panic!("c") });

    registry.notify(&()).await;
    assert_eq!(count.load(Ordering::Relaxed), 3);
}

#[tokio::test]
async fn custom_panic_payload_round_trips_through_payload() {
    #[derive(Debug, PartialEq, Eq)]
    struct MyErr(u32);

    let registry: AsyncRegistry<()> = AsyncRegistry::new();
    let captured: Arc<Mutex<Option<u32>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&captured);
    registry.on_panic(move |info| {
        if let Some(err) = info.payload().downcast_ref::<MyErr>() {
            *sink.lock().unwrap() = Some(err.0);
        }
    });

    let _ = registry.register(|_| async move { panic_any(MyErr(99)) });
    registry.notify_sequential(&()).await;

    assert_eq!(*captured.lock().unwrap(), Some(99));
}

#[tokio::test]
async fn no_callback_silently_absorbs_panics() {
    let registry: AsyncRegistry<()> = AsyncRegistry::new();
    for _ in 0..3 {
        let _ = registry.register(|_| async move { panic!("nobody listening") });
    }
    // Must not unwind, must not deadlock.
    registry.notify(&()).await;
}

#[tokio::test]
async fn panics_do_not_disturb_handler_count() {
    let registry: AsyncRegistry<()> = AsyncRegistry::new();
    let _ = registry.register(|_| async move { panic!("a") });
    let _ = registry.register(|_| async move { panic!("b") });
    let _ = registry.register(|_| async move {});

    assert_eq!(registry.handler_count(), 3);
    registry.notify(&()).await;
    assert_eq!(registry.handler_count(), 3);
}

#[tokio::test]
async fn clear_panic_callback_disables_observability() {
    let registry: AsyncRegistry<()> = AsyncRegistry::new();
    let count = Arc::new(AtomicUsize::new(0));

    let sink = Arc::clone(&count);
    registry.on_panic(move |_| {
        let _ = sink.fetch_add(1, Ordering::Relaxed);
    });
    let _ = registry.register(|_| async move { panic!("seen") });
    registry.notify(&()).await;
    assert_eq!(count.load(Ordering::Relaxed), 1);

    registry.clear_panic_callback();
    registry.notify(&()).await;
    assert_eq!(count.load(Ordering::Relaxed), 1);
}

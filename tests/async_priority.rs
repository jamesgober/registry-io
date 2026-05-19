//! Priority ordering for `AsyncRegistry`.
//!
//! Concurrent dispatch's *spawn* order matches priority; sequential
//! dispatch's *execution* order matches priority. Tests below cover both.

#![cfg(feature = "async")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use registry_io::r#async::AsyncRegistry;

#[tokio::test]
async fn sequential_notify_observes_priority_order() {
    let registry: AsyncRegistry<()> = AsyncRegistry::new();
    let log = Arc::new(Mutex::new(Vec::<i32>::new()));

    for p in [0_i32, 10, -5] {
        let l = Arc::clone(&log);
        let _ = registry.register_with_priority(p, move |_| {
            let l = Arc::clone(&l);
            async move {
                l.lock().unwrap().push(p);
            }
        });
    }

    registry.notify_sequential(&()).await;
    assert_eq!(log.lock().unwrap().as_slice(), &[10, 0, -5]);
}

#[tokio::test]
async fn sequential_equal_priority_fires_in_registration_order() {
    let registry: AsyncRegistry<()> = AsyncRegistry::new();
    let log = Arc::new(Mutex::new(Vec::<&'static str>::new()));

    for tag in ["first", "second", "third"] {
        let l = Arc::clone(&log);
        let _ = registry.register_with_priority(0, move |_| {
            let l = Arc::clone(&l);
            async move {
                l.lock().unwrap().push(tag);
            }
        });
    }

    registry.notify_sequential(&()).await;
    assert_eq!(
        log.lock().unwrap().as_slice(),
        &["first", "second", "third"]
    );
}

#[tokio::test]
async fn default_register_priority_is_zero_sequential() {
    let registry: AsyncRegistry<()> = AsyncRegistry::new();
    let log = Arc::new(Mutex::new(Vec::<&'static str>::new()));

    let l = Arc::clone(&log);
    let _ = registry.register_with_priority(5, move |_| {
        let l = Arc::clone(&l);
        async move {
            l.lock().unwrap().push("high");
        }
    });
    let l = Arc::clone(&log);
    let _ = registry.register(move |_| {
        let l = Arc::clone(&l);
        async move {
            l.lock().unwrap().push("default");
        }
    });
    let l = Arc::clone(&log);
    let _ = registry.register_with_priority(-5, move |_| {
        let l = Arc::clone(&l);
        async move {
            l.lock().unwrap().push("low");
        }
    });

    registry.notify_sequential(&()).await;
    assert_eq!(log.lock().unwrap().as_slice(), &["high", "default", "low"]);
}

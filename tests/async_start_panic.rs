//! Regression: a handler that panics while *building* its future (before
//! any `.await`) must be isolated like one that panics while being
//! polled. Before 1.1.0 this panic escaped `notify`.

#![cfg(feature = "async")]
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Once};

use registry_io::{AsyncRegistry, HandlerId};

fn quiet_panics() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let payload = info.payload();
            let message = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str));
            if !message.is_some_and(|m| m.starts_with("intentional")) {
                default(info);
            }
        }));
    });
}

/// A handler whose synchronous part panics.
fn panics_before_future(_: &u32) -> std::future::Ready<()> {
    panic!("intentional: panicked while building the future")
}

fn counting(total: &Arc<AtomicU32>) -> impl Fn(&u32) -> std::future::Ready<()> + use<> {
    let t = Arc::clone(total);
    move |n| {
        let _ = t.fetch_add(*n, Ordering::SeqCst);
        std::future::ready(())
    }
}

/// Registry, running total, panicking ids seen by `on_panic`, and the
/// id of the handler that panics.
type Setup = (
    AsyncRegistry<u32>,
    Arc<AtomicU32>,
    Arc<Mutex<Vec<HandlerId>>>,
    HandlerId,
);

fn setup() -> Setup {
    quiet_panics();
    let registry = AsyncRegistry::new();
    let total = Arc::new(AtomicU32::new(0));
    let _ = registry.register(counting(&total));
    let bad = registry.register(panics_before_future);
    let _ = registry.register(counting(&total));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = Arc::clone(&seen);
    registry.on_panic(move |info| s.lock().unwrap().push(info.handler_id()));
    (registry, total, seen, bad)
}

fn block_on<F: Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(f)
}

#[test]
fn concurrent_notify_isolates_panic_before_future() {
    let (registry, total, seen, bad) = setup();
    block_on(registry.notify(&5));
    assert_eq!(total.load(Ordering::SeqCst), 10, "siblings still ran");
    assert_eq!(seen.lock().unwrap().as_slice(), &[bad]);
}

#[test]
fn sequential_notify_isolates_panic_before_future() {
    let (registry, total, seen, bad) = setup();
    block_on(registry.notify_sequential(&5));
    assert_eq!(total.load(Ordering::SeqCst), 10, "siblings still ran");
    assert_eq!(seen.lock().unwrap().as_slice(), &[bad]);
}

#[test]
fn single_handler_panic_before_future_is_isolated() {
    quiet_panics();
    let registry: AsyncRegistry<u32> = AsyncRegistry::new();
    let bad = registry.register(panics_before_future);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = Arc::clone(&seen);
    registry.on_panic(move |info| s.lock().unwrap().push(info.handler_id()));
    block_on(registry.notify(&1));
    assert_eq!(seen.lock().unwrap().as_slice(), &[bad]);
}

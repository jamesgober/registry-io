//! Panic-isolation tests: a panicking handler must not interfere with
//! siblings, and the registry must offer observability via `on_panic`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::panic::panic_any;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use registry_io::SyncRegistry;

#[test]
fn panic_in_handler_does_not_propagate_to_caller() {
    let registry: SyncRegistry<()> = SyncRegistry::new();
    let _ = registry.register(|_| panic!("boom"));
    // Should not unwind out of notify.
    registry.notify(&());
}

#[test]
fn panicking_handler_does_not_break_siblings() {
    let registry: SyncRegistry<()> = SyncRegistry::new();
    let count = Arc::new(AtomicUsize::new(0));

    let c = Arc::clone(&count);
    let _ = registry.register(move |_| {
        let _ = c.fetch_add(1, Ordering::Relaxed);
    });
    let _ = registry.register(|_| panic!("middle handler dies"));
    let c = Arc::clone(&count);
    let _ = registry.register(move |_| {
        let _ = c.fetch_add(10, Ordering::Relaxed);
    });

    registry.notify(&());
    assert_eq!(count.load(Ordering::Relaxed), 11);
}

#[test]
fn on_panic_callback_receives_handler_id_and_message() {
    let registry: SyncRegistry<()> = SyncRegistry::new();
    let captured: Arc<Mutex<Vec<(u64, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    registry.on_panic(move |info| {
        let id = info.handler_id().as_u64();
        let msg = info.message().unwrap_or("<opaque>").to_owned();
        sink.lock().unwrap().push((id, msg));
    });

    let id = registry.register(|_| panic!("first failure"));
    registry.notify(&());

    let log = captured.lock().unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].0, id.as_u64());
    assert_eq!(log[0].1, "first failure");
}

#[test]
fn on_panic_fires_once_per_panicking_handler() {
    let registry: SyncRegistry<()> = SyncRegistry::new();
    let count = Arc::new(AtomicUsize::new(0));
    let sink = Arc::clone(&count);
    registry.on_panic(move |_| {
        let _ = sink.fetch_add(1, Ordering::Relaxed);
    });

    let _ = registry.register(|_| panic!("a"));
    let _ = registry.register(|_| {});
    let _ = registry.register(|_| panic!("b"));
    let _ = registry.register(|_| panic!("c"));

    registry.notify(&());
    assert_eq!(count.load(Ordering::Relaxed), 3);
}

#[test]
fn on_panic_can_be_replaced_and_cleared() {
    let registry: SyncRegistry<()> = SyncRegistry::new();
    let count = Arc::new(AtomicUsize::new(0));

    let sink = Arc::clone(&count);
    registry.on_panic(move |_| {
        let _ = sink.fetch_add(1, Ordering::Relaxed);
    });

    let _ = registry.register(|_| panic!("first"));
    registry.notify(&());
    assert_eq!(count.load(Ordering::Relaxed), 1);

    // Replace with a callback that counts by 100.
    let sink = Arc::clone(&count);
    registry.on_panic(move |_| {
        let _ = sink.fetch_add(100, Ordering::Relaxed);
    });
    registry.notify(&());
    assert_eq!(count.load(Ordering::Relaxed), 101);

    // Clear: future panics are absorbed silently.
    registry.clear_panic_callback();
    registry.notify(&());
    assert_eq!(count.load(Ordering::Relaxed), 101);
}

#[test]
fn panic_in_panic_callback_is_caught() {
    let registry: SyncRegistry<()> = SyncRegistry::new();
    let after = Arc::new(AtomicUsize::new(0));

    registry.on_panic(|_| panic!("callback also dies"));
    let _ = registry.register(|_| panic!("handler dies"));
    let s = Arc::clone(&after);
    let _ = registry.register(move |_| {
        let _ = s.fetch_add(1, Ordering::Relaxed);
    });

    // Notify must not unwind even though both handler and callback panic.
    registry.notify(&());
    assert_eq!(after.load(Ordering::Relaxed), 1);
}

#[test]
fn custom_panic_payload_round_trips_through_payload() {
    #[derive(Debug, PartialEq, Eq)]
    struct MyErr(u32);

    let registry: SyncRegistry<()> = SyncRegistry::new();
    let captured: Arc<Mutex<Option<u32>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&captured);
    registry.on_panic(move |info| {
        if let Some(err) = info.payload().downcast_ref::<MyErr>() {
            *sink.lock().unwrap() = Some(err.0);
        }
    });

    let _ = registry.register(|_| panic_any(MyErr(99)));
    registry.notify(&());

    assert_eq!(*captured.lock().unwrap(), Some(99));
}

#[test]
fn no_callback_silently_absorbs_panics() {
    let registry: SyncRegistry<()> = SyncRegistry::new();
    for _ in 0..5 {
        let _ = registry.register(|_| panic!("no observer"));
    }
    // Must not unwind, must not deadlock, must just return.
    registry.notify(&());
}

#[test]
fn panics_do_not_disturb_handler_count() {
    let registry: SyncRegistry<()> = SyncRegistry::new();
    let _ = registry.register(|_| panic!("a"));
    let _ = registry.register(|_| panic!("b"));
    let _ = registry.register(|_| {});

    assert_eq!(registry.handler_count(), 3);
    registry.notify(&());
    // No handler is auto-unregistered on panic.
    assert_eq!(registry.handler_count(), 3);
}

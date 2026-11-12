//! Panic-isolation tests: a panicking handler must not interfere with
//! siblings, and the registry must offer observability via `on_panic`.

#![cfg(feature = "sync")]
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

/// A panic payload whose `Drop` panics again.
struct DropBomb;

impl Drop for DropBomb {
    fn drop(&mut self) {
        panic!("intentional: payload drop");
    }
}

fn quiet_panics() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let payload = info.payload();
            let message = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str));
            let intentional =
                message.is_some_and(|m| m.starts_with("intentional")) || payload.is::<DropBomb>();
            if !intentional {
                default(info);
            }
        }));
    });
}

// Review F-3 (1.1.0): the caught payload was dropped outside any
// `catch_unwind`, so a payload whose `Drop` panics escaped `notify` and
// the handlers after it never ran.
#[test]
fn payload_with_panicking_drop_does_not_escape_notify() {
    quiet_panics();
    let registry: SyncRegistry<()> = SyncRegistry::new();
    let later = Arc::new(AtomicUsize::new(0));
    let _first = registry.register_with_priority(10, |_| panic_any(DropBomb));
    let sink = Arc::clone(&later);
    let _second = registry.register(move |_| {
        let _previous = sink.fetch_add(1, Ordering::SeqCst);
    });
    let escaped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        registry.notify(&());
    }))
    .is_err();
    assert!(!escaped, "the payload's Drop panic escaped notify");
    assert_eq!(
        later.load(Ordering::SeqCst),
        1,
        "the next handler was skipped"
    );
    assert_eq!(registry.panic_count(), 1);
}

// Review F-3 (1.1.0): a handler that unregisters itself is freed by the
// notifying thread on its way out of the dispatch; a panicking `Drop` in
// its captured state escaped `notify`.
#[test]
fn panicking_drop_of_a_self_unregistered_handler_does_not_escape_notify() {
    quiet_panics();
    let registry: Arc<SyncRegistry<()>> = Arc::new(SyncRegistry::new());
    let own_id: Arc<std::sync::OnceLock<registry_io::HandlerId>> =
        Arc::new(std::sync::OnceLock::new());
    let weak = Arc::downgrade(&registry);
    let cell = Arc::clone(&own_id);
    let bomb = DropBomb;
    let id = registry.register(move |_| {
        let _keep = &bomb;
        let _removed = weak.upgrade().unwrap().unregister(*cell.get().unwrap());
    });
    own_id.set(id).unwrap();
    let escaped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        registry.notify(&());
    }))
    .is_err();
    assert!(!escaped, "the capture's Drop panic escaped notify");
    assert!(registry.is_empty());
}

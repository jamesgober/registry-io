//! Integration tests for the core `SyncRegistry` API.
//!
//! These tests exercise the public surface without touching internals:
//! register / unregister / notify / clear / contains / handler_count.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use registry_io::SyncRegistry;

#[test]
fn new_registry_is_empty_and_default_matches_new() {
    let a: SyncRegistry<u32> = SyncRegistry::new();
    let b: SyncRegistry<u32> = SyncRegistry::default();
    assert!(a.is_empty());
    assert!(b.is_empty());
    assert_eq!(a.handler_count(), 0);
    assert_eq!(b.handler_count(), 0);
}

#[test]
fn with_capacity_does_not_change_observable_count() {
    let registry: SyncRegistry<u32> = SyncRegistry::with_capacity(64);
    assert_eq!(registry.handler_count(), 0);
    assert!(registry.is_empty());
}

#[test]
fn register_increments_count() {
    let registry: SyncRegistry<u32> = SyncRegistry::new();
    let _ = registry.register(|_| {});
    let _ = registry.register(|_| {});
    let _ = registry.register(|_| {});
    assert_eq!(registry.handler_count(), 3);
}

#[test]
fn register_returns_distinct_ids() {
    let registry: SyncRegistry<u32> = SyncRegistry::new();
    let mut ids = Vec::new();
    for _ in 0..100 {
        ids.push(registry.register(|_| {}));
    }
    let unique: std::collections::HashSet<_> = ids.iter().copied().collect();
    assert_eq!(unique.len(), ids.len());
}

#[test]
fn notify_invokes_every_handler_exactly_once() {
    let registry: SyncRegistry<u32> = SyncRegistry::new();
    let count = Arc::new(AtomicUsize::new(0));
    for _ in 0..10 {
        let c = Arc::clone(&count);
        let _ = registry.register(move |_| {
            let _ = c.fetch_add(1, Ordering::Relaxed);
        });
    }

    registry.notify(&42);
    assert_eq!(count.load(Ordering::Relaxed), 10);

    registry.notify(&42);
    assert_eq!(count.load(Ordering::Relaxed), 20);
}

#[test]
fn notify_with_no_handlers_is_noop() {
    let registry: SyncRegistry<u32> = SyncRegistry::new();
    registry.notify(&7);
    assert_eq!(registry.handler_count(), 0);
}

#[test]
fn unregister_removes_handler_and_returns_true() {
    let registry: SyncRegistry<u32> = SyncRegistry::new();
    let count = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&count);
    let id = registry.register(move |_| {
        let _ = c.fetch_add(1, Ordering::Relaxed);
    });
    assert!(registry.contains(id));

    assert!(registry.unregister(id));
    assert!(!registry.contains(id));

    registry.notify(&1);
    assert_eq!(count.load(Ordering::Relaxed), 0);
}

#[test]
fn unregister_of_unknown_id_returns_false() {
    let registry: SyncRegistry<u32> = SyncRegistry::new();
    let id = registry.register(|_| {});
    assert!(registry.unregister(id));
    assert!(!registry.unregister(id));
}

#[test]
fn unregister_keeps_siblings_alive() {
    let registry: SyncRegistry<u32> = SyncRegistry::new();
    let count = Arc::new(AtomicUsize::new(0));

    let c = Arc::clone(&count);
    let id_keep = registry.register(move |_| {
        let _ = c.fetch_add(1, Ordering::Relaxed);
    });
    let c = Arc::clone(&count);
    let id_remove = registry.register(move |_| {
        let _ = c.fetch_add(100, Ordering::Relaxed);
    });

    assert!(registry.unregister(id_remove));
    registry.notify(&1);
    assert_eq!(count.load(Ordering::Relaxed), 1);
    assert!(registry.contains(id_keep));
}

#[test]
fn clear_removes_every_handler() {
    let registry: SyncRegistry<u32> = SyncRegistry::new();
    for _ in 0..20 {
        let _ = registry.register(|_| {});
    }
    assert_eq!(registry.handler_count(), 20);

    registry.clear();
    assert_eq!(registry.handler_count(), 0);
    assert!(registry.is_empty());
}

#[test]
fn clear_followed_by_register_continues_to_issue_unique_ids() {
    let registry: SyncRegistry<u32> = SyncRegistry::new();
    let id_before = registry.register(|_| {});
    registry.clear();
    let id_after = registry.register(|_| {});
    assert_ne!(id_before, id_after);
}

#[test]
fn notify_passes_event_by_reference() {
    let registry: SyncRegistry<String> = SyncRegistry::new();
    let captured = Arc::new(std::sync::Mutex::new(String::new()));
    let sink = Arc::clone(&captured);
    let _ = registry.register(move |s: &String| {
        sink.lock().unwrap().clone_from(s);
    });

    let event = String::from("hello");
    registry.notify(&event);
    // The original event was not moved; we still own it.
    assert_eq!(event.as_str(), "hello");
    assert_eq!(captured.lock().unwrap().as_str(), "hello");
}

#[test]
fn registry_is_send_and_sync() {
    fn require_send_sync<T: Send + Sync>() {}
    require_send_sync::<SyncRegistry<u32>>();
    require_send_sync::<SyncRegistry<String>>();
}

#[test]
fn handler_id_round_trips_through_as_u64() {
    let registry: SyncRegistry<()> = SyncRegistry::new();
    let id = registry.register(|_| {});
    let raw = id.as_u64();
    assert!(raw > 0);
    assert!(registry.unregister(id));
}

#[test]
fn handler_can_capture_owned_data() {
    let registry: SyncRegistry<()> = SyncRegistry::new();
    let output = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

    for tag in ["alpha", "beta", "gamma"] {
        let sink = Arc::clone(&output);
        let owned = tag.to_owned();
        let _ = registry.register(move |_| {
            sink.lock().unwrap().push(owned.clone());
        });
    }

    registry.notify(&());
    assert_eq!(
        output.lock().unwrap().as_slice(),
        &["alpha".to_owned(), "beta".to_owned(), "gamma".to_owned(),]
    );
}

#[test]
fn many_register_unregister_cycles_keep_state_stable() {
    let registry: SyncRegistry<u32> = SyncRegistry::new();
    let mut ids = Vec::new();
    for _ in 0..1000 {
        ids.push(registry.register(|_| {}));
    }
    assert_eq!(registry.handler_count(), 1000);

    for id in ids {
        assert!(registry.unregister(id));
    }
    assert_eq!(registry.handler_count(), 0);
    assert!(registry.is_empty());

    // Subsequent registrations still work.
    let _ = registry.register(|_| {});
    assert_eq!(registry.handler_count(), 1);
}

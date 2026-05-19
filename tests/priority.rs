//! Priority ordering tests for `SyncRegistry`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use registry_io::SyncRegistry;

fn run_with_priorities(priorities: &[i32]) -> Vec<i32> {
    let registry: SyncRegistry<()> = SyncRegistry::new();
    let log = Arc::new(Mutex::new(Vec::<i32>::new()));
    for &p in priorities {
        let l = Arc::clone(&log);
        let _ = registry.register_with_priority(p, move |_| {
            l.lock().unwrap().push(p);
        });
    }
    registry.notify(&());
    log.lock().unwrap().clone()
}

#[test]
fn higher_priority_fires_first() {
    let order = run_with_priorities(&[0, 10, -5]);
    assert_eq!(order, vec![10, 0, -5]);
}

#[test]
fn equal_priority_fires_in_registration_order() {
    let registry: SyncRegistry<()> = SyncRegistry::new();
    let log = Arc::new(Mutex::new(Vec::<&'static str>::new()));

    let l = Arc::clone(&log);
    let _ = registry.register_with_priority(0, move |_| l.lock().unwrap().push("first"));
    let l = Arc::clone(&log);
    let _ = registry.register_with_priority(0, move |_| l.lock().unwrap().push("second"));
    let l = Arc::clone(&log);
    let _ = registry.register_with_priority(0, move |_| l.lock().unwrap().push("third"));

    registry.notify(&());
    assert_eq!(
        log.lock().unwrap().as_slice(),
        &["first", "second", "third"]
    );
}

#[test]
fn default_register_priority_is_zero() {
    let registry: SyncRegistry<()> = SyncRegistry::new();
    let log = Arc::new(Mutex::new(Vec::<&'static str>::new()));

    let l = Arc::clone(&log);
    let _ = registry.register_with_priority(5, move |_| l.lock().unwrap().push("high"));
    let l = Arc::clone(&log);
    let _ = registry.register(move |_| l.lock().unwrap().push("default"));
    let l = Arc::clone(&log);
    let _ = registry.register_with_priority(-5, move |_| l.lock().unwrap().push("low"));

    registry.notify(&());
    assert_eq!(log.lock().unwrap().as_slice(), &["high", "default", "low"]);
}

#[test]
fn priority_extremes_are_honored() {
    let order = run_with_priorities(&[i32::MIN, 0, i32::MAX]);
    assert_eq!(order, vec![i32::MAX, 0, i32::MIN]);
}

#[test]
fn unregister_preserves_order_of_remaining_handlers() {
    let registry: SyncRegistry<()> = SyncRegistry::new();
    let log = Arc::new(Mutex::new(Vec::<i32>::new()));

    let l = Arc::clone(&log);
    let a = registry.register_with_priority(10, move |_| l.lock().unwrap().push(10));
    let l = Arc::clone(&log);
    let _b = registry.register_with_priority(0, move |_| l.lock().unwrap().push(0));
    let l = Arc::clone(&log);
    let _c = registry.register_with_priority(-10, move |_| l.lock().unwrap().push(-10));

    assert!(registry.unregister(a));
    registry.notify(&());
    assert_eq!(log.lock().unwrap().as_slice(), &[0, -10]);
}

#[test]
fn inserting_mid_priority_does_not_reorder_existing_groups() {
    let registry: SyncRegistry<()> = SyncRegistry::new();
    let log = Arc::new(Mutex::new(Vec::<&'static str>::new()));

    let l = Arc::clone(&log);
    let _ = registry.register_with_priority(10, move |_| l.lock().unwrap().push("high_1"));
    let l = Arc::clone(&log);
    let _ = registry.register_with_priority(10, move |_| l.lock().unwrap().push("high_2"));
    let l = Arc::clone(&log);
    let _ = registry.register_with_priority(-10, move |_| l.lock().unwrap().push("low_1"));
    let l = Arc::clone(&log);
    let _ = registry.register_with_priority(0, move |_| l.lock().unwrap().push("mid_1"));

    registry.notify(&());
    assert_eq!(
        log.lock().unwrap().as_slice(),
        &["high_1", "high_2", "mid_1", "low_1",]
    );
}

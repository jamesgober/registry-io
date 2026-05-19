//! Priority ordering: higher priorities fire first, ties broken by
//! registration order.
//!
//! Run with: `cargo run --example priority`

use std::sync::{Arc, Mutex};

use registry_io::SyncRegistry;

fn main() {
    let registry: SyncRegistry<&'static str> = SyncRegistry::new();
    let order: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let o = Arc::clone(&order);
    let _ = registry.register_with_priority(50, move |event| {
        o.lock().unwrap().push(format!("hi-prio: {event}"));
    });

    let o = Arc::clone(&order);
    let _ = registry.register(move |event| {
        o.lock().unwrap().push(format!("default: {event}"));
    });

    let o = Arc::clone(&order);
    let _ = registry.register_with_priority(-100, move |event| {
        o.lock().unwrap().push(format!("lo-prio: {event}"));
    });

    registry.notify(&"ping");

    for line in order.lock().unwrap().iter() {
        println!("{line}");
    }
}

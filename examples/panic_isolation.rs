//! Panic isolation: one handler panicking does NOT stop sibling handlers,
//! and an `on_panic` callback receives an actionable `PanicInfo`.
//!
//! Run with: `cargo run --example panic_isolation`

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use registry_io::SyncRegistry;

fn main() {
    let registry: SyncRegistry<()> = SyncRegistry::new();

    let panic_count = Arc::new(AtomicU32::new(0));
    let sink = Arc::clone(&panic_count);
    registry.on_panic(move |info| {
        let _ = sink.fetch_add(1, Ordering::Relaxed);
        println!(
            "[panic] handler {} failed: {}",
            info.handler_id(),
            info.message().unwrap_or("<opaque>")
        );
    });

    let _ = registry.register(|_| {
        println!("handler 1: fine");
    });

    let _ = registry.register(|_| {
        panic!("handler 2 is broken");
    });

    let _ = registry.register(|_| {
        println!("handler 3: still ran");
    });

    let _ = registry.register(|_| {
        panic!("handler 4 also broken");
    });

    println!("dispatching to {} handlers...", registry.handler_count());
    registry.notify(&());
    println!(
        "notify returned cleanly. observed {} panics across handlers.",
        panic_count.load(Ordering::Relaxed)
    );
}

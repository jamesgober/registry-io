//! Basic `SyncRegistry` usage: register, notify, unregister.
//!
//! Run with: `cargo run --example basic`

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use registry_io::SyncRegistry;

#[derive(Debug)]
struct Tick(u32);

fn main() {
    let registry: SyncRegistry<Tick> = SyncRegistry::new();
    let counter = Arc::new(AtomicU32::new(0));

    let sink = Arc::clone(&counter);
    let id_add = registry.register(move |tick| {
        let _ = sink.fetch_add(tick.0, Ordering::Relaxed);
    });

    let _ = registry.register(|tick: &Tick| {
        println!("observed tick: {}", tick.0);
    });

    registry.notify(&Tick(2));
    registry.notify(&Tick(3));

    println!("total accumulated: {}", counter.load(Ordering::Relaxed));

    assert!(registry.unregister(id_add));
    registry.notify(&Tick(100));

    println!(
        "after unregistering the accumulator, total stayed at: {}",
        counter.load(Ordering::Relaxed)
    );
}

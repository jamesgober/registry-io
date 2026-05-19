//! RAII handler guards — automatic unregistration when the guard drops.
//!
//! Run with: `cargo run --example guards`

use std::sync::Arc;

use registry_io::SyncRegistry;

fn main() {
    let registry = Arc::new(SyncRegistry::<u32>::new());
    println!("initial handler count: {}", registry.handler_count());

    {
        let _guard = registry.register_guard(|n| {
            println!("guarded handler saw: {n}");
        });
        println!("inside scope: handler count = {}", registry.handler_count());
        registry.notify(&7);
    }
    println!("after scope: handler count = {}", registry.handler_count());

    // Use `forget` to detach the guard while keeping the handler registered.
    let guard = registry.register_guard(|n| {
        println!("detached handler saw: {n}");
    });
    let detached_id = guard.id();
    guard.forget();
    println!(
        "after forget: handler count = {}, detached id = {}",
        registry.handler_count(),
        detached_id
    );
    registry.notify(&42);

    // Manually unregister the detached handler.
    assert!(registry.unregister(detached_id));
    println!("final handler count: {}", registry.handler_count());
}

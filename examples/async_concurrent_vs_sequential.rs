//! Side-by-side comparison of concurrent vs sequential async dispatch.
//!
//! Each handler sleeps for 50ms. Concurrent dispatch (`notify`) runs all
//! handlers in parallel, so the total wall-clock is ~50ms. Sequential
//! dispatch (`notify_sequential`) awaits each one, so the wall-clock is
//! ~50ms × N handlers.
//!
//! Run with: `cargo run --example async_concurrent_vs_sequential --features async`

use std::time::{Duration, Instant};

use registry_io::r#async::AsyncRegistry;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let registry: AsyncRegistry<()> = AsyncRegistry::new();
    for i in 0..4 {
        let _ = registry.register(move |_| async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            println!("handler {i} done");
        });
    }

    let started = Instant::now();
    registry.notify(&()).await;
    println!("concurrent notify took {:?}\n", started.elapsed());

    let started = Instant::now();
    registry.notify_sequential(&()).await;
    println!("sequential notify took {:?}", started.elapsed());
}

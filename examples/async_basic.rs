//! Basic `AsyncRegistry` usage: register an async handler, notify, observe.
//!
//! Run with: `cargo run --example async_basic --features async`

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use registry_io::r#async::AsyncRegistry;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let registry: AsyncRegistry<u64> = AsyncRegistry::new();
    let total = Arc::new(AtomicU64::new(0));

    for label in ["alpha", "beta", "gamma"] {
        let sink = Arc::clone(&total);
        let _ = registry.register(move |value| {
            let sink = Arc::clone(&sink);
            let v = *value;
            async move {
                // Pretend this is real async work — a tokio::time::sleep, a
                // database write, an HTTP call. The registry doesn't care.
                tokio::task::yield_now().await;
                let _ = sink.fetch_add(v, Ordering::Relaxed);
                println!("[{label}] saw {v}");
            }
        });
    }

    registry.notify(&7).await;
    println!("total: {}", total.load(Ordering::Relaxed));
}

//! Benchmarks for the asynchronous `notify` path.
//!
//! Run with: `cargo bench --bench async_notify --features async`
//!
//! Two flavors are measured:
//!
//! - `async_notify/concurrent/N_handlers` — `AsyncRegistry::notify` via the
//!   crate-local `JoinAll` combinator.
//! - `async_notify/sequential/N_handlers` — `AsyncRegistry::notify_sequential`.
//!
//! Both use trivially-fast handlers (no real `.await` work). Their cost is
//! dominated by the boxed-future allocation per handler plus the
//! `catch_unwind` setup.

#![cfg(feature = "async")]

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use registry_io::r#async::AsyncRegistry;
use tokio::runtime::Builder;

fn build_runtime() -> tokio::runtime::Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread tokio runtime")
}

fn bench_concurrent_notify(c: &mut Criterion) {
    let rt = build_runtime();
    for handler_count in [0usize, 1, 4, 16].iter().copied() {
        let registry: AsyncRegistry<u64> = AsyncRegistry::new();
        for _ in 0..handler_count {
            let _ = registry.register(|value| {
                let v = *value;
                async move {
                    let _ = black_box(v);
                }
            });
        }

        let label = format!("async_notify/concurrent/{handler_count}_handlers");
        let _ = c.bench_function(&label, |b| {
            b.to_async(&rt).iter(|| async {
                registry.notify(black_box(&42_u64)).await;
            });
        });
    }
}

fn bench_sequential_notify(c: &mut Criterion) {
    let rt = build_runtime();
    for handler_count in [0usize, 1, 4, 16].iter().copied() {
        let registry: AsyncRegistry<u64> = AsyncRegistry::new();
        for _ in 0..handler_count {
            let _ = registry.register(|value| {
                let v = *value;
                async move {
                    let _ = black_box(v);
                }
            });
        }

        let label = format!("async_notify/sequential/{handler_count}_handlers");
        let _ = c.bench_function(&label, |b| {
            b.to_async(&rt).iter(|| async {
                registry.notify_sequential(black_box(&42_u64)).await;
            });
        });
    }
}

criterion_group!(benches, bench_concurrent_notify, bench_sequential_notify);
criterion_main!(benches);

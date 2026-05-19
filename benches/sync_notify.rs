//! Benchmarks for the synchronous `notify` hot path.
//!
//! Run with: `cargo bench --bench sync_notify`
//!
//! Scenarios:
//!
//! - `notify/0_handlers` — baseline cost of a no-op notify
//! - `notify/N_handlers` — dispatch cost with N registered handlers
//! - `notify/contended/N_threads` — dispatch under thread contention

use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use criterion::{Criterion, criterion_group, criterion_main};
use registry_io::SyncRegistry;

fn bench_notify_handler_count(c: &mut Criterion) {
    for handler_count in [0usize, 1, 4, 16, 64].iter().copied() {
        let registry: SyncRegistry<u64> = SyncRegistry::new();
        for _ in 0..handler_count {
            let _ = registry.register(|value| {
                let _ = black_box(*value);
            });
        }

        let label = format!("notify/{handler_count}_handlers");
        let _ = c.bench_function(&label, |b| {
            b.iter(|| {
                registry.notify(black_box(&42_u64));
            });
        });
    }
}

fn bench_notify_contended(c: &mut Criterion) {
    for threads in [1usize, 4, 16].iter().copied() {
        let registry = Arc::new(SyncRegistry::<u64>::new());
        for _ in 0..4 {
            let counter = Arc::new(AtomicU64::new(0));
            let sink = Arc::clone(&counter);
            let _ = registry.register(move |value| {
                sink.fetch_add(*value, Ordering::Relaxed);
            });
        }

        let label = format!("notify/contended/{threads}_threads");
        let _ = c.bench_function(&label, |b| {
            b.iter_custom(|iters| {
                let start = std::time::Instant::now();
                thread::scope(|s| {
                    for _ in 0..threads {
                        let r = Arc::clone(&registry);
                        let _ = s.spawn(move || {
                            for _ in 0..iters / threads as u64 {
                                r.notify(black_box(&1_u64));
                            }
                        });
                    }
                });
                start.elapsed()
            });
        });
    }
}

criterion_group!(benches, bench_notify_handler_count, bench_notify_contended);
criterion_main!(benches);

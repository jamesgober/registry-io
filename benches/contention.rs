//! Thread-contention benchmark for `SyncRegistry::notify`.
//!
//! Sweeps `1, 4, 16, 64` concurrent notifiers against a fixed handler set,
//! measuring per-notify wall-clock time under contended reads of the
//! lock-free `ArcSwap` snapshot.
//!
//! Run with: `cargo bench --bench contention`
//!
//! Each benchmark uses `iter_custom` and `thread::scope` to launch worker
//! threads inside the timed region. The reported nanoseconds are
//! `elapsed / total_notifies`, so the number is *per notify call*, not
//! *per benchmark iteration*.

use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Instant;

use criterion::{Criterion, criterion_group, criterion_main};
use registry_io::SyncRegistry;

fn bench_contended_notify(c: &mut Criterion) {
    for thread_count in [1usize, 4, 16, 64].iter().copied() {
        for handler_count in [1usize, 4, 16].iter().copied() {
            let registry = Arc::new(SyncRegistry::<u64>::new());
            let total = Arc::new(AtomicU64::new(0));
            for _ in 0..handler_count {
                let sink = Arc::clone(&total);
                let _ = registry.register(move |value| {
                    let _ = sink.fetch_add(*value, Ordering::Relaxed);
                });
            }

            let label =
                format!("contention/notify/{thread_count}_threads/{handler_count}_handlers");
            let _ = c.bench_function(&label, |b| {
                b.iter_custom(|iters| {
                    let per_thread = iters / thread_count as u64;
                    let start = Instant::now();
                    thread::scope(|s| {
                        for _ in 0..thread_count {
                            let r = Arc::clone(&registry);
                            let _ = s.spawn(move || {
                                for _ in 0..per_thread {
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
}

criterion_group!(benches, bench_contended_notify);
criterion_main!(benches);

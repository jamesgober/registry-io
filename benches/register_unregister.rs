//! Benchmarks for the slow-path `register` and `unregister` operations.
//!
//! Run with: `cargo bench --bench register_unregister`

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use registry_io::SyncRegistry;

fn bench_register(c: &mut Criterion) {
    for existing in [0usize, 16, 100, 1000].iter().copied() {
        let label = format!("register/into_{existing}_handlers");
        let _ = c.bench_function(&label, |b| {
            b.iter_batched(
                || {
                    let registry: SyncRegistry<u64> = SyncRegistry::new();
                    for _ in 0..existing {
                        let _ = registry.register(|v| {
                            let _ = black_box(*v);
                        });
                    }
                    registry
                },
                |registry| {
                    let id = registry.register(|v| {
                        let _ = black_box(*v);
                    });
                    black_box(id);
                },
                BatchSize::SmallInput,
            );
        });
    }
}

fn bench_unregister(c: &mut Criterion) {
    for existing in [1usize, 16, 100, 1000].iter().copied() {
        let label = format!("unregister/from_{existing}_handlers");
        let _ = c.bench_function(&label, |b| {
            b.iter_batched(
                || {
                    let registry: SyncRegistry<u64> = SyncRegistry::new();
                    let mut ids = Vec::with_capacity(existing);
                    for _ in 0..existing {
                        ids.push(registry.register(|v| {
                            let _ = black_box(*v);
                        }));
                    }
                    (registry, ids[existing / 2])
                },
                |(registry, id)| {
                    let _ = black_box(registry.unregister(id));
                },
                BatchSize::SmallInput,
            );
        });
    }
}

criterion_group!(benches, bench_register, bench_unregister);
criterion_main!(benches);

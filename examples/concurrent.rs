//! Concurrent notify from many threads against a shared registry.
//!
//! Demonstrates the lock-free read path: 16 threads each fire `notify`
//! 10_000 times concurrently while four handlers tally the events.
//!
//! Run with: `cargo run --release --example concurrent`

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Instant;

use registry_io::SyncRegistry;

fn main() {
    let registry = Arc::new(SyncRegistry::<u64>::new());
    let total = Arc::new(AtomicU64::new(0));

    for _ in 0..4 {
        let sink = Arc::clone(&total);
        let _ = registry.register(move |value| {
            let _ = sink.fetch_add(*value, Ordering::Relaxed);
        });
    }

    let threads = 16usize;
    let per_thread = 10_000usize;
    let started = Instant::now();

    let mut handles = Vec::new();
    for _ in 0..threads {
        let r = Arc::clone(&registry);
        handles.push(thread::spawn(move || {
            for _ in 0..per_thread {
                r.notify(&1);
            }
        }));
    }
    for h in handles {
        h.join().expect("worker did not panic");
    }

    let elapsed = started.elapsed();
    let notifies = (threads * per_thread) as u128;
    let per_notify_ns = elapsed.as_nanos() / notifies;

    println!("threads: {threads}");
    println!("notifies per thread: {per_thread}");
    println!("handlers per notify: 4");
    println!("total counted: {}", total.load(Ordering::Relaxed));
    println!("elapsed: {elapsed:?}");
    println!("ns per notify (4 handlers, 16-way contended): ~{per_notify_ns}");
}

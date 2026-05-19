//! Pattern: **metric-event collection**.
//!
//! The hot path of a busy server is the wrong place to call a metric
//! exporter directly: an `HTTP POST` to Prometheus or a `tcp::send` to
//! StatsD would dominate the budget. `SyncRegistry<MetricEvent>` lets
//! the hot path fire a typed event in ~10 ns and lets the actual
//! shipping happen out-of-band in handler closures (which may, in turn,
//! offload to a worker pool).
//!
//! The example below demonstrates one in-process aggregator (lockless,
//! good for tight loops) plus one batching exporter (collects events,
//! flushes periodically — what you'd attach a real exporter to).
//!
//! Run with: `cargo run --example pattern_metric_event`

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use registry_io::SyncRegistry;

#[derive(Debug, Clone)]
#[allow(dead_code)] // `route` and `kind` are part of the public event
// schema but the in-tree aggregator below only
// discriminates on the variant — real exporters
// would emit them.
enum MetricEvent {
    RequestStarted {
        route: &'static str,
    },
    RequestCompleted {
        route: &'static str,
        latency_us: u64,
    },
    Error {
        route: &'static str,
        kind: &'static str,
    },
}

/// Atomic in-process counters. Updated inside handlers — no allocation,
/// no lock contention.
#[derive(Default)]
struct Counters {
    requests_started: AtomicU64,
    requests_completed: AtomicU64,
    total_latency_us: AtomicU64,
    errors: AtomicU64,
}

impl Counters {
    fn snapshot(&self) -> (u64, u64, u64, u64) {
        (
            self.requests_started.load(Ordering::Relaxed),
            self.requests_completed.load(Ordering::Relaxed),
            self.total_latency_us.load(Ordering::Relaxed),
            self.errors.load(Ordering::Relaxed),
        )
    }
}

/// A second handler that batches raw events for an out-of-band exporter
/// (HTTP shipper, StatsD writer, etc.) to drain.
#[derive(Default)]
struct EventBatch {
    pending: Mutex<Vec<MetricEvent>>,
}

impl EventBatch {
    fn drain(&self) -> Vec<MetricEvent> {
        std::mem::take(&mut *self.pending.lock().unwrap())
    }
}

fn main() {
    let bus: SyncRegistry<MetricEvent> = SyncRegistry::new();
    let counters: Arc<Counters> = Arc::new(Counters::default());
    let batch: Arc<EventBatch> = Arc::new(EventBatch::default());

    // Aggregator handler — runs in nanoseconds, never blocks.
    {
        let counters = Arc::clone(&counters);
        let _ = bus.register(move |evt: &MetricEvent| match evt {
            MetricEvent::RequestStarted { .. } => {
                let _ = counters.requests_started.fetch_add(1, Ordering::Relaxed);
            }
            MetricEvent::RequestCompleted { latency_us, .. } => {
                let _ = counters.requests_completed.fetch_add(1, Ordering::Relaxed);
                let _ = counters
                    .total_latency_us
                    .fetch_add(*latency_us, Ordering::Relaxed);
            }
            MetricEvent::Error { .. } => {
                let _ = counters.errors.fetch_add(1, Ordering::Relaxed);
            }
        });
    }

    // Batching handler — accumulates raw events for an exporter.
    {
        let batch = Arc::clone(&batch);
        let _ = bus.register(move |evt: &MetricEvent| {
            batch.pending.lock().unwrap().push(evt.clone());
        });
    }

    // Simulated hot-path traffic.
    for i in 0..1_000 {
        bus.notify(&MetricEvent::RequestStarted {
            route: "/api/v1/data",
        });
        if i % 97 == 0 {
            bus.notify(&MetricEvent::Error {
                route: "/api/v1/data",
                kind: "timeout",
            });
        }
        bus.notify(&MetricEvent::RequestCompleted {
            route: "/api/v1/data",
            latency_us: 150 + (i % 50) as u64,
        });
    }

    let (started, completed, total_us, errors) = counters.snapshot();
    let avg_us = total_us.checked_div(completed).unwrap_or(0);
    println!("aggregated metrics:");
    println!("  requests_started   = {started}");
    println!("  requests_completed = {completed}");
    println!("  avg_latency_us     = {avg_us}");
    println!("  errors             = {errors}");

    let drained = batch.drain();
    println!("\nbatched events pending export: {}", drained.len());
    println!("  (an out-of-band exporter would ship these now)");
}

//! Pattern: **hot-reload notification**.
//!
//! Concrete-but-minimal reproduction of the workflow `config-lib` and
//! similar configuration crates need: a single producer mutates a
//! configuration value and many subscribers must immediately re-derive
//! their internal state from the new value.
//!
//! Replaces the typical `mpsc::channel`-per-subscriber wiring with one
//! shared `SyncRegistry<Snapshot>` that fan-outs every change in
//! ~10 ns/handler, with no per-event allocation.
//!
//! Run with: `cargo run --example pattern_hot_reload`

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use registry_io::SyncRegistry;

/// What the rest of the application is allowed to observe about the
/// current configuration. Cheap to construct; cheap to share by `&`.
#[derive(Debug, Clone)]
struct ConfigSnapshot {
    generation: u64,
    request_timeout_ms: u32,
    feature_flag_search_v2: bool,
}

/// The `Config` type itself owns the registry and is the only entity
/// that mutates the snapshot.
struct Config {
    snapshot: RwLock<Arc<ConfigSnapshot>>,
    on_change: Arc<SyncRegistry<ConfigSnapshot>>,
    generation: AtomicU64,
}

impl Config {
    fn new(initial: ConfigSnapshot) -> Self {
        Self {
            snapshot: RwLock::new(Arc::new(initial)),
            on_change: Arc::new(SyncRegistry::new()),
            generation: AtomicU64::new(0),
        }
    }

    /// Read the current snapshot: a brief read lock and one `Arc` clone.
    fn current(&self) -> Arc<ConfigSnapshot> {
        Arc::clone(&self.snapshot.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// Apply a mutation atomically, then notify every subscriber.
    fn mutate<F>(&self, f: F)
    where
        F: FnOnce(&mut ConfigSnapshot),
    {
        let next = {
            let mut slot = self
                .snapshot
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            let mut next = (**slot).clone();
            f(&mut next);
            next.generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
            let next = Arc::new(next);
            *slot = Arc::clone(&next);
            next
        };
        // Notify outside the lock so subscribers may read `current()`.
        self.on_change.notify(&*next);
    }
}

fn main() {
    let config = Config::new(ConfigSnapshot {
        generation: 0,
        request_timeout_ms: 30_000,
        feature_flag_search_v2: false,
    });

    let timeout_observed = Arc::new(AtomicU64::new(0));
    let toggle_count = Arc::new(AtomicUsize::new(0));

    // Subscriber 1: re-derive an HTTP-client timeout.
    let sink = Arc::clone(&timeout_observed);
    let _ = config.on_change.register(move |snap: &ConfigSnapshot| {
        sink.store(u64::from(snap.request_timeout_ms), Ordering::Relaxed);
    });

    // Subscriber 2: count feature-flag toggles. `Fn` handlers can't
    // mutate captured locals directly, so we share state via an atomic.
    let sink = Arc::clone(&toggle_count);
    let last_v2 = Arc::new(AtomicBool::new(false));
    let last = Arc::clone(&last_v2);
    let _ = config.on_change.register(move |snap: &ConfigSnapshot| {
        let prev = last.swap(snap.feature_flag_search_v2, Ordering::Relaxed);
        if prev != snap.feature_flag_search_v2 {
            let _ = sink.fetch_add(1, Ordering::Relaxed);
        }
    });

    println!(
        "initial:        gen={} timeout={}ms search_v2={}",
        config.current().generation,
        config.current().request_timeout_ms,
        config.current().feature_flag_search_v2,
    );

    config.mutate(|c| c.request_timeout_ms = 5_000);
    config.mutate(|c| c.feature_flag_search_v2 = true);
    config.mutate(|c| c.feature_flag_search_v2 = false);
    config.mutate(|c| c.feature_flag_search_v2 = true);

    println!(
        "after mutates:  gen={} timeout={}ms search_v2={}",
        config.current().generation,
        config.current().request_timeout_ms,
        config.current().feature_flag_search_v2,
    );
    println!(
        "timeout subscriber latest:    {}ms",
        timeout_observed.load(Ordering::Relaxed)
    );
    println!(
        "toggle subscriber transitions: {}",
        toggle_count.load(Ordering::Relaxed)
    );
}

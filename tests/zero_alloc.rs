//! Verify that `SyncRegistry::notify` allocates **zero** heap blocks on the
//! no-panic hot path.
//!
//! Gated behind the `dhat-heap` feature because it installs `dhat::Alloc`
//! as the global allocator, which would skew any other benchmark or test
//! run in the same `cargo` invocation.
//!
//! Run with:
//!
//! ```text
//! cargo test --features dhat-heap --test zero_alloc -- --nocapture
//! ```

#![cfg(feature = "dhat-heap")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use registry_io::SyncRegistry;

/// Both scenarios share one `dhat::Profiler` because dhat allows at most
/// one profiler per process; running two concurrent `#[test]` functions
/// (the rust test harness default) would panic with
/// `creating a profiler while a profiler is already running`.
#[test]
fn sync_notify_hot_path_does_not_allocate() {
    let _profiler = dhat::Profiler::builder().testing().build();

    // ----- Scenario 1: empty registry ---------------------------------
    {
        let registry: SyncRegistry<u64> = SyncRegistry::new();
        for _ in 0..1_000 {
            registry.notify(&1);
        }
        let before = dhat::HeapStats::get();
        for _ in 0..100_000 {
            registry.notify(&1);
        }
        let after = dhat::HeapStats::get();
        let new_blocks = after.total_blocks.saturating_sub(before.total_blocks);
        assert_eq!(
            new_blocks, 0,
            "empty notify() allocated {new_blocks} new blocks across 100k calls"
        );
    }

    // ----- Scenario 2: 8 registered handlers --------------------------
    {
        let registry: SyncRegistry<u64> = SyncRegistry::new();
        let counter = Arc::new(AtomicU64::new(0));
        for _ in 0..8 {
            let sink = Arc::clone(&counter);
            let _ = registry.register(move |value| {
                let _ = sink.fetch_add(*value, Ordering::Relaxed);
            });
        }

        // Warmup so any one-shot lazy allocations land before accounting.
        for _ in 0..1_000 {
            registry.notify(&1);
        }

        let before = dhat::HeapStats::get();
        for _ in 0..100_000 {
            registry.notify(&1);
        }
        let after = dhat::HeapStats::get();

        let new_blocks = after.total_blocks.saturating_sub(before.total_blocks);
        let new_bytes = after.total_bytes.saturating_sub(before.total_bytes);
        assert_eq!(
            new_blocks, 0,
            "notify() with 8 handlers allocated {new_blocks} new blocks ({new_bytes} bytes) across 100k calls"
        );
        assert_eq!(
            new_bytes, 0,
            "notify() with 8 handlers touched {new_bytes} bytes across 100k calls; expected zero"
        );
    }
}

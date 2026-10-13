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

/// Calls per measurement window.
const CALLS: u64 = 100_000;

/// Measures `body` over [`CALLS`] calls and returns the fewest new heap
/// blocks seen in up to three windows.
///
/// dhat counts allocations from every thread in the process, and the test
/// harness's own threads allocate while this test starts up (bookkeeping
/// after spawning the test thread). A window that overlaps that work sees
/// a handful of blocks that are not ours. So each window is preceded by a
/// short pause, and the minimum over the windows is used: a genuine
/// hot-path allocation shows up as at least one block per call in every
/// window, which the per-window ceiling below also rejects outright.
fn fewest_new_blocks(mut body: impl FnMut()) -> u64 {
    let mut fewest = u64::MAX;
    for _ in 0..3 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        let before = dhat::HeapStats::get();
        for _ in 0..CALLS {
            body();
        }
        let after = dhat::HeapStats::get();
        let new_blocks = after.total_blocks.saturating_sub(before.total_blocks);
        assert!(
            new_blocks < CALLS / 1_000,
            "{new_blocks} new blocks in {CALLS} calls: allocation on the hot path"
        );
        fewest = fewest.min(new_blocks);
        if fewest == 0 {
            break;
        }
    }
    fewest
}

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
        let new_blocks = fewest_new_blocks(|| registry.notify(&1));
        assert_eq!(
            new_blocks, 0,
            "empty notify() allocated {new_blocks} new blocks across {CALLS} calls"
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

        let new_blocks = fewest_new_blocks(|| registry.notify(&1));
        assert_eq!(
            new_blocks, 0,
            "notify() with 8 handlers allocated {new_blocks} new blocks across {CALLS} calls"
        );
    }
}

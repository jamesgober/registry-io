//! Fuzz target: random sequences of register / unregister / clear / notify
//! operations against a `SyncRegistry<u32>`.
//!
//! The fuzzer drives the registry from an arbitrary byte stream decoded
//! into an [`Op`] sequence. Invariants checked after each operation:
//!
//! - `handler_count` matches the test's own bookkeeping.
//! - Every successful `unregister` returns `true`; every duplicate
//!   `unregister` returns `false`.
//! - `notify` never panics (panics inside handlers are caught by the
//!   registry itself).
//!
//! Run with `cargo +nightly fuzz run handler_churn`.

#![no_main]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;
use registry_io::SyncRegistry;

#[derive(Arbitrary, Debug)]
enum Op {
    Register { panicky: bool },
    UnregisterIndex(u8),
    UnregisterStale,
    Clear,
    Notify(u32),
    HandlerCount,
    Contains(u8),
}

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    let Ok(ops) = Vec::<Op>::arbitrary(&mut u) else {
        return;
    };

    let registry: SyncRegistry<u32> = SyncRegistry::new();
    let invocations = Arc::new(AtomicUsize::new(0));
    let mut live_ids: Vec<registry_io::HandlerId> = Vec::new();
    let mut stale_id: Option<registry_io::HandlerId> = None;

    for op in ops {
        match op {
            Op::Register { panicky } => {
                let counter = Arc::clone(&invocations);
                let id = if panicky {
                    registry.register(move |_| {
                        let _ = counter.fetch_add(1, Ordering::Relaxed);
                        panic!("fuzz panic");
                    })
                } else {
                    registry.register(move |_| {
                        let _ = counter.fetch_add(1, Ordering::Relaxed);
                    })
                };
                live_ids.push(id);
            }
            Op::UnregisterIndex(idx) => {
                if live_ids.is_empty() {
                    continue;
                }
                let pos = (idx as usize) % live_ids.len();
                let id = live_ids.swap_remove(pos);
                assert!(registry.unregister(id), "live id must be removable");
                stale_id = Some(id);
            }
            Op::UnregisterStale => {
                if let Some(id) = stale_id {
                    assert!(!registry.unregister(id), "stale id must return false");
                }
            }
            Op::Clear => {
                registry.clear();
                live_ids.clear();
            }
            Op::Notify(value) => {
                registry.notify(&value);
            }
            Op::HandlerCount => {
                assert_eq!(registry.handler_count(), live_ids.len());
            }
            Op::Contains(idx) => {
                if !live_ids.is_empty() {
                    let pos = (idx as usize) % live_ids.len();
                    let id = live_ids[pos];
                    assert!(registry.contains(id), "live id must be contained");
                }
            }
        }
    }
});

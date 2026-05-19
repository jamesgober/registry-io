//! Fuzz target: register a fixed set of handlers and dispatch arbitrary
//! event payloads to them.
//!
//! Where `handler_churn` focuses on the registry state machine, this
//! target focuses on the **dispatch path** — does anything inside the
//! `notify` loop break when fed adversarial event values?
//!
//! Run with `cargo +nightly fuzz run event_payload`.

#![no_main]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use libfuzzer_sys::fuzz_target;
use registry_io::SyncRegistry;

#[derive(Debug)]
struct FuzzEvent<'a> {
    tag: u32,
    payload: &'a [u8],
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let registry: SyncRegistry<FuzzEvent<'_>> = SyncRegistry::new();
    let total_bytes = Arc::new(AtomicUsize::new(0));
    let max_tag = Arc::new(AtomicUsize::new(0));

    let s = Arc::clone(&total_bytes);
    let _ = registry.register(move |evt: &FuzzEvent<'_>| {
        let _ = s.fetch_add(evt.payload.len(), Ordering::Relaxed);
    });
    let s = Arc::clone(&max_tag);
    let _ = registry.register(move |evt: &FuzzEvent<'_>| {
        let _ = s.fetch_max(evt.tag as usize, Ordering::Relaxed);
    });
    // A panicky handler that fires on every other event — verifies that
    // panic isolation holds across diverse payloads.
    let _ = registry.register(move |evt: &FuzzEvent<'_>| {
        if evt.tag % 2 == 0 {
            panic!("fuzz panic on tag {}", evt.tag);
        }
    });

    // Chunk the input bytes into events. Tag = leading 4 bytes; rest =
    // payload. Drive notify across each event in order.
    let mut cursor = 0;
    while cursor < data.len() {
        let take = std::cmp::min(64, data.len() - cursor);
        let chunk = &data[cursor..cursor + take];
        let tag = u32::from_le_bytes([
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
            chunk.get(3).copied().unwrap_or(0),
        ]);
        let payload = chunk.get(4..).unwrap_or(&[]);
        let evt = FuzzEvent { tag, payload };
        registry.notify(&evt);
        cursor += take;
    }
});

//! Loom models of the hazard-pointer handshake behind `SnapCell`
//! (`src/snapshot.rs` `protect` / `update` / `try_reclaim` /
//! `collect_unprotected`, `src/hazard.rs` `pin` / `is_announced`).
//!
//! "Freeing" a retired value is modelled as a write to its `UnsafeCell`;
//! the reader's use is a read. Loom reports a causality violation if the
//! two can race, which is exactly a use-after-free in the real code.
//!
//! The crate writes the handshake with `SeqCst` stores and loads. Loom
//! models `SeqCst` accesses as acquire/release only and reports a false
//! use-after-free for that form, so these models use the equivalent
//! `SeqCst`-fence form, which loom checks exactly.
//!
//! Run with:
//!
//! ```text
//! RUSTFLAGS="--cfg loom" LOOM_MAX_PREEMPTIONS=4 cargo test --release --test loom_handshake
//! ```

#![cfg(loom)]
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use loom::cell::UnsafeCell;
use loom::sync::Arc;
use loom::sync::atomic::{AtomicUsize, Ordering, fence};
use loom::thread;

struct Cell {
    current: AtomicUsize,
    hazard: AtomicUsize,
    values: [UnsafeCell<u64>; 2],
}

fn cell() -> Arc<Cell> {
    Arc::new(Cell {
        current: AtomicUsize::new(0),
        hazard: AtomicUsize::new(0),
        values: [UnsafeCell::new(7), UnsafeCell::new(8)],
    })
}

/// One reader, one writer: the reader announces, validates and reads;
/// the writer swaps and frees the old value only when it is not
/// announced.
#[test]
fn handshake_with_seqcst_fences() {
    loom::model(|| {
        let c = cell();
        let r = Arc::clone(&c);
        let reader = thread::spawn(move || {
            let mut p = r.current.load(Ordering::Acquire);
            r.hazard.store(p + 1, Ordering::Relaxed);
            fence(Ordering::SeqCst);
            loop {
                let q = r.current.load(Ordering::Acquire);
                if q == p {
                    break;
                }
                p = q;
                r.hazard.store(p + 1, Ordering::Relaxed);
                fence(Ordering::SeqCst);
            }
            let v = r.values[p].with(|v| unsafe { *v });
            r.hazard.store(0, Ordering::Release);
            v
        });
        c.current.store(1, Ordering::Release);
        fence(Ordering::SeqCst);
        if c.hazard.load(Ordering::Acquire) != 1 {
            c.values[0].with_mut(|v| unsafe { *v = 0xdead });
        }
        let _ = reader.join().unwrap();
    });
}

/// Reader-side reclamation (`release` -> `try_reclaim`) racing a writer:
/// the reader drops its hazard and, if anything is retired, frees what
/// no hazard announces. Two readers, one writer. A value must never be
/// freed twice nor freed while announced. Fenced variant.
#[test]
fn reader_reclaim_vs_writer_two_readers_fenced() {
    loom::model(|| {
        struct C2 {
            current: AtomicUsize,
            hazards: [AtomicUsize; 2],
            values: [UnsafeCell<u64>; 2],
            // 0 = live, 1 = retired, 2 = freed. Mutated only under `lock`.
            state: [UnsafeCell<u8>; 2],
            lock: loom::sync::Mutex<()>,
        }
        let c = Arc::new(C2 {
            current: AtomicUsize::new(0),
            hazards: [AtomicUsize::new(0), AtomicUsize::new(0)],
            values: [UnsafeCell::new(7), UnsafeCell::new(8)],
            state: [UnsafeCell::new(0), UnsafeCell::new(0)],
            lock: loom::sync::Mutex::new(()),
        });
        fn collect(c: &C2) {
            fence(Ordering::SeqCst);
            for i in 0..2 {
                let st = c.state[i].with(|s| unsafe { *s });
                if st == 1
                    && c.hazards[0].load(Ordering::Acquire) != i + 1
                    && c.hazards[1].load(Ordering::Acquire) != i + 1
                {
                    c.values[i].with_mut(|v| unsafe { *v = 0xdead });
                    c.state[i].with_mut(|s| unsafe { *s = 2 });
                }
            }
        }
        let mut readers = Vec::new();
        for slot in 0..2 {
            let r = Arc::clone(&c);
            readers.push(thread::spawn(move || {
                let mut p = r.current.load(Ordering::Acquire);
                r.hazards[slot].store(p + 1, Ordering::Relaxed);
                fence(Ordering::SeqCst);
                loop {
                    let q = r.current.load(Ordering::Acquire);
                    if q == p {
                        break;
                    }
                    p = q;
                    r.hazards[slot].store(p + 1, Ordering::Relaxed);
                    fence(Ordering::SeqCst);
                }
                let _v = r.values[p].with(|v| unsafe { *v });
                r.hazards[slot].store(0, Ordering::Release);
                if let Ok(_g) = r.lock.try_lock() {
                    collect(&r);
                }
            }));
        }
        {
            let _g = c.lock.lock().unwrap();
            c.current.store(1, Ordering::Release);
            c.state[0].with_mut(|s| unsafe { *s = 1 });
            collect(&c);
        }
        for r in readers {
            r.join().unwrap();
        }
    });
}

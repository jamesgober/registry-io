//! Process-wide hazard-pointer domain.
//!
//! A *hazard pointer* is a per-thread announcement: "I am reading the
//! object at this address; do not free it." Readers publish one before
//! dereferencing a shared snapshot and clear it afterwards. Writers that
//! retire a snapshot scan every announcement and free the snapshot only
//! when no thread has announced it.
//!
//! # Layout
//!
//! Each thread that ever dispatches gets one [`Record`] holding
//! [`SLOTS`] announcement slots, one per level of nested dispatch (a
//! handler that itself calls `notify` uses the next slot). Records live in
//! a lock-free, append-only linked list. A record is never freed: when its
//! thread exits, the record is marked free and the next new thread reuses
//! it, so the list length is bounded by the peak number of threads that
//! were dispatching at the same time.
//!
//! This list is the crate's only global state. It is required because a
//! hazard announcement must be visible to writers on every thread, for
//! every registry; it is thread-safe by construction and never mutated
//! except through atomics.
//!
//! # Cost
//!
//! Protecting a pointer is one thread-local lookup, a scan for a free
//! slot (almost always the first), one sequentially-consistent store, and
//! one load to validate. Releasing is one release store. Scanning is
//! `O(threads * SLOTS)` and only happens on the write path.

use core::cell::Cell;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

/// Announcement slots per thread: the deepest nesting of dispatch (a
/// handler calling `notify`, whose handler calls `notify`, ...) that can be
/// protected without falling back to the slower locked path.
pub(crate) const SLOTS: usize = 8;

/// One thread's announcements.
///
/// Aligned to 128 bytes so no two threads' records share a cache line
/// (or an adjacent-line prefetch pair on Intel and Apple silicon).
/// Without this, records allocated next to each other false-share, and a
/// thread publishing an announcement stalls every thread whose record sits
/// on the same line.
#[repr(align(128))]
pub(crate) struct Record {
    slots: [AtomicPtr<()>; SLOTS],
    in_use: AtomicBool,
    next: AtomicPtr<Record>,
}

/// Head of the append-only list of records.
static HEAD: AtomicPtr<Record> = AtomicPtr::new(ptr::null_mut());

/// The calling thread's record, claimed on first use and released at
/// thread exit.
struct Local {
    record: Cell<*const Record>,
}

impl Drop for Local {
    fn drop(&mut self) {
        let record = self.record.get();
        if !record.is_null() {
            // SAFETY: records are leaked on creation and never freed, so a
            // non-null pointer stored here is valid for the whole process.
            let record = unsafe { &*record };
            // Every slot is null here: a `Pin` borrows the record for the
            // length of one dispatch, and dispatches cannot outlive the
            // thread. Clearing defensively costs nothing on this cold path.
            for slot in &record.slots {
                slot.store(ptr::null_mut(), Ordering::Release);
            }
            record.in_use.store(false, Ordering::Release);
        }
    }
}

std::thread_local! {
    static LOCAL: Local = const { Local { record: Cell::new(ptr::null()) } };
}

/// A published announcement. Clears its slot when dropped, including
/// during unwinding.
pub(crate) struct Pin {
    slot: &'static AtomicPtr<()>,
}

impl Pin {
    /// Publishes `p` in this slot. Sequentially consistent so that it is
    /// ordered before the validating load that follows, and against the
    /// writer's swap and scan (the classic hazard-pointer handshake).
    #[inline]
    pub(crate) fn protect(&self, p: *const ()) {
        self.slot.store(p.cast_mut(), Ordering::SeqCst);
    }
}

impl Drop for Pin {
    #[inline]
    fn drop(&mut self) {
        self.slot.store(ptr::null_mut(), Ordering::Release);
    }
}

/// Claims a free announcement slot for the calling thread and publishes
/// `p` in it. Claiming and publishing are one step, so a slot can never be
/// handed out twice.
///
/// Returns `None` when the thread has no record available (its
/// thread-local storage is already being torn down) or when all
/// [`SLOTS`] are in use by nested dispatches. Callers then use their
/// locked fallback path.
#[inline]
pub(crate) fn pin(p: *const ()) -> Option<Pin> {
    LOCAL
        .try_with(|local| {
            let mut record = local.record.get();
            if record.is_null() {
                record = acquire_record();
                local.record.set(record);
            }
            // SAFETY: records are leaked on creation and never freed.
            let record: &'static Record = unsafe { &*record };
            // Only the owning thread stores non-null values into its own
            // slots, so a relaxed load reliably reports whether this
            // thread is using a slot.
            record
                .slots
                .iter()
                .find(|slot| slot.load(Ordering::Relaxed).is_null())
                .map(|slot| {
                    let pin = Pin { slot };
                    pin.protect(p);
                    pin
                })
        })
        .ok()
        .flatten()
}

/// Whether the calling thread currently announces `p`, meaning the caller
/// is running inside a dispatch over `p` on this thread.
#[cfg(test)]
pub(crate) fn this_thread_protects(p: *const ()) -> bool {
    LOCAL
        .try_with(|local| {
            let record = local.record.get();
            if record.is_null() {
                return false;
            }
            // SAFETY: records are leaked on creation and never freed.
            let record = unsafe { &*record };
            record
                .slots
                .iter()
                .any(|slot| slot.load(Ordering::Relaxed).cast_const() == p)
        })
        .unwrap_or(false)
}

/// Whether any thread other than the caller announces `p`.
#[cfg(any(test, feature = "sync"))]
pub(crate) fn others_protect(p: *const ()) -> bool {
    let own = LOCAL.try_with(|l| l.record.get()).unwrap_or(ptr::null());
    let mut found = false;
    for_each_record(|record| {
        if ptr::eq(record, own) {
            return;
        }
        if record
            .slots
            .iter()
            .any(|slot| slot.load(Ordering::SeqCst).cast_const() == p)
        {
            found = true;
        }
    });
    found
}

/// Every pointer currently announced by any thread, in an unspecified
/// order. Used by writers to decide which retired snapshots are safe to
/// free.
pub(crate) fn announced() -> Vec<*const ()> {
    let mut out = Vec::new();
    for_each_record(|record| {
        for slot in &record.slots {
            let p = slot.load(Ordering::SeqCst);
            if !p.is_null() {
                out.push(p.cast_const());
            }
        }
    });
    out
}

fn for_each_record(mut f: impl FnMut(&'static Record)) {
    // SeqCst, not Acquire. A writer's scan must be ordered after its swap
    // of `current` in the single total order of SeqCst operations, and a
    // new thread's publication of its record (the SeqCst CAS in
    // `acquire_record`) is ordered before that thread's announcement and
    // validating load. Then either this load sees the new record (and the
    // scan sees its announcement), or the new thread's validating load
    // sees the swap and retries. With Acquire here, both could miss each
    // other and a value in use could be freed; Miri's weak-memory
    // emulation found exactly that interleaving.
    let mut cursor = HEAD.load(Ordering::SeqCst);
    while !cursor.is_null() {
        // SAFETY: records are leaked on creation and never freed, and a
        // record is fully initialised before it is published to `HEAD`
        // (SeqCst CAS in `acquire_record`, SeqCst/Acquire loads here).
        let record: &'static Record = unsafe { &*cursor };
        f(record);
        cursor = record.next.load(Ordering::Acquire);
    }
}

/// Reuses a released record or appends a new one.
#[cold]
fn acquire_record() -> *const Record {
    let mut reused: *const Record = ptr::null();
    for_each_record(|record| {
        if reused.is_null()
            && record
                .in_use
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
        {
            reused = record;
        }
    });
    if !reused.is_null() {
        return reused;
    }
    let record: &'static Record = Box::leak(Box::new(Record {
        slots: [const { AtomicPtr::new(ptr::null_mut()) }; SLOTS],
        in_use: AtomicBool::new(true),
        next: AtomicPtr::new(ptr::null_mut()),
    }));
    let raw = ptr::from_ref(record).cast_mut();
    let mut head = HEAD.load(Ordering::Acquire);
    loop {
        record.next.store(head, Ordering::Relaxed);
        // SeqCst: see `for_each_record` for why publication must take part
        // in the SeqCst total order.
        match HEAD.compare_exchange_weak(head, raw, Ordering::SeqCst, Ordering::Acquire) {
            Ok(_) => return raw,
            Err(actual) => head = actual,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn pin_publishes_and_clears() {
        let target: *const () = core::ptr::without_provenance(0x1000);
        {
            let _pin = pin(target).expect("slot available");
            assert!(this_thread_protects(target));
            assert!(announced().contains(&target));
        }
        assert!(!this_thread_protects(target));
    }

    #[test]
    fn nested_pins_use_distinct_slots_until_exhausted() {
        let pins: Vec<Pin> = (0..SLOTS)
            .map(|i| pin(core::ptr::without_provenance(0x2000 + i * 8)).expect("slot"))
            .collect();
        assert!(
            pin(core::ptr::without_provenance(0x2100)).is_none(),
            "all slots taken"
        );
        drop(pins);
        assert!(pin(core::ptr::without_provenance(0x2100)).is_some());
    }

    #[test]
    fn others_protect_sees_other_threads_only() {
        let target: *const () = core::ptr::without_provenance(0x3000);
        let (tx, rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let worker = std::thread::spawn(move || {
            let _pin = pin(core::ptr::without_provenance(0x3000)).expect("slot");
            tx.send(()).unwrap();
            done_rx.recv().unwrap();
        });
        rx.recv().unwrap();
        assert!(others_protect(target));
        let own = pin(core::ptr::without_provenance(0x3008)).expect("slot");
        assert!(!others_protect(core::ptr::without_provenance(0x3008)));
        drop(own);
        done_tx.send(()).unwrap();
        worker.join().unwrap();
        assert!(!others_protect(target), "slot cleared at unpin");
    }

    #[test]
    fn records_are_reused_after_thread_exit() {
        let count = || {
            let mut n = 0;
            for_each_record(|_| n += 1);
            n
        };
        for _ in 0..4 {
            std::thread::spawn(|| drop(pin(core::ptr::without_provenance(0x10))))
                .join()
                .unwrap();
        }
        let before = count();
        for _ in 0..32 {
            std::thread::spawn(|| drop(pin(core::ptr::without_provenance(0x10))))
                .join()
                .unwrap();
        }
        // Without reuse the list would grow by exactly 32. Other tests in
        // this process may add a few records concurrently, so the bound is
        // "fewer than one per thread" rather than "zero".
        assert!(
            count() < before + 32,
            "sequential threads must reuse records"
        );
    }
}

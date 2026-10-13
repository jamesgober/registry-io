//! An atomically replaceable, hazard-protected shared value.
//!
//! [`SnapCell<T>`] holds one `Arc<T>` that many threads read concurrently
//! and a few threads occasionally replace. It is the storage behind both
//! registries' handler lists and replaces the `arc-swap` dependency used
//! before 1.1.
//!
//! - **Readers** ([`SnapCell::read`]) publish a hazard announcement,
//!   validate it, and borrow the value in place: no reference-count
//!   traffic, no lock, no allocation.
//! - **Writers** ([`SnapCell::update`], [`SnapCell::replace`]) serialize
//!   on a mutex, install the new value, and *retire* the old one. A retired
//!   value is freed as soon as no hazard announces it, either by the
//!   writer immediately or by the last reader that was still using it.
//!
//! Writers never wait for readers, so a handler that unregisters itself
//! (or anything else) from inside a dispatch cannot deadlock.
//! [`SnapCell::wait_until_unused`] is the opt-in exception that does wait.

use core::cell::Cell;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::hazard;

std::thread_local! {
    /// How many fallback reads the current thread is inside, across all
    /// cells. See [`SnapCell::wait_until_unused`].
    static FALLBACK_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// A shared `Arc<T>` with lock-free, allocation-free reads.
pub(crate) struct SnapCell<T> {
    /// The current value, as produced by `Arc::into_raw`. Never null.
    current: AtomicPtr<T>,
    /// Number of retired values not yet freed. Read on every dispatch so
    /// the last reader of a retired value can free it promptly.
    retired_len: AtomicUsize,
    /// Readers currently inside the locked fallback path (no hazard
    /// slot). They hold an owned `Arc` instead of an announcement, so
    /// [`wait_until_unused`](Self::wait_until_unused) counts them here.
    fallback_readers: AtomicUsize,
    /// Serializes writers and holds the retired values.
    writer: Mutex<Vec<Arc<T>>>,
}

// SAFETY: `SnapCell<T>` hands out `&T` to any thread and moves `Arc<T>`
// between threads, exactly like `Arc<T>` itself, so it is `Send` and
// `Sync` under the same bounds `Arc<T>` requires.
unsafe impl<T: Send + Sync> Send for SnapCell<T> {}
// SAFETY: see the `Send` impl above.
unsafe impl<T: Send + Sync> Sync for SnapCell<T> {}

impl<T> SnapCell<T> {
    pub(crate) fn new(value: T) -> Self {
        Self {
            current: AtomicPtr::new(Arc::into_raw(Arc::new(value)).cast_mut()),
            retired_len: AtomicUsize::new(0),
            fallback_readers: AtomicUsize::new(0),
            writer: Mutex::new(Vec::new()),
        }
    }

    /// Borrows the current value for the duration of `f`.
    ///
    /// Lock-free and allocation-free on the common path. When the calling
    /// thread has no free hazard slot (dispatch nested more than
    /// [`hazard::SLOTS`] deep, or thread-local storage already torn down),
    /// falls back to cloning the `Arc` under the writer lock.
    #[inline]
    pub(crate) fn read<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let Some((pin, p)) = self.protect() else {
            return self.read_fallback(f);
        };
        // SAFETY: `protect` returned `p` validated under a published
        // hazard (see its documentation), so the value stays alive and
        // unaliased-by-writers until `pin` is dropped below.
        let out = f(unsafe { &*p });
        self.release(pin);
        out
    }

    /// Publishes a hazard for the current value and validates it.
    ///
    /// On success the returned pointer came from `Arc::into_raw` (with
    /// provenance over the whole `Arc` allocation) and was still `current`
    /// after the announcement became visible: a `SeqCst` store followed by a
    /// `SeqCst` load. Any writer that replaces it afterwards scans the
    /// announcements with `SeqCst` loads, sees this one, and keeps the value
    /// alive until the returned [`hazard::Pin`] is dropped.
    ///
    /// `None` when the thread has no free hazard slot.
    #[inline]
    fn protect(&self) -> Option<(hazard::Pin, *mut T)> {
        let mut p = self.current.load(Ordering::Acquire);
        let pin = hazard::pin(p.cast_const().cast())?;
        loop {
            let q = self.current.load(Ordering::SeqCst);
            if q == p {
                return Some((pin, p));
            }
            p = q;
            pin.protect(p.cast_const().cast());
        }
    }

    /// Drops the hazard, then frees retired values if this was the last
    /// reader holding one.
    #[inline]
    fn release(&self, pin: hazard::Pin) {
        drop(pin);
        if self.retired_len.load(Ordering::Relaxed) != 0 {
            self.try_reclaim();
        }
    }

    /// Returns an owned handle to the current value.
    ///
    /// Use this when the value must outlive a single synchronous call,
    /// for example across an `.await`, where a thread-bound hazard slot
    /// cannot be held.
    #[cfg(test)]
    pub(crate) fn load_arc(&self) -> Arc<T> {
        // `|_| true` makes the result always `Some`; the fallback below is
        // only for the impossible `None`.
        self.load_arc_if(|_| true)
            .unwrap_or_else(|| self.load_arc_locked())
    }

    /// Like [`load_arc`](Self::load_arc), but returns `None` without
    /// touching the reference count when `keep` rejects the current value.
    /// Lets a caller skip the clone entirely for, say, an empty list.
    #[cfg(any(test, feature = "async"))]
    pub(crate) fn load_arc_if(&self, keep: impl FnOnce(&T) -> bool) -> Option<Arc<T>> {
        let Some((pin, p)) = self.protect() else {
            let arc = self.load_arc_locked();
            return keep(&arc).then_some(arc);
        };
        // SAFETY: `protect` validated `p` under a published hazard.
        if !keep(unsafe { &*p }) {
            self.release(pin);
            return None;
        }
        // SAFETY: `p` is the pointer `Arc::into_raw` produced (not one
        // re-derived from a `&T`, whose provenance would not cover the
        // reference counts), and the hazard keeps the allocation alive, so
        // its strong count is at least one while we add ours.
        let arc = unsafe {
            Arc::increment_strong_count(p);
            Arc::from_raw(p)
        };
        self.release(pin);
        Some(arc)
    }

    /// [`read`](Self::read) without a hazard slot: hold an owned `Arc`
    /// instead, and count this reader so waiters can see it.
    #[cold]
    #[inline(never)]
    fn read_fallback<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        struct Active<'a>(&'a AtomicUsize);
        impl Drop for Active<'_> {
            fn drop(&mut self) {
                let _previous = self.0.fetch_sub(1, Ordering::SeqCst);
                // Fails only during thread-local teardown, when the depth
                // no longer matters to anyone.
                let _gone = FALLBACK_DEPTH.try_with(|d| d.set(d.get().saturating_sub(1)));
            }
        }
        let _previous = self.fallback_readers.fetch_add(1, Ordering::SeqCst);
        // See the matching decrement in `Active::drop`.
        let _gone = FALLBACK_DEPTH.try_with(|d| d.set(d.get().saturating_add(1)));
        let _active = Active(&self.fallback_readers);
        let value = self.load_arc_locked();
        f(&value)
    }

    /// Clones the current `Arc` under the writer lock.
    #[cold]
    fn load_arc_locked(&self) -> Arc<T> {
        let _writer = self.lock();
        let raw = self.current.load(Ordering::Acquire);
        // SAFETY: `current` is only replaced while the writer lock is
        // held, and the value it points to is kept alive at least until
        // it is retired under that lock, so it is valid while we hold it.
        unsafe {
            Arc::increment_strong_count(raw);
            Arc::from_raw(raw)
        }
    }

    /// Replaces the value with one derived from it.
    ///
    /// `f` sees the current value and returns `None` to leave it alone or
    /// `Some((new, result))` to install `new`. Writers are serialized, so
    /// `f` runs exactly once and sees the latest value. Values freed by
    /// this call are dropped after the lock is released, so their `Drop`
    /// impls may safely call back into the cell.
    pub(crate) fn update<R>(&self, f: impl FnOnce(&T) -> Option<(T, R)>) -> Option<R> {
        let mut writer = self.lock();
        let raw = self.current.load(Ordering::Acquire);
        // SAFETY: while the writer lock is held `current` cannot be
        // retired, so the value is alive.
        let (new, result) = f(unsafe { &*raw })?;
        let new = Arc::into_raw(Arc::new(new)).cast_mut();
        self.current.store(new, Ordering::SeqCst);
        // SAFETY: `raw` was produced by `Arc::into_raw` and has just been
        // unlinked; this reclaims the strong reference `current` owned.
        writer.push(unsafe { Arc::from_raw(raw) });
        let freed = Self::collect_unprotected(&mut writer);
        self.retired_len.store(writer.len(), Ordering::Relaxed);
        drop(writer);
        drop(freed);
        Some(result)
    }

    /// Installs `value` unconditionally.
    #[cfg(test)]
    pub(crate) fn replace(&self, value: T) {
        let _unit = self.update(|_| Some((value, ())));
    }

    /// Blocks until no thread other than the caller is still reading a
    /// value this cell has replaced.
    ///
    /// Call it right after `update` or `replace`. Every value that was
    /// current before that call is either already freed or sits in the
    /// retired list; this waits until none of the retired values is
    /// announced by another thread. Readers that start afterwards see the
    /// new value and are not waited for, but if more writes happen while
    /// waiting, readers of those newer retired values are waited for too,
    /// so the wait is conservative, never short.
    ///
    /// Dispatches running on the calling thread are never waited for,
    /// since the caller is running inside them. Readers on the locked
    /// fallback path are tracked per cell; when the caller is itself on a
    /// fallback path they are not waited for, because they may include
    /// the caller.
    ///
    /// Waiting spins briefly, then yields, then sleeps with a growing
    /// backoff capped at one millisecond.
    #[cfg(any(test, feature = "sync"))]
    pub(crate) fn wait_for_retired_readers(&self) {
        let caller_on_fallback = FALLBACK_DEPTH.try_with(Cell::get).unwrap_or(0) > 0;
        let mut round = 0u32;
        loop {
            let fallback_clear =
                caller_on_fallback || self.fallback_readers.load(Ordering::SeqCst) == 0;
            if fallback_clear {
                let retired: Vec<*const ()> = self
                    .lock()
                    .iter()
                    .map(|r| Arc::as_ptr(r).cast::<()>())
                    .collect();
                if retired.iter().all(|p| !hazard::others_protect(*p)) {
                    return;
                }
            }
            backoff(&mut round);
        }
    }

    /// Frees retired values no hazard announces. Called by readers after
    /// they release their hazard; never blocks.
    #[cold]
    fn try_reclaim(&self) {
        let Ok(mut writer) = self.writer.try_lock() else {
            return;
        };
        let freed = Self::collect_unprotected(&mut writer);
        self.retired_len.store(writer.len(), Ordering::Relaxed);
        drop(writer);
        drop(freed);
    }

    /// Moves every retired value that no thread announces out of the
    /// list, for the caller to drop after releasing the lock.
    fn collect_unprotected(retired: &mut Vec<Arc<T>>) -> Vec<Arc<T>> {
        if retired.is_empty() {
            return Vec::new();
        }
        let announced = hazard::announced();
        let mut freed = Vec::new();
        let mut i = 0;
        while i < retired.len() {
            let p: *const () = Arc::as_ptr(&retired[i]).cast();
            if announced.contains(&p) {
                i += 1;
            } else {
                freed.push(retired.swap_remove(i));
            }
        }
        freed
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Arc<T>>> {
        // Poisoning can only come from a panic while the lock is held,
        // and nothing that runs under it can leave the retired list
        // inconsistent (user `Drop` code runs after unlocking), so the
        // data is sound to keep using.
        self.writer.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<T> Drop for SnapCell<T> {
    fn drop(&mut self) {
        let raw = *self.current.get_mut();
        // SAFETY: `&mut self` means no reader can be inside `read` on
        // this cell, so nothing else references the current value through
        // it; `raw` came from `Arc::into_raw` and is released exactly
        // once here. Retired values drop with the `Vec`.
        drop(unsafe { Arc::from_raw(raw) });
    }
}

#[cfg(any(test, feature = "sync"))]
fn backoff(round: &mut u32) {
    *round = round.saturating_add(1);
    if *round < 64 {
        core::hint::spin_loop();
    } else if *round < 128 {
        std::thread::yield_now();
    } else {
        let micros = u64::from((*round - 127).min(1000));
        std::thread::sleep(std::time::Duration::from_micros(micros));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    struct DropFlag(Arc<AtomicBool>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn read_sees_latest_value() {
        let cell = SnapCell::new(1u32);
        assert_eq!(cell.read(|v| *v), 1);
        cell.replace(2);
        assert_eq!(cell.read(|v| *v), 2);
    }

    #[test]
    fn update_none_leaves_value_and_allocates_nothing() {
        let cell = SnapCell::new(5u32);
        let before = cell.load_arc();
        assert_eq!(cell.update(|_| None::<(u32, ())>), None);
        assert!(Arc::ptr_eq(&before, &cell.load_arc()));
    }

    #[test]
    fn replaced_value_is_freed_immediately_when_unread() {
        let dropped = Arc::new(AtomicBool::new(false));
        let cell = SnapCell::new(Some(DropFlag(Arc::clone(&dropped))));
        cell.replace(None);
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(cell.retired_len.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn value_read_during_replace_survives_until_reader_finishes() {
        let dropped = Arc::new(AtomicBool::new(false));
        let cell = SnapCell::new(Some(DropFlag(Arc::clone(&dropped))));
        cell.read(|v| {
            assert!(v.is_some());
            cell.replace(None);
            // Still protected by this reader's hazard.
            assert!(!dropped.load(Ordering::SeqCst));
            assert!(v.is_some());
        });
        // The reader freed it on the way out.
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn load_arc_keeps_value_alive_after_replace() {
        let dropped = Arc::new(AtomicBool::new(false));
        let cell = SnapCell::new(Some(DropFlag(Arc::clone(&dropped))));
        let held = cell.load_arc();
        cell.replace(None);
        assert!(!dropped.load(Ordering::SeqCst));
        drop(held);
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn nested_reads_beyond_slot_count_use_fallback() {
        fn nest(cell: &SnapCell<u32>, depth: usize) -> u32 {
            if depth == 0 {
                return cell.read(|v| *v);
            }
            cell.read(|v| *v + nest(cell, depth - 1))
        }
        let cell = SnapCell::new(1u32);
        let expected = u32::try_from(hazard::SLOTS * 2 + 1).unwrap();
        assert_eq!(nest(&cell, hazard::SLOTS * 2), expected);
    }

    #[test]
    fn wait_for_retired_readers_returns_after_other_reader_leaves() {
        let cell = Arc::new(SnapCell::new(0u32));
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let release = Arc::new(AtomicBool::new(false));
        let reader = {
            let (cell, release) = (Arc::clone(&cell), Arc::clone(&release));
            std::thread::spawn(move || {
                cell.read(|_| {
                    entered_tx.send(()).unwrap();
                    while !release.load(Ordering::SeqCst) {
                        std::thread::yield_now();
                    }
                });
            })
        };
        entered_rx.recv().unwrap();
        cell.replace(1);
        let waiter = {
            let cell = Arc::clone(&cell);
            std::thread::spawn(move || cell.wait_for_retired_readers())
        };
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(!waiter.is_finished(), "must wait for the in-flight reader");
        release.store(true, Ordering::SeqCst);
        waiter.join().unwrap();
        reader.join().unwrap();
    }

    #[test]
    fn wait_for_retired_readers_skips_own_thread() {
        let cell = SnapCell::new(0u32);
        cell.read(|_| {
            cell.replace(1);
            // Must return even though this thread still reads the old value.
            cell.wait_for_retired_readers();
        });
    }

    #[test]
    fn wait_for_retired_readers_covers_older_generations() {
        let cell = Arc::new(SnapCell::new(0u32));
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let release = Arc::new(AtomicBool::new(false));
        let reader = {
            let (cell, release) = (Arc::clone(&cell), Arc::clone(&release));
            std::thread::spawn(move || {
                cell.read(|_| {
                    entered_tx.send(()).unwrap();
                    while !release.load(Ordering::SeqCst) {
                        std::thread::yield_now();
                    }
                });
            })
        };
        entered_rx.recv().unwrap();
        // The reader holds generation 0; two later writes retire 0 and 1.
        cell.replace(1);
        cell.replace(2);
        let waiter = {
            let cell = Arc::clone(&cell);
            std::thread::spawn(move || cell.wait_for_retired_readers())
        };
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(
            !waiter.is_finished(),
            "a reader of an older generation must be waited for"
        );
        release.store(true, Ordering::SeqCst);
        waiter.join().unwrap();
        reader.join().unwrap();
    }

    #[test]
    fn concurrent_readers_and_writers_never_observe_freed_values() {
        const READERS: usize = 4;
        let cell = Arc::new(SnapCell::new(vec![0u64; 16]));
        let stop = Arc::new(AtomicBool::new(false));
        // Each reader bumps `started` after its first read and
        // `overlapped` after its first read that saw a written value, so
        // the writer can guarantee real overlap however slowly the OS
        // schedules reader threads (CI runners may have two cores).
        let started = Arc::new(AtomicUsize::new(0));
        let overlapped = Arc::new(AtomicUsize::new(0));
        let readers: Vec<_> = (0..READERS)
            .map(|_| {
                let (cell, stop) = (Arc::clone(&cell), Arc::clone(&stop));
                let (started, overlapped) = (Arc::clone(&started), Arc::clone(&overlapped));
                std::thread::spawn(move || {
                    let mut reads = 0u64;
                    let mut saw_write = false;
                    while !stop.load(Ordering::SeqCst) {
                        let first = cell.read(|v| {
                            let first = v[0];
                            assert!(v.iter().all(|x| *x == first), "torn snapshot");
                            first
                        });
                        if reads == 0 {
                            let _ = started.fetch_add(1, Ordering::SeqCst);
                        }
                        if first != 0 && !saw_write {
                            saw_write = true;
                            let _ = overlapped.fetch_add(1, Ordering::SeqCst);
                        }
                        reads += 1;
                    }
                    reads
                })
            })
            .collect();
        while started.load(Ordering::SeqCst) < READERS {
            std::thread::yield_now();
        }
        let min_writes = if cfg!(miri) { 40 } else { 2000u64 };
        let mut i = 0u64;
        while i < min_writes || overlapped.load(Ordering::SeqCst) < READERS {
            i += 1;
            cell.replace(vec![i; 16]);
        }
        stop.store(true, Ordering::SeqCst);
        for r in readers {
            assert!(r.join().unwrap() > 0);
        }
        cell.read(|v| assert_eq!(v[0], i));
    }
}

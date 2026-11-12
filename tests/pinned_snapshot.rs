//! `notify` stays allocation-free, and cheap, while another thread is
//! still inside a dispatch of a handler list that a write has replaced.
//!
//! Review finding F-2 (1.1.0): one slow handler plus one `register` made
//! every `notify` on every thread take the writer try-lock, scan every
//! hazard record and allocate, until the slow handler returned (8 ns per
//! notify became 218 to 370 ns).
//!
//! This binary installs a counting global allocator that counts only on
//! the measuring thread, so other tests and the harness do not disturb it.

#![cfg(feature = "sync")]
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use registry_io::SyncRegistry;

struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

std::thread_local! {
    static COUNT_HERE: Cell<bool> = const { Cell::new(false) };
}

// SAFETY: forwards every call to `System` unchanged; it only counts.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNT_HERE.try_with(Cell::get).unwrap_or(false) {
            let _previous = ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller upholds `GlobalAlloc::dealloc`'s contract.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn count_allocations(body: impl FnOnce()) -> usize {
    ALLOCS.store(0, Ordering::SeqCst);
    COUNT_HERE.with(|c| c.set(true));
    body();
    COUNT_HERE.with(|c| c.set(false));
    ALLOCS.load(Ordering::SeqCst)
}

#[test]
fn notify_does_not_allocate_while_a_replaced_list_is_still_in_use() {
    let registry: Arc<SyncRegistry<u32>> = Arc::new(SyncRegistry::new());
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    {
        let (entered, release) = (Arc::clone(&entered), Arc::clone(&release));
        let _id = registry.register(move |v| {
            if *v == 1 {
                entered.store(true, Ordering::SeqCst);
                while !release.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        });
    }

    // Warm up this thread's hazard record.
    for _ in 0..1000 {
        registry.notify(&0);
    }
    assert_eq!(count_allocations(|| registry.notify(&0)), 0);

    let blocker = {
        let registry = Arc::clone(&registry);
        std::thread::spawn(move || registry.notify(&1))
    };
    while !entered.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }
    // Retires the list the blocker is still dispatching.
    let extra = registry.register(|_| {});

    let allocs = count_allocations(|| {
        for _ in 0..100_000 {
            registry.notify(&0);
        }
    });
    release.store(true, Ordering::SeqCst);
    blocker.join().unwrap();
    assert_eq!(
        allocs, 0,
        "notify allocated while a retired list was pinned"
    );
    assert!(registry.unregister(extra));
}

/// The retired list is still freed once the slow dispatch returns: the
/// thread that was reading it frees it on the way out.
#[test]
fn replaced_list_is_freed_when_the_slow_dispatch_returns() {
    struct Flag(Arc<AtomicUsize>);
    impl Drop for Flag {
        fn drop(&mut self) {
            let _previous = self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let registry: Arc<SyncRegistry<u32>> = Arc::new(SyncRegistry::new());
    let dropped = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let slow = {
        let (entered, release) = (Arc::clone(&entered), Arc::clone(&release));
        registry.register(move |v| {
            if *v == 1 {
                entered.store(true, Ordering::SeqCst);
                while !release.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        })
    };
    let flag = Flag(Arc::clone(&dropped));
    let doomed = registry.register(move |_| {
        let _keep = &flag;
    });
    let blocker = {
        let registry = Arc::clone(&registry);
        std::thread::spawn(move || registry.notify(&1))
    };
    while !entered.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }
    assert!(registry.unregister(doomed));
    for _ in 0..1000 {
        registry.notify(&0);
    }
    assert_eq!(dropped.load(Ordering::SeqCst), 0, "freed while in use");
    release.store(true, Ordering::SeqCst);
    blocker.join().unwrap();
    assert_eq!(dropped.load(Ordering::SeqCst), 1, "never freed");
    assert!(registry.unregister(slow));
}

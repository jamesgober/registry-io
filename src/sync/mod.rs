//! Synchronous, lock-free event registry.
//!
//! [`SyncRegistry`] is the workhorse of this crate. It stores a list of
//! handler closures keyed by [`HandlerId`] and dispatches an event to every
//! handler when [`SyncRegistry::notify`] is called. The notify path is
//! lock-free, allocation-free, and panic-isolating.

#![cfg(feature = "sync")]

use core::any::Any;
use core::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use arc_swap::{ArcSwap, ArcSwapOption};

use crate::HandlerId;
use crate::handler_id::HandlerIdGenerator;
use crate::panic::{PanicCallbackHolder, PanicInfo};

pub mod guard;
pub use guard::HandlerGuard;

/// A boxed handler closure stored inside the registry.
type StoredHandler<E> = Arc<dyn Fn(&E) + Send + Sync + 'static>;

/// One entry in the handler list: an id, a priority value, and the closure.
///
/// Cloneable via [`Arc`] refcount bumps; cloning the entire list during
/// register/unregister therefore costs N refcount increments and one
/// allocation for the new `Vec`, not N allocations for the handlers.
struct HandlerEntry<E: Send + Sync + 'static> {
    id: HandlerId,
    priority: i32,
    handler: StoredHandler<E>,
}

impl<E: Send + Sync + 'static> Clone for HandlerEntry<E> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            priority: self.priority,
            handler: Arc::clone(&self.handler),
        }
    }
}

impl<E: Send + Sync + 'static> fmt::Debug for HandlerEntry<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HandlerEntry")
            .field("id", &self.id)
            .field("priority", &self.priority)
            .finish_non_exhaustive()
    }
}

/// Synchronous event registry.
///
/// Hosts a set of closure handlers and dispatches an event reference to every
/// handler on [`SyncRegistry::notify`]. Reads are lock-free; the hot path is
/// allocation-free; panics in handlers are caught and isolated so that one
/// misbehaving handler cannot prevent its siblings from running.
///
/// # Type parameter
///
/// `E` is the event type. Handlers receive `&E` (a borrow) so events do not
/// need to be `Clone`. The bound `E: Send + Sync + 'static` is required for
/// the registry itself to be `Send + Sync`.
///
/// # Cost model
///
/// - `notify`: lock-free, allocation-free, walks an array of `Arc` pointers
///   and calls each one through dynamic dispatch. Each call is wrapped in
///   [`catch_unwind`].
/// - `register` / `unregister` / `clear`: slow path. Clones the current
///   handler list, mutates, and atomically swaps. Cost is `O(N)` in the
///   number of currently-registered handlers.
///
/// # Concurrency
///
/// All methods can be called from any thread concurrently. Many threads may
/// fire `notify` in parallel without serialization. Concurrent
/// `register`/`unregister` calls coordinate via an atomic compare-and-swap
/// retry loop (see [`arc_swap::ArcSwap::rcu`]).
///
/// # Examples
///
/// Basic registration and dispatch:
///
/// ```
/// use registry_io::SyncRegistry;
/// use std::sync::atomic::{AtomicU32, Ordering};
/// use std::sync::Arc;
///
/// #[derive(Debug)]
/// struct Tick(u32);
///
/// let registry: SyncRegistry<Tick> = SyncRegistry::new();
/// let counter = Arc::new(AtomicU32::new(0));
/// let sink = Arc::clone(&counter);
///
/// let id = registry.register(move |tick| {
///     sink.fetch_add(tick.0, Ordering::Relaxed);
/// });
///
/// registry.notify(&Tick(2));
/// registry.notify(&Tick(3));
/// assert_eq!(counter.load(Ordering::Relaxed), 5);
///
/// assert!(registry.unregister(id));
/// registry.notify(&Tick(100));
/// assert_eq!(counter.load(Ordering::Relaxed), 5);
/// ```
pub struct SyncRegistry<E: Send + Sync + 'static> {
    handlers: ArcSwap<Vec<HandlerEntry<E>>>,
    id_generator: HandlerIdGenerator,
    panic_callback: ArcSwapOption<PanicCallbackHolder>,
}

impl<E: Send + Sync + 'static> SyncRegistry<E> {
    /// Create a new, empty registry.
    ///
    /// # Examples
    ///
    /// ```
    /// use registry_io::SyncRegistry;
    ///
    /// let registry: SyncRegistry<&'static str> = SyncRegistry::new();
    /// assert_eq!(registry.handler_count(), 0);
    /// assert!(registry.is_empty());
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            handlers: ArcSwap::from_pointee(Vec::new()),
            id_generator: HandlerIdGenerator::new(),
            panic_callback: ArcSwapOption::empty(),
        }
    }

    /// Create a new, empty registry that pre-allocates room for `capacity`
    /// handlers.
    ///
    /// This is a slow-path optimization: it does not change the cost of
    /// `notify`. Useful when the expected steady-state handler count is
    /// known up front, to avoid intermediate allocations as the list grows.
    ///
    /// # Examples
    ///
    /// ```
    /// use registry_io::SyncRegistry;
    ///
    /// let registry: SyncRegistry<u64> = SyncRegistry::with_capacity(32);
    /// assert!(registry.is_empty());
    /// ```
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            handlers: ArcSwap::from_pointee(Vec::with_capacity(capacity)),
            id_generator: HandlerIdGenerator::new(),
            panic_callback: ArcSwapOption::empty(),
        }
    }

    /// Register a handler at the default priority (`0`) and return its id.
    ///
    /// The handler will fire on every subsequent [`SyncRegistry::notify`]
    /// call until [`SyncRegistry::unregister`] is invoked with the returned
    /// id (or [`SyncRegistry::clear`] removes all handlers).
    ///
    /// # Examples
    ///
    /// ```
    /// use registry_io::SyncRegistry;
    ///
    /// let registry: SyncRegistry<i32> = SyncRegistry::new();
    /// let id = registry.register(|n| {
    ///     println!("value: {n}");
    /// });
    /// assert_eq!(registry.handler_count(), 1);
    /// assert!(registry.unregister(id));
    /// ```
    pub fn register<F>(&self, handler: F) -> HandlerId
    where
        F: Fn(&E) + Send + Sync + 'static,
    {
        self.register_with_priority(0, handler)
    }

    /// Register a handler with an explicit priority value.
    ///
    /// On `notify`, handlers fire in **descending priority order**. Handlers
    /// with the same priority fire in **registration order** (FIFO).
    ///
    /// The default priority used by [`SyncRegistry::register`] is `0`. Use
    /// positive values to fire before defaults and negative values to fire
    /// after.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::{Arc, Mutex};
    /// use registry_io::SyncRegistry;
    ///
    /// let registry: SyncRegistry<()> = SyncRegistry::new();
    /// let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    ///
    /// let s = Arc::clone(&order);
    /// let _ = registry.register_with_priority(0, move |_| s.lock().unwrap().push("mid"));
    /// let s = Arc::clone(&order);
    /// let _ = registry.register_with_priority(10, move |_| s.lock().unwrap().push("first"));
    /// let s = Arc::clone(&order);
    /// let _ = registry.register_with_priority(-5, move |_| s.lock().unwrap().push("last"));
    ///
    /// registry.notify(&());
    /// assert_eq!(order.lock().unwrap().as_slice(), &["first", "mid", "last"]);
    /// ```
    pub fn register_with_priority<F>(&self, priority: i32, handler: F) -> HandlerId
    where
        F: Fn(&E) + Send + Sync + 'static,
    {
        let id = self.id_generator.next();
        let entry = HandlerEntry {
            id,
            priority,
            handler: Arc::new(handler),
        };
        drop(self.handlers.rcu(|current| {
            let mut new_vec: Vec<HandlerEntry<E>> = Vec::with_capacity(current.len() + 1);
            new_vec.extend(current.iter().cloned());
            let pos = new_vec.partition_point(|e| e.priority >= entry.priority);
            new_vec.insert(pos, entry.clone());
            Arc::new(new_vec)
        }));
        id
    }

    /// Register a handler and return a RAII [`HandlerGuard`] that
    /// automatically unregisters when dropped.
    ///
    /// The registry must be wrapped in an [`Arc`] so the guard can hold a
    /// weak reference. If the registry is dropped before the guard, the
    /// guard's `Drop` impl becomes a no-op.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use registry_io::SyncRegistry;
    ///
    /// let registry = Arc::new(SyncRegistry::<u32>::new());
    /// {
    ///     let _guard = registry.register_guard(|n| {
    ///         println!("got {n}");
    ///     });
    ///     assert_eq!(registry.handler_count(), 1);
    /// }
    /// assert_eq!(registry.handler_count(), 0);
    /// ```
    pub fn register_guard<F>(self: &Arc<Self>, handler: F) -> HandlerGuard<E>
    where
        F: Fn(&E) + Send + Sync + 'static,
    {
        let id = self.register(handler);
        HandlerGuard::new(id, Arc::downgrade(self))
    }

    /// Like [`SyncRegistry::register_guard`] but with an explicit priority.
    ///
    /// See [`SyncRegistry::register_with_priority`] for the ordering rules.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use registry_io::SyncRegistry;
    ///
    /// let registry = Arc::new(SyncRegistry::<&'static str>::new());
    /// let guard = registry.register_guard_with_priority(100, |s| {
    ///     println!("high priority: {s}");
    /// });
    /// assert_eq!(registry.handler_count(), 1);
    /// drop(guard);
    /// assert_eq!(registry.handler_count(), 0);
    /// ```
    pub fn register_guard_with_priority<F>(
        self: &Arc<Self>,
        priority: i32,
        handler: F,
    ) -> HandlerGuard<E>
    where
        F: Fn(&E) + Send + Sync + 'static,
    {
        let id = self.register_with_priority(priority, handler);
        HandlerGuard::new(id, Arc::downgrade(self))
    }

    /// Unregister a handler by id.
    ///
    /// Returns `true` if a handler with that id was found and removed,
    /// `false` if no such handler is currently registered. Repeated calls
    /// with the same id after the first successful one will return `false`.
    ///
    /// # Examples
    ///
    /// ```
    /// use registry_io::SyncRegistry;
    ///
    /// let registry: SyncRegistry<()> = SyncRegistry::new();
    /// let id = registry.register(|_| {});
    /// assert!(registry.unregister(id));
    /// assert!(!registry.unregister(id));
    /// ```
    pub fn unregister(&self, id: HandlerId) -> bool {
        let mut removed = false;
        drop(self.handlers.rcu(|current| {
            let mut new_vec: Vec<HandlerEntry<E>> = Vec::with_capacity(current.len());
            new_vec.extend(current.iter().filter(|e| e.id != id).cloned());
            removed = new_vec.len() != current.len();
            Arc::new(new_vec)
        }));
        removed
    }

    /// Remove every registered handler.
    ///
    /// In-flight `notify` calls that loaded the snapshot before `clear` was
    /// invoked will still iterate over their snapshot to completion.
    ///
    /// # Examples
    ///
    /// ```
    /// use registry_io::SyncRegistry;
    ///
    /// let registry: SyncRegistry<u8> = SyncRegistry::new();
    /// let _ = registry.register(|_| {});
    /// let _ = registry.register(|_| {});
    /// assert_eq!(registry.handler_count(), 2);
    ///
    /// registry.clear();
    /// assert_eq!(registry.handler_count(), 0);
    /// ```
    pub fn clear(&self) {
        self.handlers.store(Arc::new(Vec::new()));
    }

    /// Number of currently registered handlers.
    ///
    /// This is a `O(1)` snapshot read. The value may already be stale by
    /// the time the caller observes it if another thread is concurrently
    /// registering or unregistering.
    ///
    /// # Examples
    ///
    /// ```
    /// use registry_io::SyncRegistry;
    ///
    /// let registry: SyncRegistry<()> = SyncRegistry::new();
    /// assert_eq!(registry.handler_count(), 0);
    /// let _ = registry.register(|_| {});
    /// assert_eq!(registry.handler_count(), 1);
    /// ```
    #[inline]
    #[must_use]
    pub fn handler_count(&self) -> usize {
        self.handlers.load().len()
    }

    /// Returns `true` if no handlers are registered.
    ///
    /// Equivalent to `self.handler_count() == 0`.
    ///
    /// # Examples
    ///
    /// ```
    /// use registry_io::SyncRegistry;
    ///
    /// let registry: SyncRegistry<()> = SyncRegistry::new();
    /// assert!(registry.is_empty());
    /// let _ = registry.register(|_| {});
    /// assert!(!registry.is_empty());
    /// ```
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.handlers.load().is_empty()
    }

    /// Returns `true` if a handler with the given id is currently registered.
    ///
    /// # Examples
    ///
    /// ```
    /// use registry_io::SyncRegistry;
    ///
    /// let registry: SyncRegistry<()> = SyncRegistry::new();
    /// let id = registry.register(|_| {});
    /// assert!(registry.contains(id));
    /// assert!(registry.unregister(id));
    /// assert!(!registry.contains(id));
    /// ```
    #[must_use]
    pub fn contains(&self, id: HandlerId) -> bool {
        self.handlers.load().iter().any(|e| e.id == id)
    }

    /// Install a callback that will be invoked when a registered handler
    /// panics during `notify`.
    ///
    /// The callback runs on whichever thread invoked `notify`, immediately
    /// after the panicking handler is caught and before the next sibling
    /// handler runs. If a previous callback was set, it is replaced.
    ///
    /// Panics inside the panic callback itself are caught and silently
    /// discarded — the callback must not be relied on for further panic
    /// handling.
    ///
    /// # Default behavior
    ///
    /// Without an installed callback, a panic in a handler is **silently
    /// absorbed**: sibling handlers still fire, and `notify` returns
    /// normally. Install a callback when you want to log, count, or
    /// otherwise observe handler panics.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::atomic::{AtomicUsize, Ordering};
    /// use std::sync::Arc;
    /// use registry_io::SyncRegistry;
    ///
    /// let registry: SyncRegistry<()> = SyncRegistry::new();
    /// let panic_count = Arc::new(AtomicUsize::new(0));
    /// let sink = Arc::clone(&panic_count);
    /// registry.on_panic(move |_info| {
    ///     sink.fetch_add(1, Ordering::Relaxed);
    /// });
    ///
    /// let _ = registry.register(|_| panic!("a"));
    /// let _ = registry.register(|_| {});
    /// let _ = registry.register(|_| panic!("b"));
    /// registry.notify(&());
    ///
    /// assert_eq!(panic_count.load(Ordering::Relaxed), 2);
    /// ```
    pub fn on_panic<F>(&self, callback: F)
    where
        F: Fn(&PanicInfo<'_>) + Send + Sync + 'static,
    {
        let holder = Arc::new(PanicCallbackHolder::new(callback));
        self.panic_callback.store(Some(holder));
    }

    /// Remove any previously installed panic callback.
    ///
    /// After this returns, subsequent handler panics during `notify` are
    /// silently absorbed.
    ///
    /// # Examples
    ///
    /// ```
    /// use registry_io::SyncRegistry;
    ///
    /// let registry: SyncRegistry<()> = SyncRegistry::new();
    /// registry.on_panic(|_| {});
    /// registry.clear_panic_callback();
    /// // No callback is installed; handler panics now go to /dev/null.
    /// ```
    pub fn clear_panic_callback(&self) {
        self.panic_callback.store(None);
    }

    /// Dispatch `event` to every registered handler.
    ///
    /// Hot path: lock-free, allocation-free in the no-panic case. Handlers
    /// run inline on the calling thread, in priority order
    /// (high → low), with same-priority handlers firing in registration
    /// order.
    ///
    /// Each handler invocation is wrapped in [`catch_unwind`] so that a
    /// panic in one handler does not propagate to sibling handlers nor to
    /// the caller of `notify`. If an [`on_panic`](Self::on_panic) callback
    /// is installed, it is invoked once per panicking handler.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::atomic::{AtomicU32, Ordering};
    /// use std::sync::Arc;
    /// use registry_io::SyncRegistry;
    ///
    /// let registry: SyncRegistry<u32> = SyncRegistry::new();
    /// let total = Arc::new(AtomicU32::new(0));
    ///
    /// for _ in 0..4 {
    ///     let sink = Arc::clone(&total);
    ///     let _ = registry.register(move |n| {
    ///         sink.fetch_add(*n, Ordering::Relaxed);
    ///     });
    /// }
    ///
    /// registry.notify(&10);
    /// assert_eq!(total.load(Ordering::Relaxed), 40);
    /// ```
    ///
    /// # Performance
    ///
    /// Target: sub-50ns dispatch for small handler counts, no heap
    /// allocation. See `docs/PERFORMANCE.md` for measured numbers.
    #[inline]
    pub fn notify(&self, event: &E) {
        let snapshot = self.handlers.load();
        for entry in snapshot.iter() {
            let handler = &entry.handler;
            let result = catch_unwind(AssertUnwindSafe(|| handler(event)));
            if let Err(payload) = result {
                self.handle_panic(entry.id, payload);
            }
        }
    }

    /// Internal: invoke the installed panic callback, if any. Suppresses
    /// panics inside the callback itself.
    #[cold]
    fn handle_panic(&self, handler_id: HandlerId, payload: Box<dyn Any + Send + 'static>) {
        let guard = self.panic_callback.load();
        if let Some(holder) = guard.as_ref() {
            let info = PanicInfo::new(handler_id, payload.as_ref());
            // If the user's panic callback itself panics, swallow the
            // second-order panic. Recursion into our own machinery is
            // explicitly not supported.
            drop(catch_unwind(AssertUnwindSafe(|| holder.invoke(&info))));
        }
        drop(payload);
    }
}

impl<E: Send + Sync + 'static> Default for SyncRegistry<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: Send + Sync + 'static> fmt::Debug for SyncRegistry<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyncRegistry")
            .field("handler_count", &self.handlers.load().len())
            .field("has_panic_callback", &self.panic_callback.load().is_some())
            .finish_non_exhaustive()
    }
}

// Static assertion that `SyncRegistry<E>` is `Send + Sync` whenever the
// event type allows it. Catches regressions if someone introduces a
// non-thread-safe field by accident.
#[allow(dead_code)]
const fn _assert_send_sync<E: Send + Sync + 'static>() {
    const fn assert_send<T: Send>() {}
    const fn assert_sync<T: Sync>() {}
    assert_send::<SyncRegistry<E>>();
    assert_sync::<SyncRegistry<E>>();
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::SyncRegistry;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn empty_registry_has_no_handlers() {
        let registry: SyncRegistry<u32> = SyncRegistry::new();
        assert_eq!(registry.handler_count(), 0);
        assert!(registry.is_empty());
    }

    #[test]
    fn register_increments_count_and_returns_unique_ids() {
        let registry: SyncRegistry<u32> = SyncRegistry::new();
        let a = registry.register(|_| {});
        let b = registry.register(|_| {});
        assert_ne!(a, b);
        assert_eq!(registry.handler_count(), 2);
    }

    #[test]
    fn notify_fires_each_handler_once() {
        let registry: SyncRegistry<u32> = SyncRegistry::new();
        let count = Arc::new(AtomicU32::new(0));
        for _ in 0..3 {
            let c = Arc::clone(&count);
            let _ = registry.register(move |_| {
                let _ = c.fetch_add(1, Ordering::Relaxed);
            });
        }
        registry.notify(&7);
        assert_eq!(count.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn unregister_returns_false_for_unknown_id() {
        let registry: SyncRegistry<u32> = SyncRegistry::new();
        let id = registry.register(|_| {});
        assert!(registry.unregister(id));
        assert!(!registry.unregister(id));
    }

    #[test]
    fn clear_removes_all_handlers() {
        let registry: SyncRegistry<u32> = SyncRegistry::new();
        for _ in 0..5 {
            let _ = registry.register(|_| {});
        }
        registry.clear();
        assert_eq!(registry.handler_count(), 0);
    }

    #[test]
    fn debug_does_not_panic_and_omits_handler_internals() {
        let registry: SyncRegistry<u32> = SyncRegistry::new();
        let _ = registry.register(|_| {});
        let s = format!("{registry:?}");
        assert!(s.contains("SyncRegistry"));
        assert!(s.contains("handler_count"));
    }
}

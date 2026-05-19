//! Asynchronous, lock-free event registry.
//!
//! [`AsyncRegistry`] mirrors [`crate::SyncRegistry`] for `async fn` handlers.
//! Handlers return a future that the registry drives — either concurrently
//! via [`AsyncRegistry::notify`] or sequentially via
//! [`AsyncRegistry::notify_sequential`].
//!
//! Module gated behind the `async` feature flag.

use core::any::Any;
use core::fmt;
use core::future::Future;
use core::pin::Pin;
use std::sync::Arc;

use arc_swap::{ArcSwap, ArcSwapOption};

use crate::HandlerId;
use crate::future_ext::{CatchUnwind, JoinAll};
use crate::handler_id::HandlerIdGenerator;
use crate::panic::{PanicCallbackHolder, PanicInfo};

/// `Pin<Box<dyn Future<Output = T> + Send + 'static>>` — the type-erased
/// boxed future stored inside the registry. Defined locally rather than
/// pulled from `futures-util` to keep the dependency surface minimal.
type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

pub mod guard;
pub use guard::AsyncHandlerGuard;

/// A boxed async handler closure stored inside the registry.
///
/// The returned future is `'static` so it must not borrow from the event.
/// Handlers that need to retain event data should `.clone()` it inside the
/// closure before returning the future.
type StoredAsyncHandler<E> = Arc<dyn Fn(&E) -> BoxFuture<()> + Send + Sync + 'static>;

/// One entry in the async handler list.
struct AsyncHandlerEntry<E: Send + Sync + 'static> {
    id: HandlerId,
    priority: i32,
    handler: StoredAsyncHandler<E>,
}

impl<E: Send + Sync + 'static> Clone for AsyncHandlerEntry<E> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            priority: self.priority,
            handler: Arc::clone(&self.handler),
        }
    }
}

impl<E: Send + Sync + 'static> fmt::Debug for AsyncHandlerEntry<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AsyncHandlerEntry")
            .field("id", &self.id)
            .field("priority", &self.priority)
            .finish_non_exhaustive()
    }
}

/// Asynchronous event registry.
///
/// Same lock-free, `ArcSwap`-backed read path as [`crate::SyncRegistry`], but
/// handlers return a future of `()`. Two dispatch modes are available:
///
/// - [`AsyncRegistry::notify`] — drives every handler concurrently via a
///   crate-local `JoinAll` combinator. Faster total wall-clock for
///   handlers that perform real `.await` work, since they make progress in
///   parallel under the runtime.
/// - [`AsyncRegistry::notify_sequential`] — awaits each handler in order.
///   Use when downstream ordering or back-pressure between handlers matters.
///
/// Each handler future is wrapped in a crate-local `CatchUnwind` adapter
/// so a panic during `poll` is isolated from sibling handlers and from the
/// caller awaiting `notify`.
///
/// # Type parameter
///
/// `E` is the event type. Handlers receive `&E` but return a `'static`
/// future, so they must `clone` whatever they need from `&E` before
/// `async move { ... }`.
///
/// # Examples
///
/// ```
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() {
/// use std::sync::Arc;
/// use std::sync::atomic::{AtomicU64, Ordering};
/// use registry_io::r#async::AsyncRegistry;
///
/// let registry: AsyncRegistry<u64> = AsyncRegistry::new();
/// let total = Arc::new(AtomicU64::new(0));
///
/// let sink = Arc::clone(&total);
/// let _ = registry.register(move |value| {
///     let sink = Arc::clone(&sink);
///     let v = *value;
///     async move {
///         sink.fetch_add(v, Ordering::Relaxed);
///     }
/// });
///
/// registry.notify(&7).await;
/// assert_eq!(total.load(Ordering::Relaxed), 7);
/// # }
/// ```
pub struct AsyncRegistry<E: Send + Sync + 'static> {
    handlers: ArcSwap<Vec<AsyncHandlerEntry<E>>>,
    id_generator: HandlerIdGenerator,
    panic_callback: ArcSwapOption<PanicCallbackHolder>,
}

impl<E: Send + Sync + 'static> AsyncRegistry<E> {
    /// Create a new, empty async registry.
    ///
    /// # Examples
    ///
    /// ```
    /// use registry_io::r#async::AsyncRegistry;
    ///
    /// let registry: AsyncRegistry<u32> = AsyncRegistry::new();
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

    /// Create a new, empty async registry with pre-allocated handler
    /// capacity.
    ///
    /// # Examples
    ///
    /// ```
    /// use registry_io::r#async::AsyncRegistry;
    ///
    /// let registry: AsyncRegistry<u64> = AsyncRegistry::with_capacity(16);
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

    /// Register an async handler at the default priority (`0`).
    ///
    /// The handler is a closure `Fn(&E) -> impl Future<Output = ()>`. The
    /// returned future must be `'static`: clone any data from `&E` you need
    /// before the inner `async move { ... }`.
    ///
    /// # Examples
    ///
    /// ```
    /// use registry_io::r#async::AsyncRegistry;
    ///
    /// let registry: AsyncRegistry<String> = AsyncRegistry::new();
    /// let _ = registry.register(|event| {
    ///     let owned = event.clone();
    ///     async move {
    ///         // Pretend we awaited something useful here.
    ///         let _ = owned.len();
    ///     }
    /// });
    /// ```
    pub fn register<F, Fut>(&self, handler: F) -> HandlerId
    where
        F: Fn(&E) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.register_with_priority(0, handler)
    }

    /// Register an async handler with an explicit priority.
    ///
    /// Dispatch order at notify time follows the same rule as
    /// [`crate::SyncRegistry::register_with_priority`]: higher priority
    /// fires first, ties broken in registration order. In concurrent
    /// dispatch ([`AsyncRegistry::notify`]) priority controls the order in
    /// which futures are *spawned* into the join, not the order they
    /// complete in.
    ///
    /// # Examples
    ///
    /// ```
    /// use registry_io::r#async::AsyncRegistry;
    ///
    /// let registry: AsyncRegistry<()> = AsyncRegistry::new();
    /// let _ = registry.register_with_priority(100, |_| async move {});
    /// let _ = registry.register(|_| async move {});
    /// let _ = registry.register_with_priority(-10, |_| async move {});
    /// assert_eq!(registry.handler_count(), 3);
    /// ```
    pub fn register_with_priority<F, Fut>(&self, priority: i32, handler: F) -> HandlerId
    where
        F: Fn(&E) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let id = self.id_generator.next();
        let boxed: StoredAsyncHandler<E> = Arc::new(move |event: &E| {
            let fut = handler(event);
            Box::pin(fut) as BoxFuture<()>
        });
        let entry = AsyncHandlerEntry {
            id,
            priority,
            handler: boxed,
        };
        drop(self.handlers.rcu(|current| {
            let mut new_vec: Vec<AsyncHandlerEntry<E>> = Vec::with_capacity(current.len() + 1);
            new_vec.extend(current.iter().cloned());
            let pos = new_vec.partition_point(|e| e.priority >= entry.priority);
            new_vec.insert(pos, entry.clone());
            Arc::new(new_vec)
        }));
        id
    }

    /// Register an async handler and return a RAII
    /// [`AsyncHandlerGuard`] that auto-unregisters when dropped.
    ///
    /// Requires the registry to be wrapped in [`Arc`] so the guard can hold
    /// a [`std::sync::Weak`] reference.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use registry_io::r#async::AsyncRegistry;
    ///
    /// let registry = Arc::new(AsyncRegistry::<u32>::new());
    /// {
    ///     let _guard = registry.register_guard(|_| async move {});
    ///     assert_eq!(registry.handler_count(), 1);
    /// }
    /// assert_eq!(registry.handler_count(), 0);
    /// ```
    pub fn register_guard<F, Fut>(self: &Arc<Self>, handler: F) -> AsyncHandlerGuard<E>
    where
        F: Fn(&E) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let id = self.register(handler);
        AsyncHandlerGuard::new(id, Arc::downgrade(self))
    }

    /// Like [`AsyncRegistry::register_guard`] but with an explicit
    /// priority value.
    pub fn register_guard_with_priority<F, Fut>(
        self: &Arc<Self>,
        priority: i32,
        handler: F,
    ) -> AsyncHandlerGuard<E>
    where
        F: Fn(&E) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let id = self.register_with_priority(priority, handler);
        AsyncHandlerGuard::new(id, Arc::downgrade(self))
    }

    /// Unregister an async handler by id. Returns `true` if a handler was
    /// found and removed.
    ///
    /// # Examples
    ///
    /// ```
    /// use registry_io::r#async::AsyncRegistry;
    ///
    /// let registry: AsyncRegistry<()> = AsyncRegistry::new();
    /// let id = registry.register(|_| async move {});
    /// assert!(registry.unregister(id));
    /// assert!(!registry.unregister(id));
    /// ```
    pub fn unregister(&self, id: HandlerId) -> bool {
        let mut removed = false;
        drop(self.handlers.rcu(|current| {
            let mut new_vec: Vec<AsyncHandlerEntry<E>> = Vec::with_capacity(current.len());
            new_vec.extend(current.iter().filter(|e| e.id != id).cloned());
            removed = new_vec.len() != current.len();
            Arc::new(new_vec)
        }));
        removed
    }

    /// Remove every registered handler.
    ///
    /// In-flight `notify*` calls that already loaded the snapshot still run
    /// every handler in their snapshot to completion.
    pub fn clear(&self) {
        self.handlers.store(Arc::new(Vec::new()));
    }

    /// Current handler count.
    #[inline]
    #[must_use]
    pub fn handler_count(&self) -> usize {
        self.handlers.load().len()
    }

    /// `true` if no handlers are registered.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.handlers.load().is_empty()
    }

    /// `true` if a handler with `id` is currently registered.
    #[must_use]
    pub fn contains(&self, id: HandlerId) -> bool {
        self.handlers.load().iter().any(|e| e.id == id)
    }

    /// Install a panic callback. See [`crate::SyncRegistry::on_panic`].
    pub fn on_panic<F>(&self, callback: F)
    where
        F: Fn(&PanicInfo<'_>) + Send + Sync + 'static,
    {
        let holder = Arc::new(PanicCallbackHolder::new(callback));
        self.panic_callback.store(Some(holder));
    }

    /// Remove any previously installed panic callback.
    pub fn clear_panic_callback(&self) {
        self.panic_callback.store(None);
    }

    /// Dispatch `event` to every registered handler **concurrently**.
    ///
    /// Builds one future per handler, then awaits them all via the
    /// crate-local `JoinAll` combinator. Each handler future is wrapped in
    /// a crate-local `CatchUnwind` adapter so a panic in one handler does
    /// not poison the join — its sibling handlers continue.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() {
    /// use std::sync::Arc;
    /// use std::sync::atomic::{AtomicU32, Ordering};
    /// use registry_io::r#async::AsyncRegistry;
    ///
    /// let registry: AsyncRegistry<u32> = AsyncRegistry::new();
    /// let total = Arc::new(AtomicU32::new(0));
    /// for _ in 0..4 {
    ///     let sink = Arc::clone(&total);
    ///     let _ = registry.register(move |value| {
    ///         let sink = Arc::clone(&sink);
    ///         let v = *value;
    ///         async move {
    ///             sink.fetch_add(v, Ordering::Relaxed);
    ///         }
    ///     });
    /// }
    ///
    /// registry.notify(&10).await;
    /// assert_eq!(total.load(Ordering::Relaxed), 40);
    /// # }
    /// ```
    pub async fn notify(&self, event: &E) {
        let snapshot = self.handlers.load();
        if snapshot.is_empty() {
            return;
        }

        // Single pass over the snapshot, producing parallel `ids` and
        // `wrapped` vectors so we can attribute each post-join outcome
        // back to its originating handler. `JoinAll` preserves input
        // order, so a positional zip is exact and saves the intermediate
        // `pairs` allocation the prior implementation needed.
        let n = snapshot.len();
        let mut ids: Vec<HandlerId> = Vec::with_capacity(n);
        let mut wrapped = Vec::with_capacity(n);
        for entry in snapshot.iter() {
            ids.push(entry.id);
            wrapped.push(CatchUnwind::new((entry.handler)(event)));
        }
        let results = JoinAll::new(wrapped).await;

        for (id, outcome) in ids.into_iter().zip(results) {
            if let Err(payload) = outcome {
                self.handle_panic(id, payload);
            }
        }
    }

    /// Dispatch `event` to every registered handler **sequentially**, in
    /// priority order.
    ///
    /// Each handler's future is awaited to completion before the next one
    /// starts. Use this when handlers must observe a strict happens-before
    /// relationship with one another.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() {
    /// use std::sync::{Arc, Mutex};
    /// use registry_io::r#async::AsyncRegistry;
    ///
    /// let registry: AsyncRegistry<()> = AsyncRegistry::new();
    /// let log: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
    ///
    /// let l = Arc::clone(&log);
    /// let _ = registry.register_with_priority(10, move |_| {
    ///     let l = Arc::clone(&l);
    ///     async move { l.lock().unwrap().push("first"); }
    /// });
    /// let l = Arc::clone(&log);
    /// let _ = registry.register(move |_| {
    ///     let l = Arc::clone(&l);
    ///     async move { l.lock().unwrap().push("second"); }
    /// });
    ///
    /// registry.notify_sequential(&()).await;
    /// assert_eq!(log.lock().unwrap().as_slice(), &["first", "second"]);
    /// # }
    /// ```
    pub async fn notify_sequential(&self, event: &E) {
        let snapshot = self.handlers.load();
        for entry in snapshot.iter() {
            let fut = (entry.handler)(event);
            match CatchUnwind::new(fut).await {
                Ok(()) => {}
                Err(payload) => self.handle_panic(entry.id, payload),
            }
        }
    }

    /// Invoke the panic callback (if installed), then drop the payload.
    /// Mirrors [`crate::SyncRegistry`]'s panic plumbing.
    #[cold]
    fn handle_panic(&self, handler_id: HandlerId, payload: Box<dyn Any + Send + 'static>) {
        let guard = self.panic_callback.load();
        if let Some(holder) = guard.as_ref() {
            let info = PanicInfo::new(handler_id, payload.as_ref());
            drop(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || {
                    holder.invoke(&info);
                },
            )));
        }
        drop(payload);
    }
}

impl<E: Send + Sync + 'static> Default for AsyncRegistry<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: Send + Sync + 'static> fmt::Debug for AsyncRegistry<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AsyncRegistry")
            .field("handler_count", &self.handlers.load().len())
            .field("has_panic_callback", &self.panic_callback.load().is_some())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::AsyncRegistry;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn empty_registry_notify_is_noop() {
        let registry: AsyncRegistry<u32> = AsyncRegistry::new();
        registry.notify(&42).await;
        registry.notify_sequential(&42).await;
    }

    #[tokio::test]
    async fn concurrent_notify_fires_every_handler_once() {
        let registry: AsyncRegistry<u32> = AsyncRegistry::new();
        let count = Arc::new(AtomicU32::new(0));
        for _ in 0..5 {
            let sink = Arc::clone(&count);
            let _ = registry.register(move |_| {
                let sink = Arc::clone(&sink);
                async move {
                    let _ = sink.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
        registry.notify(&0).await;
        assert_eq!(count.load(Ordering::Relaxed), 5);
    }
}

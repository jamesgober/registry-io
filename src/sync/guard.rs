//! RAII handler guard that unregisters on drop.
//!
//! A [`HandlerGuard`] is returned by
//! [`SyncRegistry::register_guard`](crate::SyncRegistry::register_guard) and
//! its priority variant. While the guard is alive the handler stays
//! registered; when the guard is dropped (or goes out of scope) the
//! corresponding handler is automatically removed.
//!
//! The guard holds a [`Weak`] reference to the registry so it does not keep
//! the registry alive on its own. If the registry is dropped before the
//! guard, dropping the guard becomes a no-op.

use std::sync::Weak;

use crate::HandlerId;

use super::SyncRegistry;

/// RAII handle for a registered handler.
///
/// Drop the guard to unregister. Call [`HandlerGuard::forget`] to detach the
/// guard from the handler, leaving the handler registered indefinitely.
///
/// The guard is not [`Clone`]; ownership of a registration is unique.
///
/// # Examples
///
/// Automatic cleanup when the guard leaves scope:
///
/// ```
/// use std::sync::Arc;
/// use registry_io::SyncRegistry;
///
/// let registry = Arc::new(SyncRegistry::<u32>::new());
/// assert_eq!(registry.handler_count(), 0);
///
/// {
///     let _guard = registry.register_guard(|n| {
///         println!("got {n}");
///     });
///     assert_eq!(registry.handler_count(), 1);
/// }
///
/// assert_eq!(registry.handler_count(), 0);
/// ```
///
/// Forgetting the guard to keep the handler registered:
///
/// ```
/// use std::sync::Arc;
/// use registry_io::SyncRegistry;
///
/// let registry = Arc::new(SyncRegistry::<()>::new());
/// let guard = registry.register_guard(|_| {});
/// let id = guard.id();
/// guard.forget();
/// // Handler is still active.
/// assert_eq!(registry.handler_count(), 1);
/// // Manually unregister via the returned id.
/// assert!(registry.unregister(id));
/// ```
#[must_use = "dropping the HandlerGuard immediately unregisters the handler; \
              bind it to a name to keep the handler alive"]
pub struct HandlerGuard<E: Send + Sync + 'static> {
    id: HandlerId,
    registry: Weak<SyncRegistry<E>>,
}

impl<E: Send + Sync + 'static> HandlerGuard<E> {
    /// Internal constructor used by `SyncRegistry`.
    pub(crate) fn new(id: HandlerId, registry: Weak<SyncRegistry<E>>) -> Self {
        Self { id, registry }
    }

    /// Returns the [`HandlerId`] of the underlying registration.
    ///
    /// Useful for diagnostic logging or to retain the id before consuming
    /// the guard with [`HandlerGuard::forget`].
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use registry_io::SyncRegistry;
    ///
    /// let registry = Arc::new(SyncRegistry::<()>::new());
    /// let guard = registry.register_guard(|_| {});
    /// let id = guard.id();
    /// drop(guard);
    /// // The id is still meaningful for logging, but the handler is gone:
    /// assert!(!registry.contains(id));
    /// ```
    #[inline]
    #[must_use]
    pub fn id(&self) -> HandlerId {
        self.id
    }

    /// Consume the guard without unregistering the handler.
    ///
    /// The handler remains registered until it is explicitly removed via
    /// [`SyncRegistry::unregister`](crate::SyncRegistry::unregister) or the
    /// registry is dropped.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use registry_io::SyncRegistry;
    ///
    /// let registry = Arc::new(SyncRegistry::<()>::new());
    /// let guard = registry.register_guard(|_| {});
    /// guard.forget();
    /// assert_eq!(registry.handler_count(), 1);
    /// ```
    pub fn forget(self) {
        let mut me = std::mem::ManuallyDrop::new(self);
        // Replace the weak ref with a sentinel that points to nothing so a
        // future `Drop` (impossible after ManuallyDrop, but defensive)
        // would do nothing.
        me.registry = Weak::new();
    }
}

impl<E: Send + Sync + 'static> Drop for HandlerGuard<E> {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            let _ = registry.unregister(self.id);
        }
    }
}

impl<E: Send + Sync + 'static> core::fmt::Debug for HandlerGuard<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HandlerGuard")
            .field("id", &self.id)
            .field("registry_alive", &(self.registry.strong_count() > 0))
            .finish()
    }
}

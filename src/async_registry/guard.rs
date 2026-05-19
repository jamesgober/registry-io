//! RAII async handler guard.
//!
//! Mirror of [`crate::HandlerGuard`] for [`AsyncRegistry`].
//! Dropping the guard unregisters the async handler; holding a
//! [`std::sync::Weak`] reference means a registry dropped before its guard
//! still drops cleanly.

use std::sync::Weak;

use crate::HandlerId;

use super::AsyncRegistry;

/// RAII handle for a registered async handler.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use registry_io::r#async::AsyncRegistry;
///
/// let registry = Arc::new(AsyncRegistry::<()>::new());
/// assert!(registry.is_empty());
///
/// {
///     let _guard = registry.register_guard(|_| async move {});
///     assert_eq!(registry.handler_count(), 1);
/// }
/// assert!(registry.is_empty());
/// ```
#[must_use = "dropping the AsyncHandlerGuard immediately unregisters the handler; \
              bind it to a name to keep the handler alive"]
pub struct AsyncHandlerGuard<E: Send + Sync + 'static> {
    id: HandlerId,
    registry: Weak<AsyncRegistry<E>>,
}

impl<E: Send + Sync + 'static> AsyncHandlerGuard<E> {
    /// Internal constructor used by `AsyncRegistry`.
    pub(crate) fn new(id: HandlerId, registry: Weak<AsyncRegistry<E>>) -> Self {
        Self { id, registry }
    }

    /// Returns the [`HandlerId`] of the underlying registration.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use registry_io::r#async::AsyncRegistry;
    ///
    /// let registry = Arc::new(AsyncRegistry::<()>::new());
    /// let guard = registry.register_guard(|_| async move {});
    /// let id = guard.id();
    /// drop(guard);
    /// assert!(!registry.contains(id));
    /// ```
    #[inline]
    #[must_use]
    pub fn id(&self) -> HandlerId {
        self.id
    }

    /// Consume the guard without unregistering the handler.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use registry_io::r#async::AsyncRegistry;
    ///
    /// let registry = Arc::new(AsyncRegistry::<()>::new());
    /// let guard = registry.register_guard(|_| async move {});
    /// let id = guard.id();
    /// guard.forget();
    /// assert!(registry.contains(id));
    /// assert!(registry.unregister(id));
    /// ```
    pub fn forget(self) {
        let mut me = std::mem::ManuallyDrop::new(self);
        me.registry = Weak::new();
    }
}

impl<E: Send + Sync + 'static> Drop for AsyncHandlerGuard<E> {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            let _ = registry.unregister(self.id);
        }
    }
}

impl<E: Send + Sync + 'static> core::fmt::Debug for AsyncHandlerGuard<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AsyncHandlerGuard")
            .field("id", &self.id)
            .field("registry_alive", &(self.registry.strong_count() > 0))
            .finish()
    }
}

//! Typed wrapper around a caught panic payload.
//!
//! When a handler panics inside [`SyncRegistry::notify`](crate::SyncRegistry::notify)
//! the unwind is caught and, if a panic callback has been installed via
//! [`SyncRegistry::on_panic`](crate::SyncRegistry::on_panic), a [`PanicInfo`]
//! describing the failure is forwarded to that callback. The handler's
//! sibling handlers continue to fire.

use core::any::Any;
use core::fmt;
use std::sync::Arc;

use crate::HandlerId;

/// Information about a panic that occurred inside a handler.
///
/// Produced by the registry when a handler invocation unwinds. The original
/// panic payload is preserved verbatim and can be inspected with
/// [`PanicInfo::message`] (for the common `&str`/`String` cases) or
/// downcast through [`PanicInfo::payload`] for custom panic types.
///
/// # Examples
///
/// ```
/// use std::sync::{Arc, Mutex};
/// use registry_io::SyncRegistry;
///
/// let registry: SyncRegistry<()> = SyncRegistry::new();
/// let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
///
/// let sink = Arc::clone(&captured);
/// registry.on_panic(move |info| {
///     if let Some(msg) = info.message() {
///         sink.lock().unwrap().push(msg.to_owned());
///     }
/// });
///
/// let _ = registry.register(|_| panic!("boom"));
/// registry.notify(&());
///
/// let messages = captured.lock().unwrap();
/// assert_eq!(messages.as_slice(), &["boom".to_owned()]);
/// ```
pub struct PanicInfo<'a> {
    handler_id: HandlerId,
    payload: &'a (dyn Any + Send + 'static),
}

impl<'a> PanicInfo<'a> {
    /// Internal constructor. Public construction is intentionally not
    /// available; the registry produces these values.
    #[inline]
    pub(crate) fn new(handler_id: HandlerId, payload: &'a (dyn Any + Send + 'static)) -> Self {
        Self {
            handler_id,
            payload,
        }
    }

    /// The id of the handler whose invocation panicked.
    ///
    /// Useful when the panic callback wants to log, unregister, or otherwise
    /// react to a specific misbehaving handler.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::atomic::{AtomicU64, Ordering};
    /// use std::sync::Arc;
    /// use registry_io::SyncRegistry;
    ///
    /// let registry: SyncRegistry<()> = SyncRegistry::new();
    /// let failing_id = Arc::new(AtomicU64::new(0));
    /// let sink = Arc::clone(&failing_id);
    /// registry.on_panic(move |info| {
    ///     sink.store(info.handler_id().as_u64(), Ordering::SeqCst);
    /// });
    ///
    /// let id = registry.register(|_| panic!("nope"));
    /// registry.notify(&());
    /// assert_eq!(failing_id.load(Ordering::SeqCst), id.as_u64());
    /// ```
    #[inline]
    #[must_use]
    pub fn handler_id(&self) -> HandlerId {
        self.handler_id
    }

    /// Returns the raw panic payload as a `&dyn Any` reference.
    ///
    /// Use this when the panic payload is a custom type that
    /// [`PanicInfo::message`] cannot interpret. Downcast with
    /// [`<dyn Any>::downcast_ref`](core::any::Any::downcast_ref::<()>).
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::{Arc, Mutex};
    /// use registry_io::SyncRegistry;
    ///
    /// #[derive(Debug, PartialEq, Eq)]
    /// struct MyErr(i32);
    ///
    /// let registry: SyncRegistry<()> = SyncRegistry::new();
    /// let captured: Arc<Mutex<Option<i32>>> = Arc::new(Mutex::new(None));
    /// let sink = Arc::clone(&captured);
    /// registry.on_panic(move |info| {
    ///     if let Some(err) = info.payload().downcast_ref::<MyErr>() {
    ///         *sink.lock().unwrap() = Some(err.0);
    ///     }
    /// });
    ///
    /// let _ = registry.register(|_| std::panic::panic_any(MyErr(7)));
    /// registry.notify(&());
    /// assert_eq!(*captured.lock().unwrap(), Some(7));
    /// ```
    #[inline]
    #[must_use]
    pub fn payload(&self) -> &(dyn Any + Send + 'static) {
        self.payload
    }

    /// Best-effort extraction of the panic message as a `&str`.
    ///
    /// Returns `Some` when the panic payload was a `&'static str` (the
    /// common `panic!("literal")` case) or a `String` (the
    /// `panic!("{}", value)` case). Returns `None` for custom panic types;
    /// in that case use [`PanicInfo::payload`] and downcast manually.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::{Arc, Mutex};
    /// use registry_io::SyncRegistry;
    ///
    /// let registry: SyncRegistry<()> = SyncRegistry::new();
    /// let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    /// let sink = Arc::clone(&log);
    /// registry.on_panic(move |info| {
    ///     sink.lock().unwrap().push(info.message().unwrap_or("<opaque>").to_owned());
    /// });
    ///
    /// let _ = registry.register(|_| panic!("static literal"));
    /// let _ = registry.register(|_| panic!("formatted {}", 42));
    /// registry.notify(&());
    ///
    /// let messages = log.lock().unwrap();
    /// assert_eq!(messages.as_slice(), &[
    ///     "static literal".to_owned(),
    ///     "formatted 42".to_owned(),
    /// ]);
    /// ```
    #[inline]
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        if let Some(s) = self.payload.downcast_ref::<&'static str>() {
            Some(*s)
        } else if let Some(s) = self.payload.downcast_ref::<String>() {
            Some(s.as_str())
        } else {
            None
        }
    }
}

impl fmt::Debug for PanicInfo<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PanicInfo")
            .field("handler_id", &self.handler_id)
            .field("message", &self.message())
            .finish()
    }
}

/// Internal alias for the type-erased panic callback function type.
pub(crate) type PanicCallback = dyn Fn(&PanicInfo<'_>) + Send + Sync + 'static;

/// Sized holder for an `Arc<dyn Fn>` panic callback, so it can be stored in
/// [`arc_swap::ArcSwapOption`] (which requires `T: Sized`).
pub(crate) struct PanicCallbackHolder {
    inner: Arc<PanicCallback>,
}

impl PanicCallbackHolder {
    /// Wrap a user-supplied callback for storage on the registry.
    #[inline]
    pub(crate) fn new<F>(callback: F) -> Self
    where
        F: Fn(&PanicInfo<'_>) + Send + Sync + 'static,
    {
        Self {
            inner: Arc::new(callback),
        }
    }

    /// Invoke the wrapped callback.
    #[inline]
    pub(crate) fn invoke(&self, info: &PanicInfo<'_>) {
        (self.inner)(info);
    }
}

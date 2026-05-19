//! Opaque handler identifier returned by registration.
//!
//! Each registration produces a [`HandlerId`] that the caller can use to
//! later unregister the handler. Ids are unique within the registry that
//! issued them and are not portable across registries.

use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

/// Opaque identifier for a registered handler.
///
/// Returned by [`SyncRegistry::register`](crate::SyncRegistry::register) and
/// its priority/guard variants, accepted by
/// [`SyncRegistry::unregister`](crate::SyncRegistry::unregister).
///
/// `HandlerId` is `Copy` and cheap to compare. The internal numeric
/// representation is intentionally opaque: callers should not rely on
/// specific values or any ordering between ids.
///
/// # Uniqueness
///
/// Ids are unique **within the registry that issued them**, for the lifetime
/// of that registry. Two registries may issue the same numeric id; never use
/// an id with a registry other than the one that returned it.
///
/// # Examples
///
/// ```
/// use registry_io::SyncRegistry;
///
/// let registry: SyncRegistry<u32> = SyncRegistry::new();
/// let id_a = registry.register(|_| {});
/// let id_b = registry.register(|_| {});
///
/// assert_ne!(id_a, id_b);
/// assert!(registry.unregister(id_a));
/// assert!(registry.unregister(id_b));
/// ```
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct HandlerId(u64);

impl HandlerId {
    /// Construct a [`HandlerId`] from a raw integer.
    ///
    /// Internal: only the id generator should call this. The numeric domain
    /// is otherwise opaque.
    #[inline]
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Returns the raw numeric value backing this id.
    ///
    /// The exact numeric domain is unspecified and may change between
    /// releases. This is provided for diagnostic use only (logging,
    /// debugging); do not rely on specific values or arithmetic over them.
    ///
    /// # Examples
    ///
    /// ```
    /// use registry_io::SyncRegistry;
    ///
    /// let registry: SyncRegistry<()> = SyncRegistry::new();
    /// let id = registry.register(|_| {});
    /// let raw = id.as_u64();
    /// assert!(raw > 0);
    /// ```
    #[inline]
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for HandlerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HandlerId({})", self.0)
    }
}

impl fmt::Display for HandlerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// Atomic counter that issues unique [`HandlerId`]s for one registry.
///
/// Uses `Ordering::Relaxed` because uniqueness only requires atomicity of
/// the fetch-and-add itself; no other memory state is ordered against the
/// id allocation.
#[derive(Debug)]
pub(crate) struct HandlerIdGenerator {
    next: AtomicU64,
}

impl HandlerIdGenerator {
    /// Construct a new generator. The first id issued will be `1`.
    #[inline]
    pub(crate) const fn new() -> Self {
        Self {
            next: AtomicU64::new(1),
        }
    }

    /// Issue the next unique id.
    #[inline]
    pub(crate) fn next(&self) -> HandlerId {
        let raw = self.next.fetch_add(1, Ordering::Relaxed);
        HandlerId::from_raw(raw)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{HandlerId, HandlerIdGenerator};

    #[test]
    fn debug_format_includes_value() {
        let id = HandlerId::from_raw(42);
        assert_eq!(format!("{id:?}"), "HandlerId(42)");
    }

    #[test]
    fn display_format_is_bare_number() {
        let id = HandlerId::from_raw(42);
        assert_eq!(format!("{id}"), "42");
    }

    #[test]
    fn ids_are_copy_eq_hash() {
        let a = HandlerId::from_raw(7);
        let b = a;
        assert_eq!(a, b);
        let mut set = std::collections::HashSet::new();
        let _ = set.insert(a);
        assert!(set.contains(&b));
    }

    #[test]
    fn generator_produces_unique_ascending_ids() {
        let generator = HandlerIdGenerator::new();
        let a = generator.next();
        let b = generator.next();
        let c = generator.next();
        assert_eq!(a.as_u64(), 1);
        assert_eq!(b.as_u64(), 2);
        assert_eq!(c.as_u64(), 3);
        assert_ne!(a, b);
        assert_ne!(b, c);
    }

    #[test]
    fn generator_is_thread_safe_and_unique() {
        use std::sync::Arc;
        use std::thread;

        let generator = Arc::new(HandlerIdGenerator::new());
        let mut handles = Vec::new();
        for _ in 0..8 {
            let g = Arc::clone(&generator);
            handles.push(thread::spawn(move || {
                let mut ids = Vec::with_capacity(1000);
                for _ in 0..1000 {
                    ids.push(g.next());
                }
                ids
            }));
        }
        let mut all = Vec::with_capacity(8 * 1000);
        for h in handles {
            let mut ids = h.join().expect("worker thread did not panic");
            all.append(&mut ids);
        }
        let unique: std::collections::HashSet<_> = all.iter().copied().collect();
        assert_eq!(unique.len(), all.len());
    }
}

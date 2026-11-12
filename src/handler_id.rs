//! Opaque handler identifier returned by registration.
//!
//! Each registration produces a [`HandlerId`] that the caller can use to
//! later unregister the handler. Ids come from one process-wide counter,
//! so no two registries ever issue the same id.

use core::fmt;
#[cfg(any(feature = "sync", feature = "async"))]
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
/// Ids are unique **across every registry in the process** (since 1.2.0;
/// before, each registry counted from 1). An id passed to a registry that
/// did not issue it matches no handler there, so `unregister` returns
/// `false` instead of removing an unrelated handler.
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
    #[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
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

#[cfg(any(feature = "sync", feature = "async"))]
/// The process-wide id counter. Shared by every registry, so an id is
/// never valid in two registries at once.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Issues unique [`HandlerId`]s for one registry, from the process-wide
/// counter.
///
/// Uses `Ordering::Relaxed` because uniqueness only requires atomicity of
/// the fetch-and-add itself; no other memory state is ordered against the
/// id allocation. At one id per nanosecond the 64-bit counter lasts more
/// than 500 years.
#[cfg(any(feature = "sync", feature = "async"))]
#[derive(Debug)]
pub(crate) struct HandlerIdGenerator {
    _private: (),
}

#[cfg(any(feature = "sync", feature = "async"))]
impl HandlerIdGenerator {
    /// Construct a new generator.
    #[inline]
    pub(crate) const fn new() -> Self {
        Self { _private: () }
    }

    /// Issue the next unique id.
    #[inline]
    #[allow(clippy::unused_self)] // keeps the per-registry call shape
    pub(crate) fn next(&self) -> HandlerId {
        let raw = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        HandlerId::from_raw(raw)
    }
}

#[cfg(all(test, any(feature = "sync", feature = "async")))]
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
        assert!(a.as_u64() >= 1);
        assert!(a.as_u64() < b.as_u64());
        assert!(b.as_u64() < c.as_u64());
    }

    // Review F-5: every registry counted from 1, so an id passed to the
    // wrong registry removed an unrelated handler there.
    #[test]
    fn two_generators_never_issue_the_same_id() {
        let first = HandlerIdGenerator::new();
        let second = HandlerIdGenerator::new();
        let a: Vec<HandlerId> = (0..100).map(|_| first.next()).collect();
        let b: Vec<HandlerId> = (0..100).map(|_| second.next()).collect();
        assert!(a.iter().all(|id| !b.contains(id)));
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

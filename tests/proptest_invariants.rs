//! Property-based invariant tests for `SyncRegistry`.
//!
//! Verifies a small set of registry invariants across many random
//! operation sequences. The properties are deliberately scoped to the
//! observable public API:
//!
//! 1. Registering `N` handlers and unregistering all of them returns the
//!    registry to a zero-handler state.
//! 2. `notify` fires exactly once per registered handler, regardless of
//!    handler count or registration order.
//! 3. `HandlerId`s issued by the same registry are unique even after many
//!    register / unregister cycles.
//! 4. A random sequence of `Register` / `Unregister` operations leaves
//!    `handler_count` consistent with the (issued - removed) bookkeeping
//!    that the test maintains alongside.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use proptest::collection::vec;
use proptest::prelude::*;

use registry_io::{HandlerId, SyncRegistry};

proptest! {
    #[test]
    fn register_then_unregister_all_leaves_registry_empty(n in 0_usize..=200) {
        let registry: SyncRegistry<u32> = SyncRegistry::new();
        let ids: Vec<HandlerId> = (0..n).map(|_| registry.register(|_| {})).collect();
        prop_assert_eq!(registry.handler_count(), n);

        for id in &ids {
            prop_assert!(registry.unregister(*id));
        }
        prop_assert_eq!(registry.handler_count(), 0);
        prop_assert!(registry.is_empty());
    }

    #[test]
    fn notify_fires_exactly_once_per_registered_handler(n in 0_usize..=200) {
        let registry: SyncRegistry<u32> = SyncRegistry::new();
        let count = Arc::new(AtomicUsize::new(0));
        for _ in 0..n {
            let c = Arc::clone(&count);
            let _ = registry.register(move |_| {
                let _ = c.fetch_add(1, Ordering::Relaxed);
            });
        }

        registry.notify(&0);
        prop_assert_eq!(count.load(Ordering::Relaxed), n);

        registry.notify(&0);
        prop_assert_eq!(count.load(Ordering::Relaxed), n * 2);
    }

    #[test]
    fn handler_ids_are_unique_across_churn(
        // A sequence of pseudo-random booleans: true = register, false =
        // unregister-most-recent. The shape of the sequence drives the
        // churn pattern.
        ops in vec(any::<bool>(), 0..=500)
    ) {
        let registry: SyncRegistry<u32> = SyncRegistry::new();
        let mut issued: HashSet<HandlerId> = HashSet::new();
        let mut live: Vec<HandlerId> = Vec::new();

        for &register in &ops {
            if register || live.is_empty() {
                let id = registry.register(|_| {});
                prop_assert!(issued.insert(id), "registry issued a duplicate id: {id:?}");
                live.push(id);
            } else {
                let id = live.pop().unwrap();
                prop_assert!(registry.unregister(id));
            }
        }
    }

    #[test]
    fn handler_count_matches_bookkeeping_across_random_ops(
        ops in vec(any::<bool>(), 0..=500)
    ) {
        let registry: SyncRegistry<u32> = SyncRegistry::new();
        let mut live: Vec<HandlerId> = Vec::new();

        for &register in &ops {
            if register || live.is_empty() {
                live.push(registry.register(|_| {}));
            } else {
                let id = live.pop().unwrap();
                prop_assert!(registry.unregister(id));
            }
            prop_assert_eq!(registry.handler_count(), live.len());
        }
    }

    #[test]
    fn unregister_of_already_removed_id_always_returns_false(
        additional_registers in 0_usize..=50,
    ) {
        let registry: SyncRegistry<u32> = SyncRegistry::new();
        let stale = registry.register(|_| {});
        prop_assert!(registry.unregister(stale));

        // Add more handlers; the stale id must still be rejected even after
        // arbitrary subsequent activity churns the internal list.
        for _ in 0..additional_registers {
            let _ = registry.register(|_| {});
        }

        prop_assert!(!registry.unregister(stale));
    }

    #[test]
    fn clear_then_register_resumes_with_fresh_unique_ids(
        before in 0_usize..=50,
        after in 1_usize..=50,
    ) {
        let registry: SyncRegistry<u32> = SyncRegistry::new();
        let mut prior: HashSet<HandlerId> = HashSet::new();
        for _ in 0..before {
            let _ = prior.insert(registry.register(|_| {}));
        }
        registry.clear();
        prop_assert_eq!(registry.handler_count(), 0);

        for _ in 0..after {
            let id = registry.register(|_| {});
            // `clear` does not reset the id generator; new ids never
            // collide with previously-issued ones.
            prop_assert!(!prior.contains(&id), "id reused after clear(): {id:?}");
        }
        prop_assert_eq!(registry.handler_count(), after);
    }
}

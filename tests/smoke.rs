//! Smoke test - verifies the crate is reachable and the headline API works.

use registry_io::{HandlerId, SyncRegistry};

#[test]
fn version_is_set() {
    assert!(!registry_io::VERSION.is_empty());
}

#[test]
fn end_to_end_register_notify_unregister() {
    let registry: SyncRegistry<u32> = SyncRegistry::new();
    let id: HandlerId = registry.register(|_| {});
    assert_eq!(registry.handler_count(), 1);
    registry.notify(&42);
    assert!(registry.unregister(id));
    assert_eq!(registry.handler_count(), 0);
}

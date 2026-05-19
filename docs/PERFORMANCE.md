# registry-io — Performance

This document describes the runtime cost model for `registry-io 0.4.0`,
how to benchmark it locally, and the principles that govern the
implementation.

The performance contract for `1.0.0` is in `.dev/ROADMAP.md`. Numbers below
reflect what the code is **designed** to deliver — measured numbers on
production hardware will be recorded as the verification phase
(`0.6.0`) completes.

---

## Cost model

| Operation                                        | Cost                | Allocates? | Locks? |
|--------------------------------------------------|---------------------|:----------:|:------:|
| `SyncRegistry::new` / `with_capacity`            | one `Arc<Vec>` alloc | ✓          | —      |
| `SyncRegistry::register*`                        | `O(N)` clone + swap | ✓          | atomic CAS |
| `SyncRegistry::unregister`                       | `O(N)` clone + swap | ✓          | atomic CAS |
| `SyncRegistry::clear`                            | one `Arc<Vec>` alloc + atomic store | ✓ | atomic store |
| `SyncRegistry::handler_count` / `is_empty`       | atomic load         | —          | —      |
| `SyncRegistry::contains`                         | `O(N)` scan         | —          | —      |
| `SyncRegistry::notify` (no panics)               | `O(N)` virtual calls + `catch_unwind` | — | — |
| `SyncRegistry::notify` (handler panics)          | + one `Box<dyn Any>` per panic | ✓ on panic | — |
| `HandlerGuard::drop`                             | one `unregister` call | ✓        | atomic CAS |

`N` is the number of currently-registered handlers.

---

## Hot path: what `notify` actually does

```rust
#[inline]
pub fn notify(&self, event: &E) {
    let snapshot = self.handlers.load();          // 1 atomic load
    for entry in snapshot.iter() {                // straight-line scan
        let handler = &entry.handler;             // borrow Arc<dyn Fn>
        let result = catch_unwind(AssertUnwindSafe(|| handler(event)));
        if let Err(payload) = result {
            self.handle_panic(entry.id, payload); // cold path
        }
    }
}
```

The no-panic path:

- Loads an [`arc_swap::Guard`] (single atomic acquire load, no allocation).
- Iterates the snapshot's `Vec<HandlerEntry<E>>` in priority order.
- Calls each handler through dynamic dispatch.
- Wraps each call in `catch_unwind` (no allocation when no panic occurs).

There is no `Mutex`, no `RwLock`, no channel send, no per-iteration
allocation. The cost per handler is roughly:

```
load arc-swap guard:    1 atomic acquire load
per-handler:            1 Arc deref + 1 vtable lookup + 1 indirect call
                        + catch_unwind setup/teardown
```

`catch_unwind` adds modest overhead per call (typically tens of cycles on
modern x86-64). This is the cost of panic isolation — handlers are isolated
from one another and from the caller. There is no opt-out in 0.4.0; a
future release may add a `notify_trusted` variant for callers that accept
panic propagation in exchange for the saved cycles.

---

## Slow path: register / unregister

`register*` and `unregister` follow the standard
read-clone-modify-CAS-swap pattern via [`arc_swap::ArcSwap::rcu`]:

1. Load the current `Arc<Vec<HandlerEntry<E>>>`.
2. Clone the `Vec` (one allocation; cloning each entry is just an `Arc`
   refcount bump).
3. Push (or remove) the entry.
4. Try to atomically swap. If another writer raced, retry from step 1.

Under heavy register/unregister contention, retries may occur. The notify
hot path is **never** affected — readers always see a complete snapshot.

`register_with_priority` inserts the new entry at the correct position
using binary search (`Vec::partition_point`), so the priority ordering
invariant is maintained without a full re-sort.

---

## Memory footprint

- An **empty registry**: one `Arc<Vec<...>>` (16 bytes) + `id_generator` (8
  bytes) + `panic_callback` (16 bytes) ≈ 48 bytes plus the empty `Vec`'s
  metadata. Well under the 128-byte target.
- Per **registered handler**: `HandlerId` (8) + priority (4 + 4 padding) +
  `Arc<dyn Fn>` (16) = **32 bytes** per slot. 100 handlers ≈ 3.2 KiB +
  per-handler closure allocation. Comfortably under the 16 KiB target.

---

## Running the benchmark suite

```bash
cargo bench --bench sync_notify
cargo bench --bench register_unregister
```

The benches use [`criterion`] and produce HTML reports under
`target/criterion/`.

### Scenarios covered

`benches/sync_notify.rs`:

- `notify/0_handlers` — baseline cost of a no-op notify.
- `notify/1_handlers` through `notify/64_handlers` — dispatch with N
  registered handlers, single thread.
- `notify/contended/N_threads` — `N` threads firing notify against a
  4-handler registry concurrently.

`benches/register_unregister.rs`:

- `register/into_N_handlers` — adding a new handler when N are already
  registered (the slow-path cost as the list grows).
- `unregister/from_N_handlers` — removing a handler.

---

## Verifying zero allocation on the hot path

The `notify` no-panic path is allocation-free **by construction**, but the
intended verification approach (planned for phase 0.6.0) is:

1. Plug the `dhat` allocator under a dedicated bench harness.
2. Register 8 handlers.
3. Call `notify` ~10⁶ times.
4. Assert `dhat::HeapStats::total_blocks` does not grow.

This adds an automated guard against regressions like an accidental
`Arc::clone` on the hot path.

---

## Concurrency characteristics

- **Many simultaneous readers**: `notify` from any number of threads in
  parallel is supported with zero coordination. The `ArcSwap::load` is a
  single atomic acquire; iteration is over a snapshot that no writer can
  mutate.
- **Reader + writer concurrency**: a register or unregister concurrent
  with notify never causes a notify to skip or duplicate handlers — both
  observe consistent snapshots.
- **Many simultaneous writers**: handled correctly by `ArcSwap::rcu`
  retry-on-conflict; under contention some writes may retry, but
  correctness is preserved.

---

## Anti-patterns to avoid

- **Slow handlers**: handlers run inline on the caller's thread. If a
  handler does network I/O, the entire notify becomes slow. Spawn slow
  work onto a runtime instead of doing it in the handler.
- **Handlers that re-enter the registry**: `register` or `unregister` from
  inside a handler is supported (it operates on the next snapshot), but
  calling `notify` recursively is unbounded — it will see whatever the
  current snapshot is and may not converge.
- **Large per-handler captures**: each registration becomes an
  `Arc<dyn Fn>` heap allocation. A handler that captures a 1 MB buffer
  costs 1 MB per registration. Keep captures small; share via `Arc`.

---

<sub>registry-io v0.4.0 — Copyright © 2026 James Gober. Apache-2.0 OR MIT.</sub>

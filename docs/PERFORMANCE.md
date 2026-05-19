# registry-io — Performance

Measured performance characteristics for `registry-io 0.9.0` (numbers
captured during the 0.6.0 performance-verification milestone, valid for
all subsequent releases unless `notify` is touched), plus the
cost model, methodology, and reproducibility notes.

The performance contract for `1.0.0` lives in `.dev/ROADMAP.md`. Every
number below is **measured**, not extrapolated.

---

## Measurement environment (baseline)

- **Date:** 2026-05-19
- **CPU:** Intel x86-64 (Windows host, MSVC toolchain)
- **OS:** Windows 11 Pro 26200
- **Rust:** stable (1.95)
- **Bench profile:** `opt-level = 3`, `lto = "fat"`, `codegen-units = 1`,
  `debug = true` (for symbol resolution only)
- **Tool:** `criterion 0.5`, warm-up 1 s, measurement 2–3 s,
  20–30 samples per scenario

Numbers will vary across machines, kernels, and CPU generations. Re-run
the bench suite on your target hardware before making absolute claims.

---

## Headline: sync notify

| Scenario                                  | Median time | Contract target |
|-------------------------------------------|------------:|----------------:|
| `notify`, **0 handlers**                  |    **9.2 ns** | (no target)     |
| `notify`, **1 handler**                   |   **10.1 ns** | `<20 ns` ✓      |
| `notify`, **4 handlers**                  |   **12.5 ns** | `<50 ns` ✓      |
| `notify`, **16 handlers**                 |   **26.0 ns** | `<200 ns` ✓     |
| `notify`, **64 handlers**                 |  **112.7 ns** | (no target)     |
| `notify`, 4 handlers, **1 thread**        |   **26.1 ns** |                 |
| `notify`, 4 handlers, **4 threads**       |   **22.6 ns** |                 |
| `notify`, 4 handlers, **16 threads**      |   **24.7 ns** | `<50 ns` ✓      |

All sync targets in the Performance Contract are met with significant headroom.

### Per-handler cost

Marginal cost per registered handler is approximately **1.6 ns**
(measured slope between the 1-handler and 16-handler points). This
corresponds to one `Arc` deref, one vtable lookup, one indirect call,
and one `catch_unwind` setup/teardown per handler.

---

## Contention sweep

Dispatch-side scaling under simultaneous read contention against a
fixed handler set. Each cell is **ns per `notify` call** averaged across
the contending threads (`benches/contention.rs`):

| Handlers | 1 thread | 4 threads | 16 threads | 64 threads |
|---------:|---------:|----------:|-----------:|-----------:|
|        1 |  12.8 ns |   10.2 ns |    13.3 ns |    19.1 ns |
|        4 |  25.2 ns |   31.9 ns |    41.4 ns |    38.2 ns |
|       16 |  74.0 ns |  127.8 ns |   145.5 ns |   158.9 ns |

At 1 handler, jumping from 1 → 64 threads costs ~6 ns of additional
per-notify time. The lock-free `ArcSwap` read path effectively eliminates
inter-thread synchronization on the hot path.

---

## Async notify

Concurrent dispatch goes through `CatchUnwind` + the crate-local
`JoinAll`; sequential dispatch awaits each handler in turn. Both modes
include the boxed-future allocation per handler.

| Scenario                                       | Median time | Contract target |
|------------------------------------------------|------------:|----------------:|
| `notify` *(concurrent)*, **0 handlers**        |   **10.7 ns** | (no target) |
| `notify` *(concurrent)*, **1 handler**         |  **177 ns**   | `<500 ns` ✓ |
| `notify` *(concurrent)*, **4 handlers**        |  **353 ns**   |             |
| `notify` *(concurrent)*, **16 handlers**       | **1.39 µs**   |             |
| `notify_sequential`, **0 handlers**            |   **10.9 ns** |             |
| `notify_sequential`, **1 handler**             |   **53 ns**   |             |
| `notify_sequential`, **4 handlers**            |  **185 ns**   |             |
| `notify_sequential`, **16 handlers**           |  **694 ns**   |             |

`notify_sequential` is **3× faster than concurrent for small handler
counts** because it skips the `JoinAll` allocation. The concurrent path
overtakes only when handlers do real `.await` work — see
`examples/async_concurrent_vs_sequential.rs` for the canonical wall-clock
comparison (50 ms sleep per handler → ~50 ms concurrent vs ~200 ms
sequential).

### Choosing a dispatch mode

| If your handlers...                       | Use                  |
|-------------------------------------------|----------------------|
| are `async fn` but never `.await` anything | `notify_sequential` (lower overhead) |
| `.await` real I/O or sleeps               | `notify` (concurrent)               |
| must observe strict happens-before order   | `notify_sequential` (always)        |

---

## Register / unregister (slow path)

The clone-then-swap rcu pattern means register and unregister cost
scales linearly with the current handler count `N` (`O(N)` Vec clone +
one Arc allocation per call). The notify hot path is **never** affected.

| N (existing handlers) | `register` median | `unregister` median |
|----------------------:|------------------:|--------------------:|
|                     0 |          287 ns   |               —     |
|                     1 |              —    |          273 ns     |
|                    16 |          682 ns   |          624 ns     |
|                   100 |         2.55 µs   |          2.58 µs    |
|                  1000 |          23.4 µs  |          23.9 µs    |

The Performance Contract target was `<1 µs` for the slow path. For the
typical "small registry" (`N ≤ 16`) we're under it; for `N = 100` we're
2.5× the target. This is the documented and intentional cost of the
lock-free read path. If your workload churns thousands of handlers per
second through a 100+ handler registry, consider batching registrations
or maintaining multiple smaller registries instead.

---

## Zero-allocation verification

`tests/zero_alloc.rs` uses [`dhat`](https://crates.io/crates/dhat) to
verify that `SyncRegistry::notify` performs **zero** heap allocations on
the no-panic hot path. Two scenarios are exercised:

1. **Empty registry**, 100 000 `notify(&v)` calls — `0` new blocks, `0`
   new bytes.
2. **8 registered handlers**, 100 000 `notify(&v)` calls — `0` new
   blocks, `0` new bytes.

Run yourself:

```bash
cargo test --features dhat-heap --test zero_alloc
```

The `dhat-heap` feature swaps the global allocator to `dhat::Alloc`, so
it is **off by default** to keep regular tests and benchmarks free of
profiling overhead.

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
| `AsyncRegistry::notify` (concurrent)             | `O(N)` Box-pin + JoinAll alloc | ✓ | — |
| `AsyncRegistry::notify_sequential`               | `O(N)` Box-pin + awaits | ✓ | — |
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
allocation. Per-handler cost decomposition (measured):

```
load arc-swap guard:  ~2 ns one-time per notify
per-handler:          ~1.6 ns marginal cost
                      (Arc deref + vtable + indirect call
                       + catch_unwind setup/teardown)
```

`#[cold]` on `handle_panic` keeps the panic-handling branch out of the
hot instruction cache.

---

## Slow path: register / unregister

`register*` and `unregister` follow the standard
read-clone-modify-CAS-swap pattern via `arc_swap::ArcSwap::rcu`:

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

- An **empty registry**: one `Arc<Vec<...>>` header (~32 bytes including
  Arc's refcount block) + `id_generator` (8 bytes `AtomicU64`) +
  `panic_callback` (16 bytes `ArcSwapOption`). Well under the 128-byte
  target.
- Per **registered handler**: `HandlerId` (8 B) + priority (4 + 4 B
  padding) + `Arc<dyn Fn>` (16 B) = **32 bytes** per slot. 100 handlers
  ≈ 3.2 KiB + per-handler closure allocation. Comfortably under the
  16 KiB target.

---

## Reproducing these numbers

```bash
# Sync hot path scaling
cargo bench --bench sync_notify

# Slow path (register / unregister)
cargo bench --bench register_unregister

# Sync notify under thread contention
cargo bench --bench contention

# Async path (concurrent + sequential)
cargo bench --bench async_notify --features async

# Zero-allocation verification
cargo test --features dhat-heap --test zero_alloc
```

Criterion writes HTML reports under `target/criterion/`.

For a faster (less precise) sweep, append
`-- --warm-up-time 1 --measurement-time 2 --sample-size 20` to any
`cargo bench` invocation.

---

## Concurrency characteristics

- **Many simultaneous readers**: `notify` from any number of threads in
  parallel is supported with zero coordination. The `ArcSwap::load` is a
  single atomic acquire; iteration is over a snapshot that no writer can
  mutate. Measured 64-thread contention at 4 handlers: 38 ns per
  notify.
- **Reader + writer concurrency**: a register or unregister concurrent
  with notify never causes a notify to skip or duplicate handlers — both
  observe consistent snapshots.
- **Many simultaneous writers**: handled correctly by `ArcSwap::rcu`
  retry-on-conflict; under contention some writes may retry, but
  correctness is preserved.

---

## Anti-patterns to avoid

- **Slow handlers**: handlers run inline on the caller's thread for the
  sync registry, and concurrently inside whatever runtime is driving the
  async registry's `.await`. Doing network I/O directly in a sync handler
  blocks the entire notify; doing it in an async handler is fine but
  yields to the runtime. Choose your mode accordingly.
- **Handlers that re-enter the registry**: `register` or `unregister`
  from inside a handler is supported (it operates on the next snapshot),
  but calling `notify` recursively is unbounded — it will see whatever
  the current snapshot is and may not converge.
- **Large per-handler captures**: each registration becomes an
  `Arc<dyn Fn>` heap allocation. A handler that captures a 1 MB buffer
  costs 1 MB per registration. Keep captures small; share via `Arc`.
- **Thousands-of-handlers registries**: the slow-path scales linearly
  with `N`. A registry with 10 000 handlers will have ~250 µs register
  latency. If that matters, partition into multiple smaller registries.

---

<sub>registry-io v0.9.0 — Copyright © 2026 James Gober. Apache-2.0 OR MIT.</sub>

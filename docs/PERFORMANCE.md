# registry-io — Performance

Measured performance characteristics for `registry-io 1.1.0`, compared
against a fresh baseline run of the 1.0.0 code, plus the cost model,
methodology, and reproducibility notes.

The performance contract for `1.0.0` lives in `.dev/ROADMAP.md` and is
restated in [`STABILITY-1.0.md`](./STABILITY-1.0.md#performance-contract).
Every number below is **measured**, not extrapolated.

---

## Measurement environment (baseline)

- **Date:** 2026-10-13
- **CPU:** Intel x86-64 (Windows host, MSVC toolchain)
- **OS:** Windows 11 Pro 26200
- **Rust:** stable
- **Bench profile:** `opt-level = 3`, `lto = "fat"`, `codegen-units = 1`,
  `debug = true` (for symbol resolution only)
- **Tool:** `criterion 0.5`, warm-up 1 s, measurement 3 s
- **Comparison:** the "1.0.0" column is the 1.0.0 code and the "1.1.0"
  column is the current code, both run on the same machine with the
  same benchmark code. The 1.0.0 column therefore differs slightly from
  the figures published with 1.0.0. "Change" is relative to 1.0.0;
  negative is faster.

Numbers will vary across machines, kernels, and CPU generations. Re-run
the bench suite on your target hardware before making absolute claims.

---

## Headline: sync notify

Single thread, handlers that do no shared writes (`benches/sync_notify.rs`,
`notify/N_handlers`):

| Scenario                   |     1.0.0 |     1.1.0 | Change | Contract target |
|----------------------------|----------:|----------:|-------:|----------------:|
| `notify`, **0 handlers**   |   9.49 ns |   5.41 ns |   -45% | (no target)     |
| `notify`, **1 handler**    |  10.50 ns |   6.37 ns |   -39% | `<20 ns`        |
| `notify`, **4 handlers**   |  12.78 ns |   8.58 ns |   -32% | `<50 ns`        |
| `notify`, **16 handlers**  |  26.38 ns |  24.78 ns |    -7% | `<200 ns`       |
| `notify`, **64 handlers**  | 109.85 ns |  99.31 ns |    -9% | (no target)     |

The contended contract target (4 handlers, 16 threads, `<50 ns`) is
covered by the contention tables below.

### Per-handler cost

Marginal cost per registered handler is approximately **1.2 ns**
(slope between the 1-handler and 16-handler points), rising to about
1.6 ns between 16 and 64 handlers as the handler list and closures
leave the L1 cache. Each handler costs one `Arc` deref, one vtable
lookup, one indirect call, and one `catch_unwind` setup/teardown.

The gain in 1.1.0 is almost entirely in the fixed per-`notify` cost
(about 4 ns at 0 to 4 handlers), which is the read-side protection of
the handler list. The per-handler work is unchanged, so the relative
gain shrinks as the handler count grows.

A single `catch_unwind` around the whole handler loop (resuming after
a panic) was measured and rejected: it was about 36% slower at 16
handlers, because the resume index must live in memory across the
unwind boundary. One `catch_unwind` per handler is kept.

---

## Read scaling across threads

`benches/contention.rs` runs two groups over the same sweep of
`{1, 4, 16, 64}` notifying threads × `{1, 4, 16}` handlers. Each cell
is wall-clock time divided by the **total** number of `notify` calls
across all threads, so it falls as threads add parallel throughput.

### `pure_read` (handlers do no shared writes)

This group isolates the registry's own read scaling. It is new in
1.1.0; the 1.0.0 column is the 1.0.0 code run against it.

| Threads | Handlers |    1.0.0 |    1.1.0 | Change |
|--------:|---------:|---------:|---------:|-------:|
|       1 |        1 | 10.90 ns |  5.51 ns |   -47% |
|       1 |        4 | 14.54 ns |  7.70 ns |   -47% |
|       1 |       16 | 33.78 ns | 24.08 ns |   -29% |
|       4 |        1 |  3.21 ns |  1.65 ns |   -49% |
|       4 |        4 |  4.35 ns |  2.48 ns |   -45% |
|       4 |       16 | 10.13 ns |  7.62 ns |   -25% |
|      16 |        1 | 0.854 ns | 0.463 ns |   -46% |
|      16 |        4 | 1.232 ns | 0.689 ns |   -44% |
|      16 |       16 | 2.787 ns | 2.100 ns |   -25% |
|      64 |        1 | 0.588 ns | 0.315 ns |   -46% |
|      64 |        4 | 0.853 ns | 0.512 ns |   -43% |
|      64 |       16 | 2.018 ns | 1.529 ns |   -27% |

Readers write only to their own 128-byte-aligned hazard record. The
shared state they read (the handler-list pointer, the list itself, and
the retired-list count) changes only when a writer runs. Throughput
therefore scales with thread count until the cores are saturated.

### `contention/notify` (every handler increments a shared counter)

In this group (and in `notify/contended` in `benches/sync_notify.rs`)
every handler increments an `AtomicU64` that all notifying threads
share. Those numbers mostly measure cache-line contention on that
counter, not the registry. A faster registry can even look slower here,
because each thread reaches the contended counter more often. Read
these cells as "registry plus a contended atomic write per handler",
and use `pure_read` to judge the registry itself.

| Threads | Handlers |    1.1.0 | Change vs 1.0.0 |
|--------:|---------:|---------:|----------------:|
|       1 |        1 |  8.45 ns |            -37% |
|       1 |        4 | 21.74 ns |            -16% |
|       1 |       16 | 73.89 ns |             -4% |
|       4 |        1 | 10.90 ns |            -21% |
|       4 |        4 | 30.93 ns |             +6% |
|       4 |       16 | 133.9 ns |            +18% |
|      16 |        1 | 12.31 ns |            -44% |
|      16 |        4 | 37.17 ns |             +3% |
|      16 |       16 | 149.2 ns |             -3% |
|      64 |        1 | 14.07 ns |             -8% |
|      64 |        4 | 43.18 ns |            -14% |
|      64 |       16 | 165.9 ns |             +4% |

The 4-handler, 16-thread cell (37.2 ns) stays under the `<50 ns`
contract target even with the shared-counter write in every handler.

---

## Async notify

Concurrent `notify` builds one future per handler and polls them all in
place; `notify_sequential` awaits each handler in turn. Both include the
boxed-future allocation per handler, which is the floor for type-erased
async handlers.

| Scenario                                  |     1.0.0 |    1.1.0 | Change | Contract target |
|-------------------------------------------|----------:|---------:|-------:|----------------:|
| `notify` *(concurrent)*, **0 handlers**   |  10.95 ns |  6.57 ns |   -41% | (no target)     |
| `notify` *(concurrent)*, **1 handler**    | 169.95 ns | 41.24 ns |   -76% | `<500 ns`       |
| `notify` *(concurrent)*, **4 handlers**   |  341.3 ns | 160.3 ns |   -54% |                 |
| `notify` *(concurrent)*, **16 handlers**  |  1.341 µs | 497.9 ns |   -62% |                 |
| `notify_sequential`, **0 handlers**       |  11.06 ns |  6.21 ns |   -43% |                 |
| `notify_sequential`, **1 handler**        |  52.34 ns | 35.46 ns |   -32% |                 |
| `notify_sequential`, **4 handlers**       |  227.4 ns | 103.7 ns |   -54% |                 |
| `notify_sequential`, **16 handlers**      |  884.3 ns | 469.9 ns |   -52% |                 |

Heap allocations per concurrent `notify` with `n` handlers:

- **1.0.0:** `3n + 4`.
- **1.1.0:** `n + 1` for `n > 1`, `n` for one handler, none for an
  empty registry.

In 1.0.0 each handler future was boxed three times (handler wrapper,
`CatchUnwind`, `JoinAll`) and every call built four `Vec`s. In 1.1.0
the handler's own boxed future is polled in place, a single-handler
call needs no join state, and an empty registry is detected without
cloning the handler list.

For handlers that never actually `.await`, `notify_sequential` is still
the cheaper call at every size. Concurrent dispatch pays off when
handlers do real `.await` work — see
`examples/async_concurrent_vs_sequential.rs` for the wall-clock
comparison (50 ms sleep per handler → ~50 ms concurrent vs ~200 ms
sequential).

### Choosing a dispatch mode

| If your handlers...                        | Use                                  |
|--------------------------------------------|--------------------------------------|
| are `async fn` but never `.await` anything | `notify_sequential` (lower overhead) |
| `.await` real I/O or sleeps                | `notify` (concurrent)                |
| must observe strict happens-before order   | `notify_sequential` (always)         |

---

## Register / unregister (slow path)

Writes are copy-on-write: each one builds a new handler `Vec`, so cost
scales linearly with the current handler count `N`. The notify hot path
is **never** blocked by a write.

| N (existing handlers) | `register` 1.0.0 | `register` 1.1.0 | Change | `unregister` 1.0.0 | `unregister` 1.1.0 | Change |
|----------------------:|-----------------:|-----------------:|-------:|-------------------:|-------------------:|-------:|
|                     0 |         295.7 ns |         185.3 ns |   -41% |                  — |                  — |      — |
|                     1 |                — |                — |      — |           280.7 ns |           154.0 ns |   -46% |
|                    16 |         678.4 ns |         614.8 ns |   -10% |           645.1 ns |           570.6 ns |    -3% |
|                   100 |          2.65 µs |          2.89 µs |    +7% |            2.72 µs |            2.45 µs |    -9% |
|                  1000 |         24.74 µs |         27.75 µs |   +12% |           25.02 µs |           23.97 µs |    -4% |

At large `N` the cost is dominated by cloning every handler `Arc` into
the new list (one atomic increment each, plus the matching decrements
when the old list is freed), the same work 1.0 did. `register` into
100 to 1000 handlers is slightly slower in 1.1.0; that is an accepted
trade on the rare control-plane path for the faster read path.
`unregister` now locates the id once and copies the slices on either
side of it into an exactly-sized `Vec`, and is faster at every size.

`unregister` of an unknown id and `clear` of an empty registry swap
nothing and allocate nothing.

The Performance Contract target was `<1 µs` for the slow path. For the
typical "small registry" (`N ≤ 16`) we're under it; for `N = 100` we're
about 2.5× to 3× the target. This is the documented and intentional
cost of the lock-free read path. If your workload churns thousands of
handlers per second through a 100+ handler registry, consider batching
registrations or maintaining multiple smaller registries instead.

---

## Zero-allocation verification

`tests/zero_alloc.rs` uses [`dhat`](https://crates.io/crates/dhat) to
verify that `SyncRegistry::notify` performs **zero** heap allocations on
the no-panic hot path. Two scenarios are exercised:

1. **Empty registry**, 100 000 `notify(&v)` calls — `0` new blocks, `0`
   new bytes.
2. **8 registered handlers**, 100 000 `notify(&v)` calls — `0` new
   blocks, `0` new bytes.

Both pass on 1.1.0. Run yourself:

```bash
cargo test --features dhat-heap --test zero_alloc
```

The `dhat-heap` feature swaps the global allocator to `dhat::Alloc`, so
it is **off by default** to keep regular tests and benchmarks free of
profiling overhead.

The first `notify` on a thread that has never dispatched allocates that
thread's hazard record once (128 bytes). Records are never freed and are
reused by later threads, so this is a one-time cost bounded by the peak
number of concurrently dispatching threads, not a per-call allocation.

---

## Cost model

| Operation                                  | Cost                                      | Allocates?       | Locks?                    |
|--------------------------------------------|-------------------------------------------|:----------------:|:-------------------------:|
| `SyncRegistry::new` / `with_capacity`      | one `Arc<Vec>` alloc                      | ✓                | —                         |
| `SyncRegistry::register*`                  | `O(N)` copy + swap                        | ✓                | writer mutex              |
| `SyncRegistry::unregister`                 | `O(N)` copy + swap                        | ✓ (if found)     | writer mutex              |
| `SyncRegistry::clear`                      | one `Arc<Vec>` alloc + swap               | ✓ (if non-empty) | writer mutex              |
| `SyncRegistry::*_and_wait`                 | as above, then wait for in-flight readers | as above         | writer mutex, then blocks |
| `SyncRegistry::handler_count` / `is_empty` | hazard-protected read                     | —                | —                         |
| `SyncRegistry::contains`                   | hazard-protected `O(N)` scan              | —                | —                         |
| `SyncRegistry::panic_count`                | one relaxed atomic load                   | —                | —                         |
| `SyncRegistry::notify` (no panics)         | `O(N)` virtual calls + `catch_unwind`     | —                | —                         |
| `SyncRegistry::notify` (handler panics)    | + one `Box<dyn Any>` per panic            | ✓ on panic       | callback read lock        |
| `AsyncRegistry::notify` (concurrent)       | `O(N)` boxed futures + join state         | ✓ (`n + 1`)      | —                         |
| `AsyncRegistry::notify_sequential`         | `O(N)` boxed futures, awaited in turn     | ✓ (`n`)          | —                         |
| `HandlerGuard::drop`                       | one `unregister` call                     | ✓                | writer mutex              |

`N` is the number of currently-registered handlers. "Writer mutex" is
the per-registry lock that serializes writers; readers never take it
except on the fallback path described below.

---

## Hot path: what `notify` actually does

```rust
#[inline]
pub fn notify(&self, event: &E) {
    self.handlers.read(|list| {               // hazard-protected borrow
        for entry in list {                   // straight-line scan
            let handler = &entry.handler;     // borrow Arc<dyn Fn>
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| handler(event))) {
                self.handle_panic(entry.id, payload); // cold path
            }
        }
    });
}
```

`SnapCell::read` on the no-panic path:

1. Thread-local lookup of the calling thread's hazard record (claimed
   on the thread's first dispatch).
2. Scan the record's 8 slots for a free one (almost always the first).
3. One `SeqCst` store: announce the current handler-list pointer.
4. One `SeqCst` load: validate that the pointer is still current
   (retry with the new pointer if a writer swapped in between).
5. Borrow the list in place and run the loop above. No reference
   count is touched.
6. One release store: clear the slot.
7. One relaxed load of the retired-list count. Only if it is non-zero
   does the reader try (non-blocking `try_lock`) to free retired lists
   that no thread announces.

There is no `Mutex`, no `RwLock`, no channel send, no reference-count
increment, and no per-iteration allocation. Measured cost decomposition:

```
fixed per notify:  ~5.4 ns  (0-handler notify, includes the call itself)
per handler:       ~1.2 ns marginal cost
                   (Arc deref + vtable + indirect call
                    + catch_unwind setup/teardown)
```

`#[cold]` on `handle_panic` keeps the panic-handling branch out of the
hot instruction cache.

### Fallback path

If the calling thread has no free slot (dispatch nested more than 8
deep, or the thread's thread-local storage is already being torn down)
`read` clones the current `Arc` under the writer lock and iterates that
instead. It is slower, but correct, and is counted per registry so that
`unregister_and_wait` can see such readers. Ordinary code never reaches
it.

---

## Slow path: register / unregister

`register*`, `unregister`, and `clear` all go through `SnapCell::update`:

1. Take the per-registry writer mutex. Readers never take it.
2. Build the new `Vec` from the current one (one allocation; cloning
   each entry is an `Arc` refcount bump). For `register` the entry is
   inserted at its priority position; for `unregister` the slices on
   either side of the removed id are copied. If there is nothing to do
   (unknown id, already empty), return without swapping or allocating.
3. Store the new list pointer (`SeqCst`).
4. Push the old list onto the retired list.
5. Scan every thread's announcements and take out each retired list
   that no thread announces.
6. Release the mutex, then drop the lists taken out in step 5. Handler
   `Drop` code therefore always runs outside the lock.

Writers never wait for readers. A retired list that is still announced
stays on the retired list until the last reader using it (or the next
write) frees it. Writers are serialized, so there are no
compare-and-swap retries.

`register_with_priority` finds the insertion point with a binary search
(`Vec::partition_point`), so the priority ordering invariant is
maintained without a full re-sort.

The scan in step 5 is `O(threads × 8)` and touches only the write path.

---

## Memory footprint

- An **empty `SyncRegistry`**: 96 bytes inline on x86_64 (handler-list
  pointer, retired-list counter, fallback-reader counter, writer mutex
  with its retired `Vec`, id generator, panic-callback `RwLock`, panic
  counter) plus one `Arc<Vec<...>>` allocation of 40 bytes. Well under
  the 128-byte target for the inline part.
- Per **registered handler**: `HandlerId` (8 B) + priority (4 + 4 B
  padding) + `Arc<dyn Fn>` (16 B) = **32 bytes** per slot. 100 handlers
  ≈ 3.2 KiB + per-handler closure allocation. Comfortably under the
  16 KiB target.
- **Process-wide**: one 128-byte hazard record per thread that has
  dispatched, reused after the thread exits. The record count is
  bounded by the peak number of threads dispatching at the same time.
- **Retired lists**: a replaced list lives until no thread announces
  it. Without a long-running handler on another thread, it is freed by
  the write that replaced it.

---

## Reproducing these numbers

```bash
# Sync hot path scaling
cargo bench --bench sync_notify

# Slow path (register / unregister)
cargo bench --bench register_unregister

# Sync notify under thread contention (contention/notify and pure_read)
cargo bench --bench contention

# Async path (concurrent + sequential)
cargo bench --bench async_notify --features async

# Zero-allocation verification
cargo test --features dhat-heap --test zero_alloc
```

Criterion writes HTML reports under `target/criterion/`.

The published numbers used
`-- --warm-up-time 1 --measurement-time 3`. To compare against an older
version, save a baseline on that version with `-- --save-baseline old`
and run the current code with `-- --baseline old`.

---

## Concurrency characteristics

- **Many simultaneous readers**: `notify` from any number of threads in
  parallel is supported with no shared writes between readers. Each
  reader announces into its own record and borrows a list that no
  writer can mutate. Measured `pure_read` at 64 threads and 4 handlers:
  0.51 ns per notify across all threads.
- **Reader + writer concurrency**: a register or unregister concurrent
  with notify never causes a notify to skip or duplicate handlers — each
  notify iterates one complete list.
- **Many simultaneous writers**: serialized on the per-registry writer
  mutex. Each write is applied exactly once; there are no retries.
- **Writers inside handlers**: a handler may register or unregister
  (including itself) during dispatch. Writers never wait for readers,
  so this cannot deadlock. `unregister_and_wait` called from inside a
  handler does not wait for its own thread.

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
  the current snapshot is and may not converge. Beyond 8 nested levels
  per thread, reads also take the slower locked fallback path.
- **Large per-handler captures**: each registration becomes an
  `Arc<dyn Fn>` heap allocation. A handler that captures a 1 MB buffer
  costs 1 MB per registration. Keep captures small; share via `Arc`.
- **Thousands-of-handlers registries**: the slow path scales linearly
  with `N`. A registry with 10 000 handlers will have roughly 280 µs
  register latency. If that matters, partition into multiple smaller
  registries.
- **`unregister_and_wait` in async code**: it blocks the thread. Use
  `unregister` from async tasks.

---

<sub>registry-io v1.1.0 — Copyright © 2026 James Gober. Apache-2.0 OR MIT.</sub>

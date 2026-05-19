# registry-io — Architecture

This document walks through how `registry-io` is built internally:
the storage model, the hot and slow paths, how the async side
mirrors the sync side, the trade-offs in each design decision, and
the file-tree map so a new contributor can find anything in under
30 seconds.

If you only want the public surface, see [`API.md`](./API.md). For
measured costs, see [`PERFORMANCE.md`](./PERFORMANCE.md).

---

## Big picture

```
┌─────────────────────────── SyncRegistry<E> ─────────────────────────┐
│                                                                     │
│  ArcSwap<Vec<HandlerEntry<E>>>     ◄─── lock-free snapshot read     │
│  ┌──────────────────────────┐                                       │
│  │ HandlerEntry { id, prio, │                                       │
│  │   handler: Arc<dyn Fn> } │   ◄─── one entry per registered       │
│  │ HandlerEntry { … }       │       handler, priority-sorted        │
│  │ HandlerEntry { … }       │                                       │
│  └──────────────────────────┘                                       │
│                                                                     │
│  HandlerIdGenerator { next: AtomicU64 }   ◄─── monotonic id counter │
│  ArcSwapOption<PanicCallbackHolder>       ◄─── optional on_panic    │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘

  notify(&event):
    1. snapshot = handlers.load()      // atomic acquire load
    2. for entry in snapshot.iter() {
         catch_unwind(|| handler(event))
         if Err(payload) { handle_panic(id, payload) }
       }

  register(F):
    1. id = next id
    2. handlers.rcu(|current| {
         clone → insert at priority position → return new Arc<Vec>
       })
```

The async side is structurally identical, with `Arc<dyn Fn(&E) ->
BoxFuture<()>>` replacing `Arc<dyn Fn(&E)>` and the notify hot path
running each future through a panic-catching adapter (`CatchUnwind`)
plus an optional concurrent combinator (`JoinAll`).

---

## File-tree map

```
src/
├── lib.rs                 — crate root; module declarations + re-exports
├── handler_id.rs          — opaque HandlerId + monotonic generator
├── panic.rs               — PanicInfo<'a>, PanicCallbackHolder (shared)
├── future_ext.rs          — CatchUnwind, JoinAll  (feature: async)
├── sync/
│   ├── mod.rs             — SyncRegistry<E>
│   └── guard.rs           — HandlerGuard<E>  (RAII unregister)
└── async_registry/
    ├── mod.rs             — AsyncRegistry<E>   (feature: async)
    └── guard.rs           — AsyncHandlerGuard<E>

tests/
├── smoke.rs               — minimal end-to-end
├── sync_registry.rs       — sync core, 16 tests
├── priority.rs            — sync priority ordering, 6 tests
├── panic_isolation.rs     — sync panic isolation, 9 tests
├── guards.rs              — HandlerGuard RAII, 8 tests
├── concurrent.rs          — sync multi-thread, 6 tests
├── async_registry.rs      — async core, 10 tests   (feature: async)
├── async_priority.rs      — async priority, 3 tests
├── async_panic.rs         — async panic isolation, 11 tests
├── async_guards.rs        — AsyncHandlerGuard, 7 tests
├── proptest_invariants.rs — property tests, 6 properties
├── leak_check.rs          — Arc::strong_count canary, 3 scenarios
└── zero_alloc.rs          — dhat zero-allocation verification

benches/
├── sync_notify.rs           — notify by handler count, by thread count
├── register_unregister.rs   — slow path latency at N = 0/16/100/1000
├── contention.rs            — {1,4,16,64} threads × {1,4,16} handlers
└── async_notify.rs          — concurrent + sequential at N = 0/1/4/16

examples/
├── basic.rs                          — register / notify / unregister
├── priority.rs                       — priority ordering
├── guards.rs                         — HandlerGuard RAII
├── panic_isolation.rs                — panic-isolation + on_panic
├── concurrent.rs                     — 16 threads, lock-free contention
├── async_basic.rs                    — async fn handlers (feat: async)
├── async_concurrent_vs_sequential.rs — dispatch-mode comparison
├── pattern_hot_reload.rs             — config-lib-style hot reload
├── pattern_audit_fanout.rs           — audit-log fan-out
├── pattern_metric_event.rs           — metric-event collection
└── pattern_transaction_hooks.rs      — priority-ordered tx hooks

fuzz/
├── Cargo.toml
└── fuzz_targets/
    ├── handler_churn.rs       — random op sequences
    └── event_payload.rs       — adversarial event values
```

---

## Storage model

### `ArcSwap<Vec<HandlerEntry<E>>>`

The single most important design decision: handlers live in a `Vec`
held behind an
[`arc_swap::ArcSwap`](https://docs.rs/arc-swap/latest/arc_swap/struct.ArcSwap.html).

`ArcSwap` is a published primitive for **wait-free reads** and
**atomic copy-on-write writes** of an `Arc<T>`. A read just loads an
`Arc<T>` from an atomic pointer; a write produces a new `Arc<T>` and
compare-and-swaps it into the slot.

Properties this gives us:

- **Readers never block writers, writers never block readers.** A
  thread firing `notify` and a thread calling `register` proceed in
  parallel with no coordination.
- **Each `notify` sees a consistent snapshot.** The `Vec` it iterates
  over cannot be mutated mid-iteration because every write replaces
  the whole `Arc<Vec>` atomically.
- **Reader cost is one atomic acquire load** (plus a thread-local
  cache fast-path inside `arc-swap`).

### `HandlerEntry<E>`

```rust
struct HandlerEntry<E: Send + Sync + 'static> {
    id: HandlerId,                                       // 8 B
    priority: i32,                                       // 4 B + 4 B padding
    handler: Arc<dyn Fn(&E) + Send + Sync + 'static>,    // 16 B
}
// 32 bytes total, fits in half a cache line.
```

Cloning a `HandlerEntry` is one `HandlerId` copy + one `i32` copy +
one `Arc::clone` (a refcount bump). Cloning the full `Vec` during
register/unregister is therefore `O(N)` *cheap* operations, not `O(N)`
*allocations*.

### `HandlerIdGenerator`

```rust
pub(crate) struct HandlerIdGenerator { next: AtomicU64 }
```

`fetch_add(1, Relaxed)` produces a monotonic id stream starting at
`1`. `Relaxed` ordering is sufficient because the only invariant is
that *each call returns a distinct value*, which `fetch_add` provides
atomically. No happens-before relation is needed between the id
allocation and other registry state.

Two registries each have their own generator. Ids are not
comparable across registries.

### Panic callback storage

```rust
pub(crate) struct PanicCallbackHolder {
    inner: Arc<dyn Fn(&PanicInfo<'_>) + Send + Sync + 'static>,
}
```

The wrapper exists because `arc-swap`'s `RefCnt` trait requires the
inner type to be `Sized`, but `dyn Fn` is not. Wrapping in a sized
holder lets us use
`ArcSwapOption<PanicCallbackHolder>` for atomic install/replace/clear.

Reads of the panic callback happen on the **cold** path (inside
`handle_panic`, after a handler has already panicked) so the
single-load cost of `ArcSwapOption::load` is irrelevant.

---

## The hot path: `SyncRegistry::notify`

```rust
#[inline]
pub fn notify(&self, event: &E) {
    let snapshot = self.handlers.load();
    for entry in snapshot.iter() {
        let handler = &entry.handler;
        let result = catch_unwind(AssertUnwindSafe(|| handler(event)));
        if let Err(payload) = result {
            self.handle_panic(entry.id, payload);
        }
    }
}
```

Cost decomposition (measured, see `PERFORMANCE.md`):

| Stage                              | Approximate cost |
|------------------------------------|------------------|
| `handlers.load()`                  | ~2 ns (one atomic acquire, thread-local cached) |
| Per-entry: deref + iter step       | ~0.5 ns          |
| Per-entry: `Arc<dyn Fn>` deref + vtable + indirect call | ~1 ns |
| Per-entry: `catch_unwind` setup/teardown (no panic) | varies by OS |
| Per-entry: total marginal          | **~1.6 ns**      |

`handle_panic` is `#[cold]` so the linker keeps it out of the hot
instruction cache. The Err branch is taken essentially never on a
well-behaved handler set.

### Why `AssertUnwindSafe`?

`catch_unwind` requires its closure to implement `UnwindSafe`. Closures
that capture mutable references to non-`UnwindSafe` types (which most
trait objects technically are) don't satisfy this. We use
`AssertUnwindSafe` to bypass the static check.

The safety reasoning is documented in `docs/SECURITY.md`: the
registry's own state lives behind an immutable `Arc<Vec<...>>` snapshot
during iteration, so a panicking handler cannot corrupt it.

### Why no `notify_trusted` variant?

We considered an opt-out for `catch_unwind` ("if you trust your
handlers, save the cost"). Measured numbers showed the saving is
negligible (`catch_unwind` is essentially free on the no-panic path
across all our supported targets). Maintaining two variants of the
hot path was not worth the imperceptible win.

---

## The slow path: `register` / `unregister` / `clear`

All three use `ArcSwap::rcu` — the standard read-copy-update pattern:

```rust
drop(self.handlers.rcu(|current| {
    let mut new_vec: Vec<_> = Vec::with_capacity(current.len() + 1);
    new_vec.extend(current.iter().cloned());
    // … modify new_vec …
    Arc::new(new_vec)
}));
```

`rcu` loads the current `Arc<Vec>`, runs the closure to produce a new
`Arc<Vec>`, and compare-and-swaps it into the slot. If the CAS
fails — because another writer raced — `rcu` retries from the load.

Properties:

- **Linearizable across writers**: every write either lands or is
  retried; no write is lost.
- **`O(N)` per write** in the number of handlers (one `Vec` allocation
  + N `Arc::clone`s + one CAS).
- **Reader-side has zero impact on writers**: the read-side `Guard`
  from a concurrent `notify` does not block the writer's CAS.

### Priority-sorted insertion

```rust
let pos = new_vec.partition_point(|e| e.priority >= entry.priority);
new_vec.insert(pos, entry.clone());
```

`partition_point` is a binary search for the first index where the
predicate flips from `true` to `false`. Inserting at that index
keeps the vec sorted by descending priority with **stable** ordering
within priority bucket.

We chose `partition_point + insert` (= `O(log N + N)`) over a full
re-sort (= `O(N log N)`) because the rest of the slow path is already
`O(N)` and binary-search insertion preserves stability without a
custom comparator.

---

## The async path: `AsyncRegistry`

Structurally a clone of `SyncRegistry` with the handler signature
swapped:

```rust
type StoredAsyncHandler<E> =
    Arc<dyn Fn(&E) -> BoxFuture<()> + Send + Sync + 'static>;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
```

The returned future is **`'static`** — it cannot borrow from `&E`.
Handlers that need event data must `clone` it inside the closure
before the inner `async move`. This is the canonical Rust async-fn
limitation; the registry doesn't try to paper over it.

### `CatchUnwind` (in `src/future_ext.rs`)

```rust
struct CatchUnwind<F: Future> {
    inner: Option<Pin<Box<F>>>,
}
```

Per-poll, `CatchUnwind` wraps `inner.as_mut().poll(cx)` in
`catch_unwind`. On panic, the inner future is consumed (set to `None`)
and the panic payload is returned as `Err`. On future completion the
inner is likewise consumed. The `Option` discriminant guards against
the otherwise-illegal "poll a Ready future" case.

### `JoinAll` (in `src/future_ext.rs`)

```rust
struct JoinAll<F: Future> {
    slots: Vec<JoinSlot<F>>,
    remaining: usize,
}
enum JoinSlot<F: Future> { Pending(Pin<Box<F>>), Done(F::Output) }
```

Polls every still-`Pending` slot per wake; transitions slots to
`Done` as they resolve; yields a `Vec<F::Output>` once `remaining ==
0`. Order of outputs is preserved.

This is the **minimal** concurrent driver. We don't pull in
`futures-util` (`join_all`, `select_all`, etc.) because we only need
this one combinator and the dependency carries its own non-trivial
surface. ~50 lines in-tree was the right trade-off.

### Two dispatch modes

```rust
pub async fn notify(&self, event: &E)             // concurrent
pub async fn notify_sequential(&self, event: &E)  // in priority order
```

`notify`: builds one wrapped future per handler, drives them
concurrently through `JoinAll`. Total wall-clock equals the slowest
handler.

`notify_sequential`: awaits each handler's future to completion
before starting the next. Total wall-clock equals the sum of
handler latencies but preserves a happens-before relation.

See `docs/PATTERNS.md#choosing-between-sync-and-async` for the
decision matrix.

---

## RAII guards

```rust
pub struct HandlerGuard<E: Send + Sync + 'static> {
    id: HandlerId,
    registry: Weak<SyncRegistry<E>>,
}
```

The guard holds a `Weak<SyncRegistry<E>>`, not an `Arc`. This breaks
a potential cycle (handler closure → captures `Arc<Self>` → owns
guard → holds Arc<Self>) and makes registry-before-guard drop a
no-op.

`Drop::drop` upgrades the `Weak`; if successful, calls
`registry.unregister(self.id)`. The `_ =` discard on the return
value is intentional — the handler may already have been removed by
a different code path.

`forget(self)` consumes the guard via `ManuallyDrop` so the registry
keeps the handler past the guard's scope. The caller is responsible
for unregistering manually after this.

---

## Cross-cutting design decisions

### Why `E: Send + Sync + 'static` at the struct level?

Putting the bound on the type (`pub struct SyncRegistry<E: Send + Sync
+ 'static>`) instead of on each impl block means:

- `Drop` impls can call methods (Rust requires the Drop impl's where
  clause to match the type's). Without this bound on the type,
  `HandlerGuard::drop` couldn't call `registry.unregister`.
- The user gets a clearer error at the construction site than at the
  point of `.register(...)`.
- The type is uniformly `Send + Sync` across all impls.

### Why monotonic ids instead of generational arenas?

The straight-forward `slotmap`/`generational-arena` approach would
let us re-use slot indices and provide compile-time-checked
liveness. We chose the `u64` counter for three reasons:

1. **Simpler invariant**: "every id ever issued is unique" is easier
   to reason about than "id is a (index, generation) pair, both of
   which can wrap."
2. **`HandlerId: Copy`** for free, with cheap equality (single `u64`
   compare).
3. **No re-use means no false positives.** Stale ids returned from
   `unregister` reliably stay rejected, which the property tests
   guarantee.

The downside — a 32-bit-counter would wrap after 4 billion
registrations — is mitigated by using `u64`, which wraps at
~10^19 registrations. At 1M registrations/sec that's ~580 000 years.

### Why panic isolation by default (no opt-out)?

A `notify` that propagates panics couples every subscriber to every
other subscriber. The "I'll be careful" argument always loses at
scale: every team that uses the registry would have to verify every
handler everywhere they touch. Catching the panic is the only
sustainable default.

The measured cost of `catch_unwind` on the no-panic path is
negligible on our supported targets (see `PERFORMANCE.md`), so the
trade-off is essentially free.

### Why no `tokio`/`async-std` runtime dependency?

`AsyncRegistry` is generic over whatever async runtime polls its
futures. The crate doesn't `spawn`, doesn't have a worker pool, and
doesn't care which executor drives `notify().await`. Pulling in a
runtime would force every consumer into the same one.

The dev-dependency `tokio` is for tests/examples/benches only and
does not propagate to downstream crates.

---

## Adding a new feature: the checklist

When adding to `registry-io`, the in-tree convention is:

1. **Public API change** → update [`STABILITY-1.0.md`](./STABILITY-1.0.md)
   if it's a major bump territory. Otherwise add the new item under
   the appropriate section.
2. **Hot path change** → add a benchmark scenario before submitting
   the change. The regression gate is `>5%` on any tracked metric.
3. **Allocation behavior change** → ensure
   `cargo test --features dhat-heap --test zero_alloc` still passes.
4. **Async surface change** → mirror against the sync surface unless
   there's an explicit reason not to.
5. **Doc** → at minimum: a one-line summary, `# Examples` with a
   runnable example, and an entry in `docs/API.md` for any new public
   item. Update `docs/PATTERNS.md` if the new item introduces a new
   integration pattern.
6. **CHANGELOG** → entry under `[Unreleased]` describing what
   changed and why. Include a fix-up line if the change is a
   correction to a prior release's behavior.
7. **Run the full gate**: `cargo fmt --all -- --check`,
   `cargo clippy --all-targets --all-features -- -D warnings`,
   `cargo test --all-features`, `RUSTDOCFLAGS="-D warnings" cargo doc
   --no-deps --all-features`, `cargo build --all-features --examples`.

---

<sub>registry-io v0.9.0 — Copyright © 2026 James Gober. Apache-2.0 OR MIT.</sub>

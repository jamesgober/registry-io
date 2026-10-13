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
│  handlers: SnapCell<Vec<HandlerEntry<E>>>                           │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ current: AtomicPtr ──────► Arc<Vec<HandlerEntry<E>>>          │  │
│  │                            ┌──────────────────────────┐       │  │
│  │                            │ HandlerEntry { id, prio, │       │  │
│  │                            │   handler: Arc<dyn Fn> } │       │  │
│  │                            │ HandlerEntry { … }       │       │  │
│  │                            └──────────────────────────┘       │  │
│  │ retired_len: AtomicUsize       ◄─── readers check, rarely > 0 │  │
│  │ fallback_readers: AtomicUsize  ◄─── readers without a slot    │  │
│  │ writer: Mutex<Vec<Arc<..>>>    ◄─── writers only; retired     │  │
│  │                                     lists awaiting free       │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                     │
│  id_generator: HandlerIdGenerator { next: AtomicU64 }               │
│  panic_callback: RwLock<Option<Arc<PanicCallbackHolder>>>           │
│  panics: AtomicU64                          ◄─── panic_count()      │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘

┌──────────────── hazard domain (process-wide, src/hazard.rs) ────────┐
│                                                                     │
│  HEAD ──► Record ──► Record ──► Record ──► null   (append-only)     │
│           slots[8]   slots[8]   slots[8]          one per thread,   │
│           in_use     in_use     in_use            128-byte aligned, │
│                                                   reused on exit    │
└─────────────────────────────────────────────────────────────────────┘

  notify(&event):
    1. p = current.load()
    2. announce p in this thread's next free slot     // SeqCst store
    3. if current.load() != p, re-announce and retry   // SeqCst load
    4. for entry in *p {
         catch_unwind(|| handler(event))
         if Err(payload) { handle_panic(id, payload) }
       }
    5. clear the slot                                  // release store
    6. if retired_len != 0 { try_lock and free unannounced lists }

  register(F):
    1. id = next id
    2. lock writer mutex
    3. build new Vec (insert at priority position), store as current
    4. push old list onto retired; scan all slots; take out
       every retired list nobody announces
    5. unlock, then drop what was taken out
```

The async side uses the same `SnapCell`, with `Arc<dyn Fn(&E) ->
BoxFuture<()>>` replacing `Arc<dyn Fn(&E)>` and the notify path
running each future through panic-catching helpers that poll it in
place (`src/future_ext.rs`).

---

## File-tree map

```
src/
├── lib.rs                 — crate root; module declarations + re-exports
├── handler_id.rs          — opaque HandlerId + monotonic generator
├── panic.rs               — PanicInfo<'a>, PanicCallbackHolder (shared)
├── hazard.rs              — process-wide hazard-pointer domain (unsafe)
├── snapshot.rs            — SnapCell<T>: hazard-protected Arc<T> (unsafe)
├── future_ext.rs          — start, CatchUnwind, Slot  (feature: async)
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
├── wait_and_once.rs       — *_and_wait, register_once, panic_count, 11 tests
├── async_registry.rs      — async core, 9 tests   (feature: async)
├── async_priority.rs      — async priority, 3 tests
├── async_panic.rs         — async panic isolation, 10 tests
├── async_start_panic.rs   — panic while building a future, 3 tests
├── async_guards.rs        — AsyncHandlerGuard, 7 tests
├── proptest_invariants.rs — property tests, 6 properties
├── leak_check.rs          — Arc::strong_count canary, 3 scenarios
└── zero_alloc.rs          — dhat zero-allocation verification

benches/
├── sync_notify.rs           — notify by handler count, by thread count
├── register_unregister.rs   — slow path latency at N = 0/16/100/1000
├── contention.rs            — {1,4,16,64} threads × {1,4,16} handlers;
│                              contention/notify (shared counter) and
│                              pure_read (no shared writes)
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

`src/hazard.rs` and `src/snapshot.rs` are the only files that contain
`unsafe`. Every block carries a `// SAFETY:` comment; see
[`SECURITY.md`](./SECURITY.md#unsafe-code-discipline).

---

## Storage model

### `SnapCell<Vec<HandlerEntry<E>>>`

The single most important design decision: handlers live in a `Vec`
behind an `Arc`, and the `Arc` is published through a crate-internal
`SnapCell<T>` (`src/snapshot.rs`). Every write replaces the whole list;
no list is ever mutated after it is published.

```rust
pub(crate) struct SnapCell<T> {
    current: AtomicPtr<T>,              // from Arc::into_raw, never null
    retired_len: AtomicUsize,           // retired lists not yet freed
    fallback_readers: AtomicUsize,      // readers on the locked fallback
    writer: Mutex<Vec<Arc<T>>>,         // serializes writers; retired lists
}
```

Properties this gives us:

- **Readers never block writers, writers never wait for readers.** A
  thread firing `notify` and a thread calling `register` proceed in
  parallel. Readers never take the writer mutex (except on the
  fallback path below).
- **Each `notify` sees a consistent snapshot.** The `Vec` it iterates
  cannot change mid-iteration because writers replace the whole list.
- **Reads touch no reference count.** A reader borrows the list in
  place under a hazard announcement instead of cloning the `Arc`, so
  concurrent readers do not contend on a shared counter.
- **Writers cannot deadlock against handlers.** A handler that
  unregisters itself or another handler during dispatch takes the
  writer mutex, swaps, and returns; it never waits for the dispatch it
  is running inside.

### The hazard domain (`src/hazard.rs`)

A *hazard pointer* is a per-thread announcement: "I am reading the
object at this address; do not free it."

- Each thread that dispatches gets one `Record` holding 8 announcement
  slots, one per level of nested dispatch (a handler that calls
  `notify` uses the next slot).
- Records are `#[repr(align(128))]`, so no two threads' records share
  a cache line or an adjacent-line prefetch pair. Without this, a
  thread publishing an announcement would stall every thread whose
  record sits on the same line.
- Records live in an append-only, lock-free linked list. They are
  never freed. When a thread exits, its record is marked free and the
  next new thread reuses it, so the list length is bounded by the
  peak number of threads that were dispatching at the same time.
- This list is the crate's only global state. It has to be global
  because an announcement must be visible to writers on every thread
  for every registry. It is mutated only through atomics.

### Reading

`SnapCell::read(f)`:

1. Load `current` (acquire).
2. Claim the first free slot in this thread's record and store the
   pointer into it (`SeqCst`).
3. Load `current` again (`SeqCst`). If it changed, store the new
   pointer into the same slot and repeat.
4. Call `f(&*p)`: the list is borrowed in place.
5. Clear the slot (release store). The `Pin` type does this in its
   `Drop`, so it also happens when `f` unwinds.
6. Load `retired_len` (relaxed). If it is non-zero, `try_lock` the
   writer mutex and free every retired list no thread announces. If
   the lock is busy, do nothing; a later reader or writer will free
   them.

### Writing

`SnapCell::update(f)`:

1. Lock the writer mutex.
2. Run `f` on the current list. `f` returns `None` to leave the list
   alone (unknown id in `unregister`, empty list in `clear`): nothing
   is swapped and nothing is allocated.
3. Otherwise store the new pointer into `current` (`SeqCst`) and push
   the old `Arc` onto the retired list.
4. Collect every pointer currently announced by any thread (`SeqCst`
   loads), and move each retired list that is not announced out of the
   retired list.
5. Update `retired_len`, unlock, then drop the moved-out lists. User
   `Drop` code (handler closures and their captures being freed) always
   runs after the lock is released, so it may call back into the
   registry.

### Why it is correct

The protocol is the classic hazard-pointer handshake, and both sides
use `SeqCst` so that their operations fall into one total order.

- **Reader:** store announcement *then* load `current`.
- **Writer:** store `current` *then* load every announcement.

Take a reader that announces `p` and a writer that replaces `p` and
then scans. In the `SeqCst` total order, either the reader's
announcement comes before the writer's scan reads that slot — so the
writer sees `p` announced and keeps it — or it comes after. In the
second case the writer's store to `current` precedes the scan, which
precedes the reader's announcement, which precedes the reader's
validating load; that load therefore sees the new pointer, the
validation fails, and the reader retries without ever dereferencing
`p`. Either way, a list is never freed while a reader that validated it
is still using it.

The record list needs the same treatment. A new thread publishes its
record with a `SeqCst` compare-and-swap on the list head before its
first announcement, and a writer's scan starts with a `SeqCst` load of
the head. If the scan misses the new record, the head load precedes the
publication in the total order, so the writer's store to `current`
(earlier still) precedes the new thread's validating load, and that
reader retries. With an acquire load of the head, both sides could miss
each other; Miri's weak-memory emulation found exactly that
interleaving during development, and the head now uses `SeqCst`.

The pointer a reader keeps is the one `Arc::into_raw` produced, never
one re-derived from a `&T`, so it carries provenance over the whole
`Arc` allocation including the reference counts. Miri caught a
violation of this in the owned-load path during development; it is
fixed.

### Fallback path

`hazard::pin` returns `None` when the calling thread has no free slot:
dispatch is nested more than 8 deep, or the thread's thread-local
storage is already being torn down (a `notify` from another
thread-local's destructor). `read` then increments
`fallback_readers`, clones the current `Arc` under the writer lock, runs
`f` on the clone, and decrements the counter. Slower, but correct, and
visible to `wait_for_retired_readers`.

### `wait_for_retired_readers`

Backs `SyncRegistry::unregister_and_wait` and `clear_and_wait`. Called
right after a successful write:

1. If any reader is on the fallback path (and the caller is not one
   itself), it is not yet safe; back off.
2. Otherwise, under the writer lock, copy the pointers of every list on
   the retired list. Release the lock and check whether any thread
   *other than the caller* announces any of them.
3. If none does, return. Otherwise back off and repeat.

The backoff spins 64 rounds, yields 64 rounds, then sleeps with a
growing delay capped at 1 ms.

Consequences, all documented on the public methods:

- Every list that was current before the write is either already freed
  or on the retired list, so a reader still running the removed handler
  is found and waited for. Readers that start afterwards load the new
  list and are not waited for.
- The calling thread's own announcements are skipped, so calling it
  from inside a handler waits for other threads but not for the stack
  frames above it.
- The check covers *every* retired list, including ones retired by
  later writes. The wait can be longer than strictly necessary, never
  shorter.
- If the caller is itself on the fallback path, fallback readers are
  not waited for, because the caller may be one of them.

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
one `Arc::clone` (a refcount bump). Building a new `Vec` during
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

panic_callback: RwLock<Option<Arc<PanicCallbackHolder>>>,
panics: AtomicU64,
```

`on_panic` and `clear_panic_callback` take the write lock and replace
the `Option`. `handle_panic` increments `panics`, takes the read lock
just long enough to clone the `Arc`, and invokes the callback outside
the lock. Both run only on the **cold** path (after a handler has
already panicked), so a lock here costs nothing on the hot path.
Poisoning is ignored: nothing that runs under the lock can leave the
`Option` inconsistent.

---

## The hot path: `SyncRegistry::notify`

```rust
#[inline]
pub fn notify(&self, event: &E) {
    self.handlers.read(|list| {
        for entry in list {
            let handler = &entry.handler;
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| handler(event))) {
                self.handle_panic(entry.id, payload);
            }
        }
    });
}
```

Cost decomposition (measured, see `PERFORMANCE.md`):

| Stage                                                   | Approximate cost |
|---------------------------------------------------------|------------------|
| Fixed per notify (TLS lookup, slot scan, `SeqCst` store, `SeqCst` load, release store, relaxed load) | ~5.4 ns (0-handler notify, including the call) |
| Per-entry: `Arc<dyn Fn>` deref + vtable + indirect call | ~1 ns            |
| Per-entry: `catch_unwind` setup/teardown (no panic)     | varies by OS     |
| Per-entry: total marginal                               | **~1.2 ns**      |

`handle_panic` is `#[cold]` so the linker keeps it out of the hot
instruction cache. The Err branch is taken essentially never on a
well-behaved handler set.

One unwind boundary per handler is deliberate. A single `catch_unwind`
around the whole loop, resuming after a panic, was measured and was
about 36% slower at 16 handlers: the resume index has to live in memory
across the unwind boundary, which costs more than the per-call
`catch_unwind` it saves.

### Why `AssertUnwindSafe`?

`catch_unwind` requires its closure to implement `UnwindSafe`. Closures
that capture mutable references to non-`UnwindSafe` types (which most
trait objects technically are) don't satisfy this. We use
`AssertUnwindSafe` to bypass the static check.

The safety reasoning is documented in `docs/SECURITY.md`: the
registry's own state is an immutable, hazard-protected list during
iteration, so a panicking handler cannot corrupt it, and the hazard
slot is cleared by a `Drop` impl even when a panic unwinds through it.

### Why no `notify_trusted` variant?

We considered an opt-out for `catch_unwind` ("if you trust your
handlers, save the cost"). Measured numbers showed the saving is
negligible (`catch_unwind` is essentially free on the no-panic path
across all our supported targets). Maintaining two variants of the
hot path was not worth the imperceptible win.

---

## The slow path: `register` / `unregister` / `clear`

All three go through `SnapCell::update` (see [Writing](#writing)).
The closures they pass:

```rust
// register_with_priority
let pos = current.partition_point(|e| e.priority >= entry.priority);
let mut next = Vec::with_capacity(current.len() + 1);
next.extend_from_slice(&current[..pos]);
next.push(entry);
next.extend_from_slice(&current[pos..]);

// unregister: ids are unique, so copy the two slices around it
let pos = current.iter().position(|e| e.id == id)?;   // None: no swap
let mut next = Vec::with_capacity(current.len() - 1);
next.extend_from_slice(&current[..pos]);
next.extend_from_slice(&current[pos + 1..]);

// clear
(!current.is_empty()).then(|| (Vec::new(), ()))       // None: no swap
```

Properties:

- **Linearizable across writers**: writers are serialized on the
  mutex, so every write sees the result of the previous one. There are
  no retries.
- **`O(N)` per write** in the number of handlers (one `Vec` allocation
  + N `Arc::clone`s), plus an `O(threads × 8)` announcement scan.
- **Writers never wait for readers.** A retired list that a reader is
  still using stays on the retired list until that reader or a later
  write frees it.

### Priority-sorted insertion

`partition_point` is a binary search for the first index where the
predicate flips from `true` to `false`. Inserting at that index
keeps the vec sorted by descending priority with **stable** ordering
within priority bucket.

We chose `partition_point` + copy (= `O(log N + N)`) over a full
re-sort (= `O(N log N)`) because the rest of the slow path is already
`O(N)` and binary-search insertion preserves stability without a
custom comparator.

### `register_once`

`register_once` wraps the user's `FnOnce` in a `Mutex<Option<F>>`
inside an ordinary `Fn` handler. On dispatch, the handler takes the
`Option` under the lock and calls the closure outside it, so exactly
one concurrent caller gets it and a panicking closure cannot poison
later attempts. A drop guard holding a `Weak<SyncRegistry<E>>` then
unregisters the id, also when the closure panics, without keeping the
registry alive.

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

A hazard slot is bound to a thread and cannot be held across an
`.await`, so async `notify` takes an owned `Arc` of the list instead
(`SnapCell::load_arc_if`: hazard-protected check, then one reference
count increment). An empty registry is detected under the hazard
without touching the reference count, so `notify` on an empty async
registry allocates nothing and clones nothing.

### Panic-isolating helpers (in `src/future_ext.rs`)

Handler futures are already boxed (`Pin<Box<dyn Future>>`) when the
handler returns them, which makes them `Unpin`. The helpers poll them
in place instead of boxing them again:

- **`start(make)`** calls the synchronous part of the handler (the
  closure that builds the future) inside `catch_unwind`. Before 1.1.0
  this call was outside any unwind boundary, so a handler that
  panicked before returning its future escaped `notify`.
- **`poll_caught(fut, cx)`** polls once inside `catch_unwind` and
  turns a panic into `Ready(Err(payload))`.
- **`CatchUnwind`** wraps one boxed future for the single-handler and
  sequential paths. It drops the inner future as soon as it completes
  or panics.
- **`Slot`** is one handler's state in a concurrent dispatch:
  `Running(BoxFuture)`, `Done`, or `Panicked(payload)`.

### Two dispatch modes

```rust
pub async fn notify(&self, event: &E)             // concurrent
pub async fn notify_sequential(&self, event: &E)  // in priority order
```

`notify`: with one handler, awaits it through `CatchUnwind` (no join
state). With more, builds a `Vec<Slot>` (one allocation) and drives it
with `poll_fn`, polling every running slot on each wake until none is
running. Panic payloads are reported to `on_panic` after all handlers
finish, in priority order. Total wall-clock equals the slowest handler.

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

### Why an in-tree hazard domain instead of a dependency?

Through 1.0 the handler list lived in an atomically swappable `Arc`
provided by an external crate. 1.1.0 replaced it with `SnapCell` for
three reasons:

1. **Read cost.** Borrowing in place under a hazard removes the fixed
   per-`notify` cost that dominated small handler counts (0-handler
   notify went from 9.5 ns to 5.4 ns) and scales across threads
   because readers write only to their own cache line.
2. **Waiting for readers.** `unregister_and_wait` needs to know which
   retired lists another thread is still using. The announcements
   provide exactly that.
3. **Dependencies.** With default features the crate now has no
   runtime dependencies.

The cost is `unsafe`-bearing code that this crate must now maintain
itself. It is confined to two internal modules, documented block by
block, and checked with Miri (see `SECURITY.md`).

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
5. **`unsafe` change** (`src/hazard.rs`, `src/snapshot.rs`) → every
   block keeps a `// SAFETY:` comment, and the change is run under Miri
   with weak-memory emulation and several random thread schedules.
6. **Doc** → at minimum: a one-line summary, `# Examples` with a
   runnable example, and an entry in `docs/API.md` for any new public
   item. Update `docs/PATTERNS.md` if the new item introduces a new
   integration pattern.
7. **CHANGELOG** → entry under `[Unreleased]` describing what
   changed and why. Include a fix-up line if the change is a
   correction to a prior release's behavior.
8. **Run the full gate**: `cargo fmt --all -- --check`,
   `cargo clippy --all-targets --all-features -- -D warnings` (and the
   sync-only and async-only feature sets), `cargo test --all-features`,
   `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`,
   `cargo build --all-features --examples`.

---

<sub>registry-io v1.1.0 — Copyright © 2026 James Gober. Apache-2.0 OR MIT.</sub>

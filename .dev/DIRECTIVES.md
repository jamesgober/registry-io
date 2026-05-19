# registry-io - Directives

> Project-specific engineering directives. Apply on top of REPS and the portfolio universal directives.

---

## Priority order

1. `REPS.md` at repo root - **SUPREME AUTHORITY**
2. `_strategy/UNIVERSAL_PROMPT.md` - portfolio-wide directives
3. This file - registry-io specific directives
4. `.dev/PROMPT.md` - project context
5. `.dev/ROADMAP.md` - current phase and tasks

REPS overrides everything else.

---

## Performance discipline (the central concern)

This crate exists because of performance. If you write code that violates the performance contract, you're working against the purpose of the crate.

### Non-negotiable

- **`notify()` allocates ZERO bytes on the hot path.** Verified by `dhat`.
- **`notify()` takes no locks on the hot path.** Verified by reading the code; no `Mutex::lock`, no `RwLock::read`, no `RwLock::write` allowed in `notify`.
- **Handler iteration is straight-line code.** Walk a `Vec<Arc<dyn Fn>>`, call each, return. No conditional checks beyond the iteration itself.
- **Every public API has a criterion benchmark.** No exceptions.

### Optimization rules

- **`#[inline]` on small hot-path functions.** Let the compiler decide on `inline(always)` based on measurement.
- **`Arc<str>` over `String` for handler IDs and names** (refcount bump on clone, no alloc).
- **`SmallVec<[Handler; 4]>` IF expected handler count is small.** Decision deferred until benchmarks show it matters.
- **Avoid `format!` in hot paths.** Use static strings or const formatters where possible.
- **Profile before optimizing.** No "this should be faster" without `perf` / `flamegraph` evidence.

### Required benchmarks

The benchmark suite must cover:

1. Single handler, single thread (the baseline)
2. 1, 4, 16, 64 handlers, single thread (scaling with handler count)
3. 1 handler, 1, 4, 16 threads contended (scaling with contention)
4. Mixed scenarios (4 handlers, 8 threads contended)
5. Async path overhead (vs sync, same operation)
6. Register/unregister latency
7. Memory footprint at 0, 100, 10K handlers

---

## Lock-free discipline

This is a lock-free crate. Locks on the read path are a design failure.

### Allowed

- `ArcSwap<Vec<Handler>>` for atomic snapshot reads
- Atomic counters (`AtomicU64::fetch_add` with `Ordering::Relaxed` where ordering doesn't matter)
- Standard library `Arc` (refcount is atomic)
- Atomic pointers / atomic state machines

### NOT allowed on the notify hot path

- `Mutex` (any kind)
- `RwLock` (any kind)
- `parking_lot` mutexes (still locks)
- Channel sends (this is what we're an alternative to)

### Acceptable on the register/unregister slow path

- Brief locks for atomic state changes are fine because these paths are rare
- Clone-then-swap pattern is the standard for `ArcSwap`-based registries

---

## Concurrency discipline

- **All public types are `Send + Sync`** unless explicitly documented otherwise.
- **Multiple threads can fire `notify()` simultaneously** without coordination.
- **Handler closures must be `Send + Sync + 'static`** (enforced by trait bound).
- **No reader-side serialization** under any handler count.
- **Writer-side latency may degrade gracefully** under high register/unregister contention. This is rare in practice.

---

## REPS compliance (non-negotiable)

`src/lib.rs` MUST contain:

```rust
#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(unused_must_use)]
#![deny(unused_results)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::todo)]
#![deny(clippy::unimplemented)]
#![deny(clippy::print_stdout)]
#![deny(clippy::print_stderr)]
#![deny(clippy::dbg_macro)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![deny(clippy::missing_safety_doc)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
```

Already in place at scaffold time. Must NOT be relaxed.

---

## API design discipline

### Sync API is the default

```rust
use registry_io::SyncRegistry;

let registry = SyncRegistry::<MyEvent>::new();
let handler_id = registry.register(|event| {
    // handler runs sync, on the calling thread
});

registry.notify(&event);  // dispatches inline, sub-50ns
registry.unregister(handler_id);
```

Sync is the workhorse. Async is opt-in via feature flag.

### Async API requires feature flag

```rust
#[cfg(feature = "async")]
use registry_io::AsyncRegistry;

let registry = AsyncRegistry::<MyEvent>::new();
let handler_id = registry.register(|event| async move {
    // async handler
});

registry.notify(&event).await;  // awaits all handlers concurrently
```

Async path is feature-gated. Sync users pay zero cost.

### Handler signatures

Sync handlers: `Fn(&E) + Send + Sync + 'static`

Async handlers: `Fn(&E) -> impl Future<Output = ()> + Send + 'static`

Use `&E` not `E` to avoid forcing handlers to consume the event (events are read-only by default).

### Handler IDs

`HandlerId` is opaque - users don't construct or inspect them. Returned by `register`, used for `unregister`.

Internal representation: `u64` (atomic counter incremented per registration). Cheap to copy, easy to compare.

### Panic policy

If a handler panics:
- The panic is caught via `catch_unwind`
- Other handlers still fire
- The panic is logged (via `tracing` if the feature is enabled, otherwise to stderr)
- `notify()` itself does NOT propagate the panic

This trades correctness (panic propagation) for resilience (one bad handler doesn't break siblings). Document this trade-off clearly.

---

## Versioning discipline

Pre-1.0:

- `0.1.x` - scaffold (current)
- `0.2.x` - sync registry foundation (functional API, no async)
- `0.5.x` - async support added
- `0.7.x` - panic isolation, priority ordering, edge cases
- `0.9.x` - hardening, fuzz, benchmark verification, RC
- `1.0.0` - stable

Each release tagged. Each release has `docs/release-notes/v<X.Y.Z>.md`.

---

## Documentation discipline

Every public item MUST have:

- A one-line summary
- A longer description if behavior is non-obvious
- A `# Examples` section with runnable code
- A `# Performance` note for hot-path methods (e.g., "Sub-50ns for sync notify")
- A `# Panics` section if the function can panic (we generally don't)
- A `# Safety` section for any `unsafe` function

---

## Testing discipline

The registry MUST have:

- Unit tests for register, unregister, notify, get
- Concurrency tests (multiple threads firing notify concurrently)
- Property tests (using `proptest`) for invariants:
  - Register N handlers, unregister M, expect N-M after
  - Notify with N handlers, expect N invocations
  - Handler IDs are unique
- Fuzz target for handler closures (weird capture types, panics)
- Memory leak test (`dhat` or manual `Arc` strong count verification)
- Stress test (10K registrations, 10K unregistrations, validate state)

---

## Dependencies

Approved dependencies for 1.0:

- `arc-swap = "1.7"` - lock-free `ArcSwap<Vec<...>>` for handler list

Optional dependencies (feature-gated):

- `futures-core = "0.3"` - async support (feature: `async`)

Approved dev-dependencies:

- `criterion = "0.5"` - benchmarks
- `tokio = "1"` - async test runtime
- `proptest` (will be added when property tests land)

**New dependencies require:**
- Strong justification (why can't we implement this in-house?)
- License compatibility (Apache-2.0 / MIT / compatible)
- MSRV check (must support Rust 1.75)
- `cargo audit` clean

---

## Out of scope (always)

- **Distributed delivery.** This is a local primitive. Use NATS, Redis, or other systems for cross-process.
- **Persistent event log.** Events are fired-and-forgotten. Use an event store if you need durability.
- **Backpressure.** Handlers must be fast. If your handler is slow, async-spawn the work yourself.
- **Schema evolution.** Events are typed Rust values. Schema management is the caller's concern.
- **Subscriber discovery.** Subscribers register explicitly. No service mesh, no broker.

---

## When you must break a directive

If a directive in this file genuinely needs an exception:

1. STOP. Don't break it silently.
2. Document why in the PR description.
3. Get explicit maintainer approval.
4. Add a `// REGISTRY-IO-EXCEPTION:` comment at the violation point with the rationale.
5. Update this file or `.dev/PROMPT.md` if the exception reveals a flaw in the directive.

---

<sub>registry-io directives - Copyright (c) 2026 James Gober. Apache-2.0 OR MIT.</sub>
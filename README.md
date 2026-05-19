<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br>
    <b>registry-io</b>
    <br>
    <sub>
        <sup>EVENT REGISTRY FOR RUST</sup>
    </sub>
</h1>

<p align="center">
    <a href="https://crates.io/crates/registry-io"><img src="https://img.shields.io/crates/v/registry-io.svg" alt="Crates.io"></a>
    <a href="https://crates.io/crates/registry-io"><img alt="downloads" src="https://img.shields.io/crates/d/registry-io.svg?color=%230099ff"></a>
    <a href="https://docs.rs/registry-io"><img src="https://docs.rs/registry-io/badge.svg" alt="Documentation"></a>
    <a href="https://github.com/jamesgober/registry-io/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/jamesgober/registry-io/actions/workflows/ci.yml/badge.svg"></a>
    <a href="https://github.com/rust-lang/rfcs/blob/master/text/2495-min-rust-version.md" title="MSRV"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.85%2B-blue"></a>
</p>

<p align="center">
    <b>High-Performance Event and Callback Registry Primitive</b>
    <br><br>
    <i>Sync-first with optional async. Lock-free reads, zero-allocation hot path, sub-50ns notify target.</i>
</p>

<br>

<p>
    <strong>registry-io</strong> is a high-performance event and callback registry primitive for Rust. It provides a focused alternative to channel-based notification when several components need to react to the same in-process event with the lowest possible dispatch overhead. The hot path is <strong>lock-free</strong>, <strong>allocation-free</strong>, and <strong>panic-isolating</strong>.
</p>

<p>
    Unlike pub/sub brokers and distributed messaging systems, <strong>registry-io</strong> stays focused on a single problem: fast in-process notification. A sync-first design means handlers run inline on the producer's thread with minimal coordination overhead. Optional async support is reserved for a future release; sync users pay zero cost for features they don't use.
</p>

<p>
    With its lock-free architecture, <strong>registry-io</strong> uses <a href="https://docs.rs/arc-swap"><code>ArcSwap</code></a> snapshots for reader-side access and atomic clone-then-swap for writer-side updates. Many threads can fire notifications simultaneously without blocking each other, while handler registration and unregistration happen on a separate slow path that does not interfere with the hot read path.
</p>

---


## Highlights

- **Cross-platform** — Linux, macOS, Windows.
- **Lock-free reads** via `ArcSwap` snapshots. Many threads can `notify`
  concurrently with no coordination.
- **Zero allocation** on the no-panic sync notify path.
- **Panic isolation** — a panicking handler does not stop siblings nor
  propagate to the caller. Optional `on_panic` callback for observability.
  Works for both sync handlers and async futures.
- **Priority ordering** — `register_with_priority(i32, ...)`. Higher fires
  first; ties broken in registration order.
- **`SyncRegistry<E>`** — generic over the event type. Handlers receive `&E`.
- **`AsyncRegistry<E>`** *(feature: `async`)* — same lock-free storage,
  futures-returning handlers, concurrent or sequential dispatch.
- **RAII guards** — `register_guard` returns a `HandlerGuard` /
  `AsyncHandlerGuard` that unregisters on drop.
- **`Send + Sync`** — share registries freely across threads.


---


## Status

**Stable. Production-ready.** `registry-io 1.0.0` is the first
stable release. The public API is **frozen** per the contract in
[`docs/STABILITY-1.0.md`](docs/STABILITY-1.0.md); breaking changes
require a major-version bump. Performance, panic-isolation, and
zero-allocation guarantees are measured and locked in.

Headline numbers (full table in
[`docs/PERFORMANCE.md`](docs/PERFORMANCE.md)):

- Sync `notify`, 1 handler, 1 thread — **10.1 ns**
- Sync `notify`, 4 handlers, 16 threads contended — **24.7 ns**
- Async `notify` concurrent, 1 handler — **177 ns**
- Async `notify_sequential`, 1 handler — **53 ns**
- Sync `notify` hot path heap allocations — **0** (verified by `dhat`)

Guarantees:

- **Zero `unsafe` in the public API.**
- **Lock-free reads** via `ArcSwap` snapshots.
- **Zero allocation** on the sync notify no-panic path.
- **Panic isolation** — one panicking handler does not stop siblings
  or propagate to the caller.
- **Priority-ordered dispatch** with stable ties.
- **RAII unregistration** via `HandlerGuard` / `AsyncHandlerGuard`.
- **`Send + Sync`** on every public type.
- **Cross-platform** — Linux, macOS, Windows on stable + MSRV
  1.85.0.

---

## When to use it

Use `registry-io` when you have:

- Multiple components that need **notification** when something happens (`config`
  `reload`, `file change`, `transaction commit`, `metric event`, etc.).
- Fast, **in-process handlers** measured in microseconds or less.
- A need to **register** and **unregister** handlers dynamically.
- **Performance-critical paths** where **channel allocation** would dominate.

**Do not** use `registry-io` when you have:

- **Cross-process** or **cross-network** delivery needs — use NATS, Redis pub/sub,
  or similar message brokers.
- **Heavy handler** workloads requiring backpressure — use
  `tokio::sync::broadcast` or channels.
- **Event sourcing** or **durability** requirements — use a real event log.

---

&nbsp;

## Quick start

```toml
[dependencies]
registry-io = "1.0"
```

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use registry_io::SyncRegistry;

let registry: SyncRegistry<u32> = SyncRegistry::new();
let total = Arc::new(AtomicU32::new(0));

let sink = Arc::clone(&total);
let id = registry.register(move |value| {
    sink.fetch_add(*value, Ordering::Relaxed);
});

registry.notify(&5);
registry.notify(&7);
assert_eq!(total.load(Ordering::Relaxed), 12);

assert!(registry.unregister(id));
```

### Priority ordering

```rust
use std::sync::{Arc, Mutex};
use registry_io::SyncRegistry;

let registry: SyncRegistry<()> = SyncRegistry::new();
let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));

let o = Arc::clone(&order);
let _ = registry.register_with_priority(100, move |_| o.lock().unwrap().push("audit"));
let o = Arc::clone(&order);
let _ = registry.register(move |_| o.lock().unwrap().push("business"));
let o = Arc::clone(&order);
let _ = registry.register_with_priority(-50, move |_| o.lock().unwrap().push("cleanup"));

registry.notify(&());
assert_eq!(order.lock().unwrap().as_slice(), &["audit", "business", "cleanup"]);
```

### RAII guards

```rust
use std::sync::Arc;
use registry_io::SyncRegistry;

let registry = Arc::new(SyncRegistry::<u32>::new());
{
    let _guard = registry.register_guard(|n| println!("scoped: {n}"));
    registry.notify(&1);
} // guard drops here -> handler is unregistered
assert!(registry.is_empty());
```

### Panic isolation

```rust
use registry_io::SyncRegistry;

let registry: SyncRegistry<()> = SyncRegistry::new();
registry.on_panic(|info| {
    eprintln!(
        "handler {} panicked: {}",
        info.handler_id(),
        info.message().unwrap_or("<opaque>")
    );
});

let _ = registry.register(|_| panic!("oops"));
let _ = registry.register(|_| println!("still ran"));
registry.notify(&()); // returns cleanly; both effects observed
```

### Async handlers *(feature: `async`)*

```toml
[dependencies]
registry-io = { version = "1.0", features = ["async"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

```rust,no_run
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use registry_io::r#async::AsyncRegistry;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let registry: AsyncRegistry<u32> = AsyncRegistry::new();
    let total = Arc::new(AtomicU32::new(0));

    for _ in 0..4 {
        let sink = Arc::clone(&total);
        let _ = registry.register(move |value| {
            let sink = Arc::clone(&sink);
            let v = *value;
            async move {
                tokio::task::yield_now().await;
                sink.fetch_add(v, Ordering::Relaxed);
            }
        });
    }

    // Concurrent dispatch — all 4 futures run in parallel.
    registry.notify(&10).await;
    assert_eq!(total.load(Ordering::Relaxed), 40);

    // Sequential dispatch — awaits each handler in priority order.
    registry.notify_sequential(&10).await;
    assert_eq!(total.load(Ordering::Relaxed), 80);
}
```

Same lock-free read path as `SyncRegistry`. Panics in handler futures are
caught via an internal `CatchUnwind` adapter and surfaced through
`on_panic`, just like sync handlers.

See [`examples/`](examples/) for runnable programs and [`docs/API.md`](docs/API.md)
for the full reference.

---

## Design philosophy

- **Sync-first.** The fast path is synchronous, runs on the calling thread,
  allocates nothing, and dispatches in nanoseconds.
- **Lock-free reads.** Multiple threads can call `notify()` concurrently
  without contention.
- **Zero allocation on the hot path.** Notify walks the handler list and
  dispatches without any heap allocation in the no-panic case.
- **Focused scope.** This is a local, in-process notification primitive.
  Not a message bus, not a distributed event system, not a pub/sub broker.

---

## Documentation

- [`docs/API.md`](docs/API.md) — full API reference with examples per item.
- [`docs/PATTERNS.md`](docs/PATTERNS.md) — four canonical integration
  patterns (hot reload, audit fan-out, metric events, transaction
  hooks), each with a runnable example.
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — internal structure
  walk-through, hot/slow path code excerpts, design decisions.
- [`docs/PERFORMANCE.md`](docs/PERFORMANCE.md) — cost model, benchmarks, and
  concurrency characteristics.
- [`docs/SECURITY.md`](docs/SECURITY.md) — threat model, panic isolation
  detail, fuzz methodology.
- [`docs/STABILITY-1.0.md`](docs/STABILITY-1.0.md) — the v1.0
  stability contract (what's frozen, what can change).
- [`docs/PLATFORM-NOTES.md`](docs/PLATFORM-NOTES.md) — per-platform
  behavior nuances (Linux/macOS/Windows).
- [`.dev/ROADMAP.md`](.dev/ROADMAP.md) — milestone plan to 1.0.
- [`CHANGELOG.md`](CHANGELOG.md) — release history.

---

## Standards

- **REPS** (Rust Efficiency & Performance Standards) governs every decision.
  See [`REPS.md`](REPS.md).
- **MSRV:** Rust 1.85.
- **Edition:** 2024.
- **Cross-platform:** Linux, macOS, Windows.

---

## License

Dual-licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.


<!-- COPYRIGHT
############################################# -->
<div align="center">
  <h2></h2>
  <sup>COPYRIGHT <small>&copy;</small> 2026 <strong>JAMES GOBER.</strong></sup>
</div>
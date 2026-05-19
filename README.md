<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br>
    <b>registry-io</b>
    <br>
    <sub>
        <sup>HIGH-PERFORMANCE EVENT REGISTRY PRIMITIVE</sup>
    </sub>
</h1>

<p align="center">
    <a href="https://crates.io/crates/registry-io"><img src="https://img.shields.io/crates/v/registry-io.svg" alt="Crates.io"></a>
    <a href="https://crates.io/crates/registry-io"><img alt="downloads" src="https://img.shields.io/crates/d/=%230099ff"></a>
    <a href="https://docs.rs/registry-io"><img src="https://docs.rs/registry-io/badge.svg" alt="Documentation"></a>
    <a href="https://github.com/jamesgober/registry-io/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/jamesgober/registry-io/actions/workflows/ci.yml/badge.svg"></a>
    <a href="#license"><img src="https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue.svg" alt="License"></a>
</p>

<p align="center">
    <b>High-Performance Event Registry for Rust</b>
    <br>
    <i>Sync-first with optional async. Lock-free reads, zero-allocation hot path, sub-50ns notify target.</i>
</p>

<br>

<p>
    <strong>registry-io</strong> is a high-performance event and callback registry primitive for Rust. It provides a focused alternative to channel-based notification when you need multiple handlers responding to the same event with minimal dispatch overhead. Built from the ground up with a lock-free read path and zero-allocation discipline on the hot path, it targets sub-50ns notify latency for synchronous handlers.
</p>

<p>
    Unlike pub/sub brokers and distributed messaging systems, <strong>registry-io</strong> stays focused on a single problem: fast in-process notification. A sync-first design means handlers run inline on the producer's thread with minimal coordination overhead. Optional async support is available via feature flag for handlers that genuinely need to await I/O, but the sync path remains the fast path and pays zero cost for users who don't need async.
</p>

<p>
    With its lock-free architecture, <strong>registry-io</strong> uses <code>ArcSwap</code> snapshots for reader-side access and atomic clone-then-swap for writer-side updates. This means many threads can fire notifications simultaneously without blocking each other, while handler registration and unregistration happen on a separate slow path that does not interfere with the hot read path.
</p>

---

## Status

**Active development.** Scaffolded and on the path to 1.0. See [.dev/ROADMAP.md](.dev/ROADMAP.md) for milestone tracking.

The public API is not yet stable. Pin specific versions; expect changes pre-1.0.

---

## What it does

High-performance event/callback registry for Rust. Sync-first with optional async. Lock-free reads, zero-allocation hot path, sub-50ns notify target. Designed as the foundation primitive for portfolio crates needing fast in-process notification.

---

## Design philosophy

- **Sync-first.** The fast path is synchronous, runs on the calling thread, allocates nothing, and dispatches in nanoseconds.
- **Async-capable.** Async handlers are available via opt-in feature flag for handlers that need to await I/O.
- **Lock-free reads.** Multiple threads can call `notify()` concurrently without contention.
- **Zero allocation on hot path.** Notify walks the handler list and dispatches without any heap allocation.
- **Focused scope.** This is a local, in-process notification primitive. NOT a message bus, NOT a distributed event system, NOT a pub/sub broker.

---

## When to use it

Use `registry-io` when you have:

- Multiple components that need notification when something happens (file change, config update, transaction commit, metric event)
- Fast, in-process handlers (microseconds or less)
- Same-thread or cross-thread but not cross-process delivery
- A need to register and unregister handlers dynamically
- Performance-critical paths where channel allocation would dominate

**Do NOT use `registry-io` when you have:**

- Cross-process or cross-network delivery needs (use NATS, Redis pub/sub, or similar)
- Heavy handler workloads requiring backpressure (use `tokio::sync::broadcast` or channels)
- Event sourcing or durability requirements (use a real event log)

---

## Quick start

```toml
[dependencies]
registry-io = "0.1"
```

```rust
// Examples land as the public API stabilizes.
// See `examples/` and the rustdoc once 0.2 ships.
```

---

## Standards

- **REPS** (Rust Efficiency & Performance Standards) governs every decision. See [REPS.md](REPS.md).
- **MSRV:** Rust 1.75.
- **Edition:** 2024.
- **Cross-platform:** Linux, macOS, Windows.
- **No-std:** opt-out via `default-features = false` (where applicable).

---

## License

Dual-licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

---

<sub>Copyright &copy; 2026 <strong>James Gober</strong>. All rights reserved.</sub>
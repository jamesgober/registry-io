# registry-io - Project Prompt

> Context document for AI editor sessions working on `registry-io`.
> Read this BEFORE writing any code on this crate.

---

## Read order (mandatory)

1. `REPS.md` at repo root - Rust Efficiency & Performance Standards. **SUPREME AUTHORITY.**
2. `_strategy/UNIVERSAL_PROMPT.md` - portfolio-wide engineering directives.
3. `.dev/DIRECTIVES.md` - this project's specific directives.
4. This file - project context.
5. `.dev/ROADMAP.md` - current phase, milestone targets, exit criteria.

REPS is mandatory and overrides anything else in this repository.

---

## What this crate is

`registry-io` is a **high-performance event and callback registry primitive** for Rust. It provides a focused alternative to channel-based notification when multiple components need notification with minimal dispatch overhead.

It is the **foundation primitive for in-process notification across the portfolio**. Several upcoming crates and current ones depend on it:

- `config-lib` - hot reload change notifications
- `fsys` - journal observer hooks, write-commit notifications
- `audit-trail` - sink notification fan-out
- `metrics-lib` - exporter notification
- `DISTRO` - transaction state change hooks (commit, abort, recovery)
- `hive-server` - connection lifecycle events
- `bouncer-io` - rate limit threshold notifications

This crate is **priority** - blocking work on these downstream crates.

## Why it exists

Most Rust projects re-implement notification primitives:

- Some use `mpsc::channel` (allocation per event, cross-thread, queued)
- Some use `Vec<Box<dyn Fn>>` behind a `Mutex` (lock contention on every notify)
- Some use `tokio::sync::broadcast` (heavyweight, async-only, bounded)
- Some use `tracing` subscribers (purpose-built, not general)

None of these are right for "I want N handlers to fire when X happens, fast, in-process, possibly across threads." That's what `registry-io` fills.

## Status

**Version:** `0.1.0` - scaffolded, no implementation yet.
**Target:** `1.0.0` stable in **3-4 focused weeks** of work.

## Skill areas

Working on this crate requires comfort with:

- **Lock-free data structures** - `ArcSwap`, atomic operations, memory ordering
- **Closure trait objects** - `Arc<dyn Fn>`, `Send + Sync + 'static` bounds
- **Async trait objects** - pinned futures, `BoxFuture`, async closures
- **Benchmarking** - `criterion`, statistical rigor, contention scenarios
- **Cross-platform threading** - same behavior on Linux/macOS/Windows
- **API design** - sync vs async vs hybrid surface design
- **Generic type parameters** - event type `E`, handler return types, lifetime bounds

## Scope (1.0)

### In scope for 1.0

- **`SyncRegistry<E>`** - lock-free, zero-allocation notify on hot path
- **`AsyncRegistry<E>`** - feature-gated, for async handler support
- **Handler IDs** - register returns ID, used for unregister
- **Handler guards** - RAII pattern for automatic unregistration
- **Panic isolation** - one handler panicking doesn't break siblings
- **Priority ordering** - optional priority value at register time
- **Comprehensive benchmarks** - verified sub-50ns sync notify, 1-64 thread contention
- **Fuzz testing** - handler closures, weird types
- **Cross-platform** - Linux, macOS, Windows, all green
- **Full REPS compliance** - all lints, all tests, all docs
- **Integration examples** - show config-lib hot reload replacement

### Out of scope (deferred to 1.1+)

- **`HybridRegistry<E>`** - mixing sync + async handlers (compose if needed)
- **Conditional dispatch** - per-handler filters/predicates (wrap externally)
- **Built-in metrics on registry** - users add their own observability
- **Weak references** - auto-cleanup of dropped subscribers (memory model gets complex)
- **`no_std` support** - currently uses `std::sync` primitives
- **Cross-process delivery** - separate concern, separate crate if needed
- **Distributed registries** - explicitly out of scope per design decision

## Performance targets (verified by benchmark before claiming)

| Operation | Target |
|-----------|--------|
| Sync notify, 1 handler, 1 thread | <20ns |
| Sync notify, 4 handlers, 1 thread | <50ns |
| Sync notify, 1 handler, 16 threads contended | <50ns |
| Sync register (rare path) | <1us |
| Sync unregister (rare path) | <1us |
| Async notify, 1 handler | <500ns (boxed future overhead) |
| Memory: empty registry | <128 bytes |
| Memory: 100 handlers | <16 KiB |

## Architectural constraints

### MUST

- Lock-free read path (no `Mutex`, no `RwLock` on the notify path)
- Zero allocation on `notify()` for the sync case
- Generic over event type `E` with minimal trait bounds (`Send + Sync + 'static` for cross-thread)
- Handlers stored as `Arc<dyn Fn(&E) + Send + Sync + 'static>` (boxed trait object)
- Compatible with stable Rust 1.75+

### MUST NOT

- Pull in `tokio` or any async runtime as a hard dependency (only feature-gated `futures-core`)
- Use `unsafe` code in the public API
- Allocate on the notify hot path
- Force users to box their event types
- Mix the sync and async APIs in a confusing way

## How to develop on this crate

1. Read this document, REPS, DIRECTIVES, ROADMAP.
2. Check current phase in `.dev/ROADMAP.md`.
3. Pick the next unchecked task.
4. Implement with REPS discipline:
   - No `unwrap`, no `expect`, no `todo!`, no `unimplemented!`
   - No `print_stdout`, no `print_stderr`, no `dbg!`
   - Every new public item: rustdoc + at least one example
   - Every new error path: test
   - Every hot path change: benchmark
5. Update `CHANGELOG.md` under `[Unreleased]` in the same commit.
6. Run the full CI gate locally before pushing.
7. Mark the task done in `.dev/ROADMAP.md` in the same commit.
8. Push.

When a phase is complete, follow `_strategy/RELEASE_WORKFLOW.md` for the release sequence.

## Reference patterns

When designing the API, look at:

- `tracing-subscriber` - subscriber composition
- `tower::Service` and `tower::Layer` - middleware patterns
- `slotmap` / `generational-arena` - handler ID design
- `arc-swap` documentation - the lock-free read pattern
- `crossbeam` - lock-free primitives for ideas

When deciding handler signatures, look at:

- Standard observer pattern in OOP languages
- `tokio::sync::broadcast` for "one-to-many" semantics

## When in doubt

- Read REPS first.
- Check the roadmap for the current phase's exit criteria.
- If a feature isn't in the roadmap, propose it (update roadmap first) before implementing.
- If performance is contested, write a benchmark.
- If correctness is contested, write a test.
- If API design is contested, look at how `tower` or `tracing` solved similar problems.

---

<sub>registry-io - Copyright (c) 2026 James Gober. Apache-2.0 OR MIT.</sub>
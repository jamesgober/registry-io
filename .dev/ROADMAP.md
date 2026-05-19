# registry-io - Production Roadmap to 1.0

> The engineering contract that takes `registry-io` from `0.1.0` scaffold to `1.0.0` stable.
>
> Reads: `REPS.md` (supreme authority), `_strategy/UNIVERSAL_PROMPT.md` (peak performance + max efficiency + max concurrency + nuclear-proof security + cross-platform), `.dev/DIRECTIVES.md`, `.dev/PROMPT.md`.
>
> Target ship date: **3-4 focused weeks**.
> Status: Phase 0.1.0 complete (scaffold). Phase 0.2.0 next.

---

## The 1.0 contract

When `registry-io 1.0.0` ships, it commits to:

### Functional contract

- `SyncRegistry<E>` - the workhorse, sync-first, lock-free reads
- `AsyncRegistry<E>` - feature-gated, for async handler support
- Handler IDs returned from `register`, used for `unregister`
- Handler guards (RAII pattern for automatic unregistration)
- Panic isolation (one handler panicking does NOT break siblings)
- Priority ordering (optional priority value at register time)
- Cross-platform identical behavior (Linux, macOS, Windows)

### Performance contract (every number verified by committed benchmark)

| Operation | Target | Measurement |
|-----------|--------|-------------|
| Sync notify, 1 handler, 1 thread | <20ns | criterion, tight loop |
| Sync notify, 4 handlers, 1 thread | <50ns | criterion |
| Sync notify, 16 handlers, 1 thread | <200ns | criterion |
| Sync notify, 1 handler, 16 threads contended | <50ns | criterion parametric |
| Sync register (rare path) | <1us | criterion |
| Sync unregister (rare path) | <1us | criterion |
| Async notify, 1 handler | <500ns | criterion async |
| Memory: empty registry | <128 bytes | dhat or sizeof analysis |
| Memory: 100 registered handlers | <16 KiB | dhat |
| Zero allocations on sync `notify()` | verified | dhat |

If any number is not verified by a committed benchmark, the version that claims it does NOT ship.

### Stability contract

- Public API frozen for the lifetime of v1.x
- `#[non_exhaustive]` on enums that may grow
- MSRV 1.75 held for v1.x
- Edition 2024
- Apache-2.0 OR MIT dual licensed
- Yank policy: critical correctness bugs only

### Security contract (nuclear-proof requirement)

- Zero unsafe code in public API
- Fuzz testing of handler closures (panics, weird types)
- `cargo audit` clean
- `cargo deny check` clean
- Panic in handler is caught and isolated

### Quality contract

- Full REPS lint discipline
- `cargo fmt --all -- --check` clean
- `cargo clippy --all-targets --all-features -- -D warnings` clean
- `cargo test --all-features` passing on Linux, macOS, Windows on stable + MSRV
- `cargo doc --no-deps --all-features` produces zero warnings with `RUSTDOCFLAGS="-D warnings"`
- Every public item: rustdoc + at least one runnable example

---

## Phase 0.1.0 - Scaffold (complete)

- [x] Repository created on GitHub
- [x] Repo description and topics set
- [x] Cargo.toml with proper metadata
- [x] REPS.md canonical at repo root
- [x] LICENSE-APACHE + LICENSE-MIT (dual licensing)
- [x] .gitignore, .editorconfig, rustfmt.toml, clippy.toml
- [x] README.md with badges and design philosophy
- [x] CHANGELOG.md (Keep a Changelog format)
- [x] src/lib.rs with REPS-disciplined lint configuration
- [x] tests/smoke.rs
- [x] benches/registry_bench.rs (placeholder)
- [x] .dev/PROMPT.md - project context
- [x] .dev/DIRECTIVES.md - project-specific directives
- [x] This roadmap
- [x] CI workflow (Linux/macOS/Windows on stable + MSRV)

---

## Phase 0.2.0 - SyncRegistry Foundation  *(rolled into v0.4.0)*

**Goal:** Implement the core `SyncRegistry<E>` with lock-free reads. Just the basics, no priority, no async.

**Effort:** 3-4 days.

### Tasks

- [x] **Design the `SyncRegistry<E>` type:**
  - Internal storage: `ArcSwap<Vec<HandlerEntry<E>>>`
  - `HandlerEntry { id: HandlerId, priority: i32, handler: Arc<dyn Fn(&E) + Send + Sync + 'static> }`
  - `HandlerId` as `u64` atomic counter
- [x] **Implement `SyncRegistry::new()`** - empty registry
- [x] **Implement `SyncRegistry::register<F>(handler: F) -> HandlerId`**
  - Slow path: load current Vec, clone, push new entry, atomic swap
  - Returns unique HandlerId
- [x] **Implement `SyncRegistry::unregister(id: HandlerId) -> bool`**
  - Slow path: load, clone, retain by id, atomic swap
  - Returns true if handler was found and removed
- [x] **Implement `SyncRegistry::notify(&self, event: &E)`**
  - Hot path: load ArcSwap guard (no lock), iterate, call each handler
  - Zero allocation on this path
  - `#[inline]` annotation
- [x] **Implement `SyncRegistry::handler_count() -> usize`** - returns current count
- [x] **Implement `SyncRegistry::clear()`** - removes all handlers
- [x] **Add `Default` impl for `SyncRegistry<E>`**
- [x] **Add `Debug` impl** (avoids displaying closures)
- [x] **Unit tests:**
  - [x] register, unregister, notify happy path
  - [x] notify with 0 handlers (no-op)
  - [x] notify with N handlers (all fire)
  - [x] register/unregister return correct IDs
  - [x] unregister returns false for unknown ID
  - [x] clear removes all
  - [x] thread-safety: 8 threads firing notify concurrently
- [x] **Smoke test passing**
- [x] **README updated with first example**
- [x] **CHANGELOG updated under [Unreleased]**
- [x] **`.dev/release/v0.4.0.md` written** *(consolidated milestone release notes)*

### Exit criteria

- [x] `SyncRegistry` is functional and tested
- [x] No REPS lint violations
- [x] Zero `unsafe` code
- [x] README has working code example
- [ ] All CI checks green on all three platforms *(verified locally; CI cross-platform run pending)*

---

## Phase 0.3.0 - Handler guards + builder  *(rolled into v0.4.0)*

**Goal:** RAII handler unregistration, ergonomic builder API.

**Effort:** 2 days.

### Tasks

- [x] **`HandlerGuard<E>`** - RAII type that unregisters on Drop
  - Returned by `SyncRegistry::register_guard()`
  - Drop impl calls unregister
  - Allows manual `forget()` for static handlers
- [x] **`SyncRegistry::with_capacity(n)`** *(`builder()` deferred; direct constructor proved sufficient)*
- [x] Tests for guard drop behavior
- [x] Documentation updates

### Exit criteria

- [x] Handler guards work correctly across drop scopes
- [x] Builder-style API is documented with examples

---

## Phase 0.4.0 - Priority ordering + panic isolation  *(shipped in v0.4.0)*

**Goal:** Optional priority value at register time, plus catch_unwind around each handler.

**Effort:** 2-3 days.

### Tasks

- [x] **`SyncRegistry::register_with_priority<F>(priority: i32, handler: F)`**
  - Higher priority = called first
  - Same priority = registration order
  - Default priority = 0
- [x] **Stable insertion by priority** via `Vec::partition_point` (cheaper than full re-sort)
- [x] **`catch_unwind` around each handler invocation** in notify
- [x] **`SyncRegistry::on_panic(callback)`** - optional panic callback (silent absorption by default)
- [x] **`SyncRegistry::clear_panic_callback()`**
- [x] Tests for priority ordering
- [x] Tests for panic isolation (handler #2 panics, #1 and #3 still fire)
- [x] Tests for on_panic callback (including callback-panics-too)

### Exit criteria

- [x] Priority ordering verified
- [x] Panic in handler is contained, siblings still fire
- [x] No memory leak from panics (payload dropped after callback returns)

---

## Phase 0.5.0 - AsyncRegistry (feature-gated)  *(shipped in v0.5.0)*

**Goal:** Async handler support via `async` feature flag.

**Effort:** 4-5 days.

### Tasks

- [x] **Feature flag `async`** in Cargo.toml (`async = ["std"]`)
- [x] **`AsyncRegistry<E>`** type:
  - Same internal storage pattern as SyncRegistry (`ArcSwap<Vec<AsyncHandlerEntry<E>>>`)
  - Handler type: `Arc<dyn Fn(&E) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> + Send + Sync + 'static>`
- [x] **`AsyncRegistry::register<F, Fut>(handler: F) -> HandlerId`**
  - Where F: Fn(&E) -> Fut + Send + Sync + 'static
  - Fut: Future<Output = ()> + Send + 'static
- [x] **`AsyncRegistry::notify(&self, event: &E)` - concurrent dispatch**
  - Concurrent via crate-local `JoinAll` (in-tree, no `futures-util`)
- [x] **`AsyncRegistry::notify_sequential(&self, event: &E)`** - explicit sequential variant
- [x] **Panic isolation** in async handlers via crate-local `CatchUnwind` adapter
- [x] **Async benchmark** (`benches/async_notify.rs`) — concurrent and sequential at N=0,1,4,16
- [x] **Async examples** in examples/ (`async_basic`, `async_concurrent_vs_sequential`)
- [x] Tests using `tokio::test` (38 async integration tests + 4 future_ext unit tests)

### Exit criteria

- [x] `AsyncRegistry` functional
- [x] Feature flag works (sync still compiles without `async`; async still compiles without `sync`)
- [x] Async overhead documented in `docs/PERFORMANCE.md` (full numbers measured in 0.6.0)

---

## Phase 0.6.0 - Performance verification + tuning

**Goal:** Run full benchmark suite, hit Performance Contract numbers, tune as needed.

**Effort:** 1 week.

### Tasks

- [ ] **Write comprehensive benchmark suite:**
  - [ ] `benches/sync_notify.rs` - all sync notify scenarios
  - [ ] `benches/async_notify.rs` - async path measurements
  - [ ] `benches/register_unregister.rs` - slow path latencies
  - [ ] `benches/contention.rs` - 1-64 thread contention
  - [ ] `benches/memory.rs` - memory footprint at various sizes
- [ ] **Run on dev machine, commit baselines.json**
- [ ] **Compare against Performance Contract:**
  - If any target missed, profile and tune
  - Common tuning: `#[inline]` placement, `SmallVec` for small handler counts, eliminate `Arc::clone` on hot path
- [ ] **Allocation profile with `dhat`** - verify zero alloc on `notify()`
- [ ] **`docs/PERFORMANCE.md`** - methodology + results + tuning guide

### Exit criteria

- [ ] All Performance Contract targets met
- [ ] Benchmark baselines committed
- [ ] `docs/PERFORMANCE.md` documents results

---

## Phase 0.7.0 - Hardening + edge cases

**Goal:** Property tests, fuzz, weird edge cases.

**Effort:** 3-4 days.

### Tasks

- [ ] **Add `proptest` for invariants:**
  - Register N, unregister M, expect N-M remaining
  - Notify N handlers, expect N invocations exactly once
  - Handler IDs are unique even after many register/unregister cycles
- [ ] **Set up `cargo-fuzz` workspace** in `fuzz/`
- [ ] **Fuzz target: handler closures** - random closure shapes, panic patterns
- [ ] **Fuzz target: event types** - structs with various trait impls
- [ ] **Run fuzz for 1 CPU-hour minimum**
- [ ] **Fix any findings**
- [ ] **Memory leak test:**
  - Register/unregister 10K cycles
  - Verify `Arc` strong count returns to baseline
- [ ] **`docs/SECURITY.md`** - fuzz methodology and state

### Exit criteria

- [ ] proptest passing
- [ ] Fuzz clean for 1 CPU-hour
- [ ] No memory leaks across stress tests

---

## Phase 0.8.0 - Integration validation

**Goal:** Prove the crate by integrating it into a real portfolio crate.

**Effort:** 2-3 days.

### Tasks

- [ ] **Write integration example** in `examples/`:
  - Replace channel-based hot reload in a config-lib-style usage
  - Show the before/after performance and code complexity
- [ ] **Coordinate with config-lib** roadmap:
  - When config-lib hits its 0.9.6 hot-reload phase, use registry-io
  - Validate the API in real use
- [ ] **Document the integration patterns** in `docs/PATTERNS.md`:
  - Hot reload notification pattern
  - Audit logging fan-out pattern
  - Metric event pattern
  - Transaction state change pattern
- [ ] **API refinements** based on real-world feedback (last chance before 1.0 freeze)

### Exit criteria

- [ ] At least one portfolio crate uses registry-io successfully
- [ ] `docs/PATTERNS.md` has 3+ documented integration patterns
- [ ] Any API refinements absorbed before 0.9.0

---

## Phase 0.9.0 - Documentation + Release Candidate

**Goal:** Final documentation pass. Cut `1.0.0-rc.1`.

**Effort:** 3-4 days.

### Tasks

- [ ] **Write `docs/STABILITY-1.0.md`** - the 1.0 contract
- [ ] **Write `docs/ARCHITECTURE.md`** - internal structure, lock-free read pattern, ArcSwap usage
- [ ] **Verify `docs/PERFORMANCE.md`** (from 0.6.0)
- [ ] **Verify `docs/SECURITY.md`** (from 0.7.0)
- [ ] **Verify `docs/PATTERNS.md`** (from 0.8.0)
- [ ] **`docs/PLATFORM-NOTES.md`** if cross-platform behavior has any nuances
- [ ] **Audit every public item's rustdoc**
- [ ] **Write `docs/release-notes/v1.0.0.md`** per `_strategy/RELEASE_NOTES_TEMPLATE.md`
- [ ] **Cut `1.0.0-rc.1`** per `_strategy/RELEASE_WORKFLOW.md`
- [ ] **Soak period** - 1 week minimum
- [ ] **Iterate on rc.N if needed**

### Exit criteria

- [ ] All docs in place
- [ ] `1.0.0-rc.1` published to crates.io as pre-release
- [ ] 1 week soak with no critical issues

---

## Phase 1.0.0 - Stable release

**Goal:** Ship the foundation primitive.

### Pre-flight

- [ ] No critical issues from RC soak
- [ ] All CI checks green
- [ ] All Performance Contract targets met
- [ ] `cargo public-api diff` clean vs rc.1
- [ ] `cargo audit` clean
- [ ] `cargo deny check` clean

### Release sequence

- [ ] Bump Cargo.toml to `1.0.0`
- [ ] Move `[Unreleased]` CHANGELOG to `[1.0.0]`
- [ ] Finalize `docs/release-notes/v1.0.0.md`
- [ ] Commit: `Milestone Update v1.0.0`
- [ ] Push, verify CI green
- [ ] Tag `v1.0.0`, push tag
- [ ] GitHub release (NOT pre-release)
- [ ] `cargo publish --dry-run` then `cargo publish`
- [ ] Verify crates.io and docs.rs

### Exit criteria

- [ ] `registry-io 1.0.0` live on crates.io
- [ ] docs.rs builds clean
- [ ] At least one portfolio crate is consuming `registry-io = "1.0"`

---

## Post-1.0 backlog

- [ ] `HybridRegistry<E>` - mix sync + async handlers
- [ ] Conditional dispatch (per-handler filters/predicates)
- [ ] Weak references (auto-cleanup of dropped subscribers)
- [ ] Built-in metrics observability (cache hits, panic counts, etc.) via feature flag
- [ ] `no_std` support (separate code path with static handler arrays)
- [ ] Hierarchical registries (parent/child for event scoping)
- [ ] Tracing integration (auto-emit spans for handler dispatch)

Explicitly **OUT** of scope forever:

- Cross-process delivery (use NATS, Redis, message brokers)
- Distributed registries (consensus is a different problem)
- Persistent event log (use an event store)
- Schema management (events are typed Rust values)

---

## Quick reference

```
==============================================================
registry-io roadmap to 1.0
==============================================================
0.1.0  Scaffold                              DONE
0.2.0  SyncRegistry foundation               3-4 days
0.3.0  Handler guards + builder              2 days
0.4.0  Priority ordering + panic isolation   2-3 days
0.5.0  AsyncRegistry (feature-gated)         4-5 days
0.6.0  Performance verification + tuning     1 week
0.7.0  Hardening + edge cases (proptest+fuzz) 3-4 days
0.8.0  Integration validation                2-3 days
0.9.0  Docs + Release Candidate              3-4 days
1.0.0  Stable Release                        1 day
==============================================================
Total: ~3-4 focused weeks
==============================================================
```

---

## Roadmap discipline

- Every task has a checkbox - track completion explicitly
- Every phase has exit criteria - dont move on until current phase exits cleanly
- No skipping phases without explicit written justification
- No performance claim without committed benchmark
- No "production-grade" claim without REPS compliance
- CHANGELOG updated under [Unreleased] in every commit
- `Milestone Update vX.Y.Z` commit format for every phase release

---

<sub>registry-io roadmap - Copyright (c) 2026 James Gober. Apache-2.0 OR MIT.</sub>
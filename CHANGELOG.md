# Changelog

All notable changes to `registry-io` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Added

### Changed

### Fixed

### Security

---

## [0.8.0] - 2026-05-19

Integration-validation milestone. Four canonical integration patterns
are now documented end-to-end (hot reload, audit fan-out, metric event
collection, transaction state-change hooks), each backed by a runnable
example. Documentation accuracy was swept across every `.md` file
under the repository to bring stale version strings, copyright
footers, and dependency snippets in line with the current release.

### Added

- **`docs/PATTERNS.md`** — integration-pattern reference covering the
  four use cases the crate was built to serve. Each pattern names the
  problem, sketches the solution shape, contrasts with the obvious
  channel/mutex alternatives, and links to its runnable example.
  Closes with a "choosing between sync and async" decision matrix and
  a "mistakes to avoid" section.
- **`examples/pattern_hot_reload.rs`** — config-lib-style hot reload.
  A single `Config` value owns an `ArcSwap<Snapshot>` plus an
  `Arc<SyncRegistry<Snapshot>>`; subscribers re-derive local state
  from each new snapshot in ~10 ns per handler.
- **`examples/pattern_audit_fanout.rs`** — three different sinks
  (stdout, file-tee, critical-alert) attached via `register_guard`;
  drop-the-sink-to-deregister semantics make detach trivial; panic
  isolation between sinks comes for free.
- **`examples/pattern_metric_event.rs`** — one lock-free atomic
  aggregator + one batching exporter against a `SyncRegistry<MetricEvent>`.
  Drives 1 000 simulated requests through 3 event kinds.
- **`examples/pattern_transaction_hooks.rs`** — priority-ordered
  transaction hooks (WAL flush → cache invalidate → replication ship →
  metrics record) using `register_with_priority`.
- **Expanded `docs/API.md`** — the previously-condensed async block
  ("`unregister / clear / contains / handler_count / is_empty`",
  "`on_panic / clear_panic_callback`") is now seven individual
  sections, each with its own description, signature, parameters,
  return value, and at least one runnable example. Matches the
  per-method depth of the sync section. Brings the async surface to
  parity with the directive: every public item has its own multi-
  example section.

### Changed

- **All documentation footers and prose version refs** updated to
  v0.8.0 in `docs/API.md`, `docs/PERFORMANCE.md`, `docs/SECURITY.md`,
  and `docs/PATTERNS.md`. The README async-feature install snippet
  bumped from `version = "0.7"` to `version = "0.8"`.
- **`src/lib.rs`** — `#![doc(html_root_url)]` bumped from `0.5.0`
  (which had been stale for three releases) to `0.8.0`. Going forward
  this constant is part of the release checklist.
- **`README.md`** — Documentation section now surfaces `PATTERNS.md`
  and `SECURITY.md` alongside `API.md` and `PERFORMANCE.md`.

### Fixed

- **Stale doc versions.** v0.5.0 footers in `API.md`, v0.6.0 footers
  in `PERFORMANCE.md`, v0.4.0 install snippet in older README copies,
  and `html_root_url = "0.5.0"` in `src/lib.rs` had all drifted away
  from the actual crate version. Every reference is now sync'd to
  v0.8.0 in a single sweep; release checklist updated to make this
  automatic next time.

[Unreleased]: https://github.com/jamesgober/registry-io/compare/v0.8.0...HEAD
[0.8.0]: https://github.com/jamesgober/registry-io/releases/tag/v0.8.0

---

## [0.7.0] - 2026-05-19

Hardening milestone. Property-based invariant testing,
`Arc`-strong-count leak verification, a fuzz-target scaffold for
`cargo-fuzz`, and a published threat model.

### Added

- **`tests/proptest_invariants.rs`** — six property tests over random
  operation sequences: register-then-unregister round-trip leaves the
  registry empty; `notify` fires exactly once per registered handler;
  handler ids stay unique across arbitrary churn; `handler_count`
  matches external bookkeeping; stale ids always return `false` from
  `unregister`; `clear()` does not let subsequent ids collide with
  pre-clear ids.
- **`tests/leak_check.rs`** — three `Arc::strong_count` canary tests
  covering 10 000 register / unregister cycles, `clear()` after 100
  registrations, and registry drop while 50 handlers are live.
- **`fuzz/` workspace scaffold** — `cargo-fuzz`-ready Cargo manifest
  plus two fuzz targets:
  - `handler_churn` — random sequences of register / unregister /
    clear / notify ops against a fresh `SyncRegistry`, with invariant
    checks after each step. Includes panicky handlers in the rotation.
  - `event_payload` — registers a fixed handler set and dispatches
    arbitrary event payloads, exercising the dispatch path against
    adversarial bytes.
  - `fuzz/.gitignore` excludes `target/`, `corpus/`, `artifacts/`,
    `coverage/`.
- **`docs/SECURITY.md`** — threat model, unsafe-code discipline, panic
  isolation in detail, fuzzing methodology, leak/zero-alloc
  verification summary, CI gate inventory, vulnerability reporting
  process.
- **`proptest` dev-dependency** — `proptest = "1"` (default features
  off, `std` only) — used exclusively in `tests/proptest_invariants.rs`.

### Changed

- **`Cargo.toml`** — version `0.6.0` → `0.7.0`; added `proptest`
  dev-dep.
- **`.github/workflows/ci.yml`** — bumped `actions/cache@v4` to
  `actions/cache@v5`. v5 declares `using: node24` in its manifest, so
  the `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24` env var added in 0.6.0 is no
  longer needed and was removed. CI runs are now warning-free for the
  Node.js deprecation pipeline.

### Security

- Public API remains free of `unsafe`. Verified by inspection;
  documented in `docs/SECURITY.md`.
- Panic isolation invariants newly covered by property tests and fuzz
  targets in addition to the existing example-based tests.
- Memory-leak posture across register / unregister churn and registry
  drop newly covered by `tests/leak_check.rs`.

[Unreleased]: https://github.com/jamesgober/registry-io/compare/v0.7.0...HEAD
[0.7.0]: https://github.com/jamesgober/registry-io/releases/tag/v0.7.0

---

## [0.6.0] - 2026-05-19

Performance verification milestone. The full benchmark suite has been
executed, the Performance Contract numbers from `.dev/ROADMAP.md` are
**all met with significant headroom**, and the zero-allocation property
of `SyncRegistry::notify` is verified by a `dhat`-backed test.

### Added

- **`benches/contention.rs`** — dedicated thread-contention sweep for
  `SyncRegistry::notify`. Combinatorial matrix of `{1, 4, 16, 64}`
  notifying threads × `{1, 4, 16}` handlers. Each benchmark uses
  `iter_custom` + `thread::scope` to put the worker spawn inside the
  timed region and reports ns *per notify call*.
- **`tests/zero_alloc.rs`** — dhat-based heap accounting test that
  verifies `notify()` allocates zero blocks across 100 000 calls with
  both an empty registry and an 8-handler registry. Gated behind the
  new `dhat-heap` feature so the global allocator swap doesn't affect
  other tests/benches.
- **`dhat-heap` Cargo feature** — when enabled, swaps the global
  allocator to `dhat::Alloc` for the zero-alloc test. `dhat 0.3` is a
  dev-dependency (does not propagate to downstream crates).
- **`docs/PERFORMANCE.md`** — extensively rewritten with **measured**
  numbers from the dev machine baseline (Windows 11, Intel x86-64,
  Rust 1.95, MSVC). Includes:
  - Headline sync notify table vs Performance Contract targets.
  - Full contention sweep (1 → 64 threads × 1 → 16 handlers).
  - Async notify table for both concurrent and sequential dispatch.
  - Register / unregister slow-path latency at `N = 0, 16, 100, 1000`.
  - Cost model, hot-path code walk, slow-path rcu pattern, memory
    footprint accounting.
  - Dispatch-mode guidance ("when to use concurrent vs sequential").
  - Reproduction commands for every bench.

### Changed

- **`src/async_registry/mod.rs`** — `notify` rewritten to do a single
  pass over the handler snapshot, building parallel `ids` and `wrapped`
  vectors instead of an intermediate `pairs` Vec. Saves one Vec
  allocation per `notify()` call. Functionally identical.
- **`Cargo.toml`** — version `0.5.0` → `0.6.0`. Added `[[bench]]` entry
  for `contention`. Added `dhat = "0.3"` dev-dep and `dhat-heap = []`
  feature.
- **`.github/workflows/ci.yml`** — set
  `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24: true` so `actions/cache@v4`
  (still on Node 20 internally) opts into Node 24 today instead of
  emitting the deprecation warning on every CI run.

### Verified

- All sync-notify targets in the Performance Contract met:
  `1 handler / 1 thread = 10.1 ns` (target `<20 ns`),
  `4 handlers / 1 thread = 12.5 ns` (target `<50 ns`),
  `4 handlers / 16 threads = 24.7 ns` (target `<50 ns`).
- Async-notify target met: `concurrent 1 handler = 177 ns`
  (target `<500 ns`).
- Zero-allocation property on `notify()` hot path confirmed by `dhat`
  across 100 000 calls.

[Unreleased]: https://github.com/jamesgober/registry-io/compare/v0.6.0...HEAD
[0.6.0]: https://github.com/jamesgober/registry-io/releases/tag/v0.6.0

---

## [0.5.0] - 2026-05-19

This release adds the **asynchronous** registry surface (`AsyncRegistry`)
behind the `async` feature flag, plus the supporting future-combinator
primitives required to run async handlers with the same panic-isolation
guarantees as the sync side.

### Added

- **`AsyncRegistry<E>`** — async-handler counterpart to `SyncRegistry`.
  Same lock-free `ArcSwap`-backed storage; handlers return a `'static`
  future of `()`. Methods: `new`, `with_capacity`, `register`,
  `register_with_priority`, `register_guard`, `register_guard_with_priority`,
  `unregister`, `clear`, `contains`, `handler_count`, `is_empty`,
  `on_panic`, `clear_panic_callback`, `notify` (concurrent), and
  `notify_sequential`. Re-exported at the crate root as
  `registry_io::AsyncRegistry` when the `async` feature is on.
- **`AsyncHandlerGuard<E>`** — RAII guard for async registrations.
  `#[must_use]`, drop-to-unregister, `forget()` to detach. Holds a
  `Weak<AsyncRegistry<E>>` so the registry can be dropped before the guard
  safely.
- **`crate::future_ext::CatchUnwind`** — internal future adapter that wraps
  each handler future and catches panics escaping `poll`. Mirrors
  `std::panic::catch_unwind` for the async path.
- **`crate::future_ext::JoinAll`** — internal future combinator that drives
  a `Vec<F>` of futures concurrently and resolves to a `Vec<F::Output>` in
  input order. Written in-tree to keep the dependency surface minimal
  (no `futures-util`).
- **`pub mod r#async`** — public module exposing the async registry and
  guard at `registry_io::r#async`.
- **Tests** — 38 new async integration tests covering registration, notify
  (both modes), priority ordering, panic isolation in both dispatch modes,
  guard drop semantics, panic-callback replacement and clearing, and
  custom panic payload round-trips. All exercised via `#[tokio::test]`.
- **Bench** — `benches/async_notify.rs` with concurrent and sequential
  scenarios at handler counts `0, 1, 4, 16`.
- **Examples** — `examples/async_basic.rs` and
  `examples/async_concurrent_vs_sequential.rs`. Both gated by
  `required-features = ["async"]` in `Cargo.toml`.
- **Documentation** — `docs/API.md` extended with the full `AsyncRegistry`
  and `AsyncHandlerGuard` reference, each with multiple runnable code
  examples. `README.md` quick-start now includes an async snippet.

### Changed

- **`Cargo.toml`** — version bumped to `0.5.0`; added `[[bench]]` for
  `async_notify` and `[[example]]` entries for the two new async examples,
  all gated by `required-features = ["async"]`. `tokio` dev-dependency now
  also enables the `time` feature (used by the
  `async_concurrent_vs_sequential` example).
- **`src/lib.rs`** — `panic` module is now gated on
  `any(feature = "sync", feature = "async")` (it's shared by both
  registries); `PanicInfo` re-export updated accordingly. New
  feature-gated `pub mod r#async;` declaration with `#[path]` pointing at
  `async_registry/mod.rs`.

[Unreleased]: https://github.com/jamesgober/registry-io/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/jamesgober/registry-io/releases/tag/v0.5.0

---

## [0.4.0] - 2026-05-19

This release ships the complete synchronous foundation: a fully functional
`SyncRegistry`, RAII handler guards, priority ordering, and panic isolation.
Phases 0.2.0, 0.3.0, and 0.4.0 from the roadmap are folded into a single
milestone.

### Added

- **`SyncRegistry<E>`** — generic, lock-free, panic-isolating event
  registry. Stores handlers as `Arc<dyn Fn(&E) + Send + Sync + 'static>`
  inside an `arc_swap::ArcSwap<Vec<...>>` snapshot.
  - `new`, `with_capacity`, `Default`, `Debug`.
  - `register` (default priority `0`) and
    `register_with_priority(priority, handler)`.
  - `register_guard` and `register_guard_with_priority` returning a
    `HandlerGuard` (RAII).
  - `unregister(id) -> bool`, `clear`, `contains`, `handler_count`,
    `is_empty`.
  - `notify(&event)` hot path: lock-free, allocation-free in the no-panic
    case, `#[inline]`. Each handler is wrapped in `std::panic::catch_unwind`.
  - `on_panic(callback)` and `clear_panic_callback()` for handler-panic
    observability.
- **`HandlerId`** — opaque, `Copy + Eq + Hash + Debug + Display` identifier
  with internal atomic-counter generator.
- **`PanicInfo<'a>`** — typed wrapper around a caught panic payload
  exposing `handler_id()`, `payload()`, and `message()`.
- **`HandlerGuard<E>`** — `#[must_use]` RAII handle returned by
  `register_guard*`. Drops to unregister; `forget()` consumes without
  unregistering. Holds a `Weak<SyncRegistry<E>>` so registry-before-guard
  drop order is safe.
- **Priority ordering** — descending priority on `notify`; equal priority
  fires in registration order. Insertion is `O(log N + N)` via
  `Vec::partition_point` + `Vec::insert`.
- **Panic isolation** — one panicking handler does not stop siblings or
  propagate to the caller. Second-order panics inside an `on_panic`
  callback are caught and dropped.
- **Benchmarks** — `benches/sync_notify.rs` (handler count + thread
  contention) and `benches/register_unregister.rs` (slow-path latency).
- **Examples** — `basic`, `priority`, `guards`, `panic_isolation`,
  `concurrent`.
- **Documentation** — `docs/API.md` (full API with multiple examples per
  item) and `docs/PERFORMANCE.md` (cost model and benchmark methodology).
- **Integration tests** — `tests/sync_registry.rs`, `tests/priority.rs`,
  `tests/panic_isolation.rs`, `tests/guards.rs`, `tests/concurrent.rs`.

### Changed

- **MSRV** bumped from `1.75` to `1.85` to support edition 2024.
- **Cargo features** — `sync` and `async` now explicitly imply `std`.
  Default features remain `["std", "sync"]`.
- **README** — replaced placeholder quick-start with working code examples
  for register/notify, priority ordering, RAII guards, and panic isolation.
- **`lib.rs`** — enabled `#![warn(clippy::pedantic)]` per directives; added
  `extern crate alloc` and feature-gated `extern crate std`.

### Security

- All handler invocations wrapped in `catch_unwind` to prevent
  unwind-related state corruption in callers.
- No `unsafe` code in the public API.

[0.4.0]: https://github.com/jamesgober/registry-io/releases/tag/v0.4.0

---

## [0.1.0] - 2026-05-18

### Added

- Initial scaffold and repository bootstrap.
- REPS compliance baseline.
- CI for Linux/macOS/Windows on stable and MSRV (1.75).
- Project documentation framework (PROMPT, DIRECTIVES, ROADMAP).

[0.1.0]: https://github.com/jamesgober/registry-io/releases/tag/v0.1.0

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

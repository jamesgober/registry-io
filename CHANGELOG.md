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

[Unreleased]: https://github.com/jamesgober/registry-io/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/jamesgober/registry-io/releases/tag/v0.4.0

---

## [0.1.0] - 2026-05-18

### Added

- Initial scaffold and repository bootstrap.
- REPS compliance baseline.
- CI for Linux/macOS/Windows on stable and MSRV (1.75).
- Project documentation framework (PROMPT, DIRECTIVES, ROADMAP).

[0.1.0]: https://github.com/jamesgober/registry-io/releases/tag/v0.1.0

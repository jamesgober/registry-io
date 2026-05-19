# registry-io — Platform Notes

`registry-io` targets Linux, macOS, and Windows as **first-class**
platforms. Every CI run exercises the full test suite on all three.
This document captures the per-platform nuances a maintainer or
downstream consumer should know about.

For overall performance numbers, see [`PERFORMANCE.md`](./PERFORMANCE.md).
For the v1.0 stability contract see
[`STABILITY-1.0.md`](./STABILITY-1.0.md).

---

## Supported platforms

| OS / Arch                  | Stable Rust | MSRV (1.85.0) | Notes                              |
|----------------------------|:-----------:|:-------------:|------------------------------------|
| Linux x86_64               | ✓           | ✓             | dev-machine baseline               |
| Linux aarch64              | not in CI   | not in CI     | should work; not gated             |
| macOS x86_64               | ✓           | ✓             |                                    |
| macOS aarch64              | ✓           | ✓             |                                    |
| Windows x86_64 (MSVC)      | ✓           | ✓             | dev-machine; SEH-based unwind      |
| Windows x86_64 (GNU)       | not in CI   | not in CI     | should work; not gated             |
| WASM (`wasm32-*`)          | **unsupported** | —          | `std::sync::Arc` semantics differ; `ArcSwap` not validated on wasm |
| `no_std`                   | **unsupported** | —          | `arc-swap`, `std::panic`, `std::sync::Arc` all required |

"Supported" means: every push to `main` runs `cargo fmt`, `cargo
clippy --all-targets --all-features`, `cargo test --all-features`,
and `cargo doc --no-deps --all-features` against the platform. A
failure on any of those blocks the release.

---

## Per-platform behavior

### Linux

The reference implementation. All published benchmark numbers are
captured here.

- **`catch_unwind` cost on the no-panic path**: essentially free
  (gcc-style unwind tables; runtime cost is a `mov` to a TLS slot).
- **`ArcSwap::load` cost**: ~2 ns, dominated by the thread-local
  cache hit path inside `arc-swap`.
- **`AtomicU64` access**: lock-free on every supported Linux target.

Nothing in the crate uses `epoll`, `io_uring`, or any other
Linux-specific facility. Everything is `std`.

### macOS

Behavior matches Linux. Performance is within margin-of-error of the
Linux numbers on equivalent hardware; we have not observed an
anomalous result on either x86_64 or aarch64 Macs.

The `dhat`-backed zero-allocation test
(`tests/zero_alloc.rs`) is exercised here too — replacing the global
allocator with `dhat::Alloc` is supported on macOS.

### Windows (MSVC)

The dev-machine baseline numbers in `PERFORMANCE.md` were captured
here.

- **`catch_unwind` cost on the no-panic path**: slightly higher than
  Linux because Windows uses SEH (structured exception handling).
  Measured median sync-notify-1-handler-1-thread is **10.1 ns** on
  the dev machine — already well under the `<20 ns` contract.
- **Long path names**: avoid checking out the repo to a path
  approaching the legacy Windows 260-character limit. Cargo's
  `target/` directory is deep and can hit the limit. If the
  repo path is constrained, set `CARGO_TARGET_DIR=C:\t\reg-io`
  or similar.
- **CRLF line endings**: enforced as **LF** for `*.rs`, `*.toml`,
  `*.md`, etc. via [`.gitattributes`](../.gitattributes). Without
  this, rustfmt's `newline_style = "Unix"` directive fails on
  Windows checkout. Don't disable the gitattributes.

### Windows (GNU)

Not gated in CI. Should work — the crate has no MSVC-specific
intrinsics or link directives. If you depend on this configuration,
the suggested next step is contributing a matrix entry to
`.github/workflows/ci.yml`.

---

## Build prerequisites

### Common to all platforms

- **Rust toolchain**: stable 1.85.0 or newer.
- **`cargo`** (bundled with rustup).

That's it for using the crate. No external native dependencies
(arc-swap is pure Rust; tokio is dev-only).

### Linux / macOS extras (development)

- `git` for cloning.
- `cargo bench` and the `dhat-heap` feature work out of the box.

### Windows extras (development)

- A Visual Studio Build Tools install with C++ tooling — required
  by some transitive dev-dependencies (`backtrace` via `dhat` uses
  cc-rs).
- PowerShell 5.1 or newer for the dev-driver scripts mentioned in
  `docs/SECURITY.md`'s fuzz section.

### Fuzz target (any platform)

- Rust **nightly** + `cargo install cargo-fuzz`.
- The fuzz binaries are not part of the parent workspace; build from
  inside `fuzz/`.

---

## CI matrix

From [`.github/workflows/ci.yml`](../.github/workflows/ci.yml):

```yaml
matrix:
  os:   [ubuntu-latest, macos-latest, windows-latest]
  rust: [stable, "1.85.0"]
```

Six job lanes per push. Each lane runs:

```
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo doc --no-deps --all-features        # with RUSTDOCFLAGS=-D warnings
```

`actions/cache@v5` is used for the `~/.cargo/{registry,git}` and
`target/` directories — this is the Node.js-24-native release of the
GitHub cache action (v4 was Node 20 and is being retired). Cache
keys are scoped by `{os, rust, Cargo.lock-hash}`.

---

## Known platform quirks

### `dhat::Profiler` is process-global

The dhat-backed zero-allocation test creates a single profiler instance.
Two concurrent `#[test]` functions inside the same binary would each
try to construct one and panic. We work around this by combining the
two scenarios into one `#[test]` (`tests/zero_alloc.rs`). This is
documented inline in that test.

This is the same constraint on Linux, macOS, and Windows — listed
here because it's a thing a contributor might trip on while extending
the test.

### Stable-Rust feature gates

The crate compiles under stable Rust. Two stable-only assumptions
are baked in:

- **Edition 2024** — requires `rustc 1.85.0+`. The Cargo manifest's
  `rust-version = "1.85"` blocks older toolchains with a clear error.
- **`std::panic::catch_unwind`** — stable since Rust 1.9. Not feature-
  gated.

We do not currently use `#![feature(...)]` anywhere. Nightly is only
needed to *run the fuzz targets*; the library itself builds cleanly
on stable everywhere.

### Allocator interactions

The crate does not install a global allocator. The `dhat-heap`
feature installs `dhat::Alloc` in the **test binary**
(`tests/zero_alloc.rs`) — that override is scoped to that test crate
and does not leak into the lib or other tests. If you use a custom
allocator (`jemalloc`, `mimalloc`, ...) in your application, all
registry operations work identically; the zero-allocation guarantee
on `notify` is independent of allocator choice.

### Threads and stack size

The crate spawns no threads of its own. Worker threads in tests/benches
use the default OS stack size (8 MB on Linux, 512 KB to 1 MB on macOS,
1 MB on Windows). Handler closures with very large stack-resident
captures might require a larger thread stack — but a closure that
large should hold its captures behind an `Arc<T>` instead.

---

## Reporting a platform-specific bug

If you observe behavior that differs between platforms in a way this
document doesn't predict, please file an issue with:

- `rustc --version --verbose` output.
- `cargo --version` output.
- The OS version (`uname -a` / `sw_vers` / `winver`).
- A minimal reproduction. If it's perf-related, please include the
  exact `cargo bench` invocation and the median + IQR from criterion.

See [`SECURITY.md`](./SECURITY.md) for the vulnerability-reporting
process if the divergence is security-relevant.

---

<sub>registry-io v0.9.0 — Copyright © 2026 James Gober. Apache-2.0 OR MIT.</sub>

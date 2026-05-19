# registry-io — Security

This document captures the security posture of `registry-io 0.9.0`: what
the crate promises, what it does not, and how that posture is verified.

For performance characteristics see [`PERFORMANCE.md`](./PERFORMANCE.md). For
the full API surface see [`API.md`](./API.md).

---

## Threat model

`registry-io` is an **in-process** primitive. It exposes a typed registry
of closures and an event-dispatch entry point. All callers are inside
the same Rust process and share the same address space.

The crate is therefore not in scope for:

- **Untrusted-input parsing.** Events are typed Rust values constructed
  by the caller. No deserialization happens inside the crate.
- **Network or IPC boundaries.** The registry does not open sockets,
  files, pipes, or shared memory.
- **Confidentiality of event payloads.** Handlers see the event the
  caller passed them. There is no encryption, no redaction, no
  access-control layer between handler and event.
- **Authentication or authorization.** Anything with a handle to the
  registry can register a handler or fire `notify`. There is no
  per-handler permission model.

The threats the crate **does** defend against, in order:

| Concern                                                  | Mitigation                                                                                  |
|----------------------------------------------------------|---------------------------------------------------------------------------------------------|
| A panicking handler tearing down sibling handlers        | `std::panic::catch_unwind` around every handler invocation, both sync and async             |
| A panicking handler propagating into the caller's stack  | Same: `notify` returns normally, the panic does not escape                                  |
| A second-order panic inside the `on_panic` callback      | Inner `catch_unwind` swallows the recursive panic; the original handler's siblings continue |
| A long-lived registry leaking handler state              | Verified by `tests/leak_check.rs` over 10 000 register / unregister cycles                  |
| Allocations creeping into the sync `notify` hot path     | Verified by `tests/zero_alloc.rs` (dhat) over 100 000 calls                                 |
| Unsafe code regressions                                  | The public API contains **zero** `unsafe` blocks; arc-swap's `unsafe` is audited upstream   |
| Supply-chain regressions                                 | `cargo audit` and `cargo deny check` (see "CI gates" below)                                 |

---

## Unsafe-code discipline

The public API of `registry-io` does **not** use `unsafe`. As of v0.7.0
the crate's own source compiles under `#![deny(unsafe_op_in_unsafe_fn)]`
with **zero** `unsafe { }` blocks. Two dependencies use `unsafe`:

- **`arc-swap`** — uses `unsafe` for the lock-free `Arc` swap primitive.
  Widely deployed, well-reviewed, and actively maintained.
- **`std`** — the standard library, which is part of the trusted base.

If any future patch introduces `unsafe` in this crate's own source, it
MUST carry a `// SAFETY:` justification per the REPS rules and MUST be
covered by targeted tests.

---

## Panic isolation in detail

The directive in `.dev/DIRECTIVES.md` makes panic isolation a hard
requirement. Implementation:

```rust
// src/sync/mod.rs, in notify():
let result = catch_unwind(AssertUnwindSafe(|| handler(event)));
if let Err(payload) = result {
    self.handle_panic(entry.id, payload);
}
```

```rust
// src/future_ext.rs, inside CatchUnwind::poll():
match catch_unwind(AssertUnwindSafe(|| inner_pin.poll(cx))) {
    Ok(Poll::Ready(out)) => { /* return Ok */ }
    Ok(Poll::Pending)    => Poll::Pending,
    Err(payload)         => { /* return Err */ }
}
```

Both the sync and async paths wrap every handler invocation in
`catch_unwind`. The `AssertUnwindSafe` adapter is required because
trait-object closures (`Arc<dyn Fn>`) and arbitrary user captures are
not statically `UnwindSafe`. Our safety justification:

- The registry's own state is held behind `ArcSwap` snapshots. A
  panicking handler cannot mutate the snapshot it is iterating over,
  because the snapshot is immutable for the duration of the iteration.
- The handler's own captured state may be left in a partially-updated
  state by the panic. That is exactly what happens with a
  non-`catch_unwind` panic in synchronous code, and the registry does
  not pretend otherwise.

Tests:

- `tests/panic_isolation.rs` — 9 tests covering sibling survival,
  callback id+message capture, replace/clear, callback-panics-too,
  custom panic payload downcast, silent default, count stability.
- `tests/async_panic.rs` — 11 tests, mirroring the sync coverage for
  both `notify` (concurrent) and `notify_sequential` dispatch modes.

---

## Fuzzing

Two `cargo-fuzz` targets live under `fuzz/`:

- **`handler_churn`** — drives a random sequence of register /
  unregister / clear / notify operations against a fresh registry and
  asserts that `handler_count`, `contains`, and `unregister` return
  values stay consistent with the test's own bookkeeping. Includes
  panicky handlers in the rotation.
- **`event_payload`** — registers a fixed set of handlers (including
  one that panics on even tags) and dispatches arbitrary event values
  drawn from the fuzzer's byte stream. Exercises the dispatch path
  against adversarial event payloads.

### Running locally

```bash
# Requires nightly + cargo-fuzz.
rustup install nightly
cargo install cargo-fuzz

cargo +nightly fuzz run handler_churn   -- -max_total_time=300
cargo +nightly fuzz run event_payload   -- -max_total_time=300
```

`-max_total_time=300` is a 5-minute soak. The 1.0 release contract
calls for at least one full **CPU-hour** soak per target with no
findings before the release tag is cut.

### Corpus

The `fuzz/corpus/` directory is `.gitignore`d. Interesting inputs
discovered locally should be filed as test cases in `tests/` instead of
checked into the corpus.

### Findings

To date: **no crashes, no hangs, no UB reports**. This document will be
updated if that ever changes.

---

## Memory-leak verification

`tests/leak_check.rs` exercises three scenarios using `Arc::strong_count`
as a canary:

1. **`register_unregister_churn_does_not_leak_handler_closures`** —
   10 000 cycles. Asserts the canary's strong count stays `<= 4`.
   The tiny slack accommodates `arc-swap`'s thread-local snapshot cache.
2. **`clear_drops_all_handler_closures`** — register 100 handlers, then
   `clear()`. Same canary bound.
3. **`dropping_registry_releases_all_handler_closures`** — register 50
   handlers, then drop the registry. The canary must return to exactly
   `1` (the test's own outside-the-registry reference).

If a future patch accidentally retains a handler — for example by
caching it in a side map that `unregister` doesn't touch — these tests
will fail loudly and immediately.

---

## Zero-allocation verification

`tests/zero_alloc.rs` (under the `dhat-heap` feature) installs
`dhat::Alloc` as the global allocator, runs 100 000 `notify(&v)` calls
in two configurations (empty registry, 8-handler registry), and asserts
that **zero** new heap blocks are created.

This bounds the worst-case observable side effect of the hot path: a
caller that fires `notify` in a tight loop sees no GC churn, no
allocator contention, no fragmentation pressure. The zero-allocation
property is part of the 1.0 contract.

---

## CI gates

Every push and pull request runs the following on Linux, macOS, and
Windows, against stable Rust and MSRV (1.85.0):

```
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo doc --no-deps --all-features        # -D warnings
```

Planned for the 1.0.0 release-candidate phase:

- `cargo audit` — RustSec advisory database scan.
- `cargo deny check` — license + banned-crates policy enforcement.

---

## Reporting a vulnerability

Open a GitHub security advisory on the repository (preferred), or
contact the maintainer directly at the address listed in
`Cargo.toml#package.authors`. Please include:

- Affected version(s).
- A minimal reproduction.
- Your assessment of impact and exploitability.

Public CVE coordination, if warranted, will be done after a fix is in
place.

---

<sub>registry-io v0.9.0 — Copyright © 2026 James Gober. Apache-2.0 OR MIT.</sub>

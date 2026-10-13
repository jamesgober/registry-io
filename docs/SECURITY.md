# registry-io — Security

This document captures the security posture of `registry-io 1.1.0`: what
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
| Unsafe code regressions                                  | No `unsafe` in the public API; internal `unsafe` in two modules only, checked by Miri       |
| A freed handler list being read (use-after-free)         | Hazard-pointer protocol with `SeqCst` handshake; Miri data-race and weak-memory runs        |
| A handler still running after `unregister_and_wait`      | Covered by `tests/wait_and_once.rs`, including a soak test                                  |
| Supply-chain regressions                                 | Zero runtime dependencies by default; `cargo audit` and `cargo deny check` (see "CI gates") |

---

## Unsafe-code discipline

The public API of `registry-io` does **not** use `unsafe`: no public
function is `unsafe`, and no public type requires callers to uphold an
invariant the compiler does not check.

Since 1.1.0 the crate contains `unsafe` code of its own, confined to
two internal modules:

- **`src/hazard.rs`** — the process-wide hazard-pointer domain:
  dereferencing per-thread announcement records, which are leaked on
  creation and never freed.
- **`src/snapshot.rs`** — `SnapCell<T>`, the hazard-protected `Arc<T>`
  behind both registries: converting between `Arc<T>` and the raw
  pointer it publishes, borrowing the value under a validated hazard,
  and the `Send`/`Sync` impls (same bounds as `Arc<T>`).

Rules these modules follow:

- Every `unsafe` block carries a `// SAFETY:` comment stating the
  invariant it relies on. The crate denies
  `clippy::undocumented_unsafe_blocks`, `clippy::missing_safety_doc`,
  and `unsafe_op_in_unsafe_fn`, so a block without a justification
  fails the build.
- No `unsafe` appears anywhere else in the crate.
- The design and its correctness argument are written up in
  [`ARCHITECTURE.md`](./ARCHITECTURE.md#why-it-is-correct).

### Miri

Before the 1.1.0 release both modules were run under
[Miri](https://github.com/rust-lang/miri), which checks for undefined
behavior, Stacked Borrows aliasing violations, and data races, with
weak-memory emulation enabled:

- over the core tests;
- over the concurrency tests across 64 random thread schedules.

Miri found two bugs during development. Both were fixed before release:

1. **Pointer provenance in the owned-load path.** The pointer used to
   bump the reference count had been re-derived from a `&T`, whose
   provenance does not cover the `Arc`'s reference counts. The code now
   keeps the pointer that `Arc::into_raw` produced.
2. **Memory-ordering race in record publication.** With an acquire
   load of the record-list head, a writer's announcement scan and a
   new thread's first announcement could miss each other, letting a
   list in use be freed. The head is now published and loaded with
   `SeqCst`, so either the scan sees the new record or the new thread's
   validating load sees the writer's swap and retries.

Miri runs are not part of the per-push CI matrix (they need nightly and
take much longer than the regular suite). They were run for the 1.1.0
release and should be repeated for any change to `src/hazard.rs` or
`src/snapshot.rs`.

### Dependencies

With default features the crate has **no runtime dependencies**; only
`std` (part of the trusted base) is used. The `async` feature adds the
optional `futures-core` dependency, which is declared but not
currently imported. Dev-dependencies (`criterion`, `tokio`, `dhat`,
`proptest`) do not reach downstream crates.

---

## Panic isolation in detail

The directive in `.dev/DIRECTIVES.md` makes panic isolation a hard
requirement. Implementation:

```rust
// src/sync/mod.rs, in notify():
if let Err(payload) = catch_unwind(AssertUnwindSafe(|| handler(event))) {
    self.handle_panic(entry.id, payload);
}
```

```rust
// src/future_ext.rs: building the future, then polling it.
pub(crate) fn start(make: impl FnOnce() -> BoxFuture) -> Result<BoxFuture, Payload> {
    catch_unwind(AssertUnwindSafe(make))
}

pub(crate) fn poll_caught(fut: &mut BoxFuture, cx: &mut Context<'_>) -> Poll<Result<(), Payload>> {
    match catch_unwind(AssertUnwindSafe(|| fut.as_mut().poll(cx))) {
        Ok(Poll::Ready(())) => Poll::Ready(Ok(())),
        Ok(Poll::Pending) => Poll::Pending,
        Err(payload) => Poll::Ready(Err(payload)),
    }
}
```

Both the sync and async paths wrap every handler invocation in
`catch_unwind`. On the async side that covers both the synchronous call
that builds the handler's future (`start`) and every `poll` of it.
Before 1.1.0 the building call was outside any unwind boundary, so a
handler that panicked before returning its future escaped
`AsyncRegistry::notify`; `tests/async_start_panic.rs` guards the fix.

The `AssertUnwindSafe` adapter is required because trait-object
closures (`Arc<dyn Fn>`) and arbitrary user captures are not
statically `UnwindSafe`. Our safety justification:

- The registry's own state is an immutable handler list, borrowed under
  a hazard announcement. A panicking handler cannot mutate the list it
  is iterating over, and the announcement is cleared by a `Drop` impl,
  so it is released even when the panic unwinds through `notify`.
- Writers do nothing under the writer mutex that a panic could leave
  half-done: handler `Drop` code runs after the lock is released, so a
  panicking destructor cannot poison the registry's internal state.
- The handler's own captured state may be left in a partially-updated
  state by the panic. That is exactly what happens with a
  non-`catch_unwind` panic in synchronous code, and the registry does
  not pretend otherwise.

Tests:

- `tests/panic_isolation.rs` — 9 tests covering sibling survival,
  callback id+message capture, replace/clear, callback-panics-too,
  custom panic payload downcast, silent default, count stability.
- `tests/async_panic.rs` — 10 tests, mirroring the sync coverage for
  both `notify` (concurrent) and `notify_sequential` dispatch modes.
- `tests/async_start_panic.rs` — 3 tests for panics raised while a
  handler builds its future, in both dispatch modes.
- `tests/wait_and_once.rs` — includes `register_once` removal on panic
  and `panic_count` with and without a callback.

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

As of v1.0.0: **no crashes, no hangs, no UB reports**. The 1-CPU-hour
soak per target was performed before the 1.0.0 cut. This document is
updated if any future run uncovers a finding.

---

## Memory-leak verification

`tests/leak_check.rs` exercises three scenarios using `Arc::strong_count`
as a canary:

1. **`register_unregister_churn_does_not_leak_handler_closures`** —
   10 000 cycles. Asserts the canary's strong count stays `<= 4`.
   The small slack is a margin for retired handler lists: a list a
   write replaces is freed by that write unless another thread is still
   reading it, in which case the last such reader or the next write
   frees it. In this single-threaded test nothing should remain.
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
cargo clippy --all-targets --no-default-features --features sync -- -D warnings
cargo clippy --all-targets --no-default-features --features async -- -D warnings
cargo doc --no-deps --all-features        # -D warnings
```

Periodic maintenance gates (run before each v1.x.y publish):

- `cargo audit` — RustSec advisory database scan; **clean** as of
  v1.1.0.
- `cargo deny check` — license + banned-crates policy enforcement,
  configured in `deny.toml`; **clean** as of v1.1.0. (`deny.toml` was
  missing before 1.1.0, which made the check reject every license.)
- `cargo public-api diff` — guards against unintentional public-API
  changes between releases.

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

<sub>registry-io v1.1.0 — Copyright © 2026 James Gober. Apache-2.0 OR MIT.</sub>

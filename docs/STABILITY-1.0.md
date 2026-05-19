# registry-io — v1.0 Stability Contract

This document **is** the contract that `registry-io 1.0.0` ships under.
It enumerates **exactly** what is frozen, **exactly** what may still
change, and the mechanics by which any of that changes again.

> Status: **in effect** as of v1.0.0. Anything in here that later needs
> to be broken requires a **MAJOR** version increment (2.0.0).

For the milestone plan see [`.dev/ROADMAP.md`](../.dev/ROADMAP.md). For the
performance guarantees that back the API see [`PERFORMANCE.md`](./PERFORMANCE.md).

---

## What v1.x guarantees

### Public API surface

The following items are **frozen** for the lifetime of v1.x as of
v1.0.0. Their *signatures*, *semantics*, and *thread-safety* do not
change in a backwards-incompatible way within v1.x; any such change
ships as v2.0.0.

| Item                              | Kind   | Notes                                |
|-----------------------------------|--------|--------------------------------------|
| `registry_io::VERSION`            | const  | crate version string                 |
| `registry_io::HandlerId`          | struct | `Copy + Eq + Hash + Debug + Display` |
| `registry_io::HandlerId::as_u64`  | fn     | diagnostic accessor                  |
| `registry_io::PanicInfo<'_>`      | struct | not constructible by callers         |
| `PanicInfo::handler_id`           | fn     |                                      |
| `PanicInfo::payload`              | fn     |                                      |
| `PanicInfo::message`              | fn     |                                      |
| `SyncRegistry<E>`                 | struct | `E: Send + Sync + 'static`           |
| `SyncRegistry::new`               | fn     |                                      |
| `SyncRegistry::with_capacity`     | fn     |                                      |
| `SyncRegistry::register`          | fn     | priority `0` default                 |
| `SyncRegistry::register_with_priority` | fn |                                      |
| `SyncRegistry::register_guard`    | fn     | requires `Arc<Self>`                 |
| `SyncRegistry::register_guard_with_priority` | fn |                              |
| `SyncRegistry::unregister`        | fn     |                                      |
| `SyncRegistry::clear`             | fn     |                                      |
| `SyncRegistry::contains`          | fn     |                                      |
| `SyncRegistry::handler_count`     | fn     |                                      |
| `SyncRegistry::is_empty`          | fn     |                                      |
| `SyncRegistry::on_panic`          | fn     |                                      |
| `SyncRegistry::clear_panic_callback` | fn  |                                      |
| `SyncRegistry::notify`            | fn     | hot path                             |
| `HandlerGuard<E>`                 | struct | `#[must_use]`                        |
| `HandlerGuard::id`                | fn     |                                      |
| `HandlerGuard::forget`            | fn     |                                      |
| `HandlerGuard::drop`              | impl   | Drop semantics specified             |
| `AsyncRegistry<E>` *(feat: async)* | struct |                                     |
| (all async methods mirroring sync) | fn    |                                      |
| `AsyncRegistry::notify`           | fn     | concurrent dispatch                  |
| `AsyncRegistry::notify_sequential` | fn    | sequential dispatch                  |
| `AsyncHandlerGuard<E>`            | struct | `#[must_use]`                        |
| `AsyncHandlerGuard::id`           | fn     |                                      |
| `AsyncHandlerGuard::forget`       | fn     |                                      |

#### Behavioral guarantees

- **`SyncRegistry::notify` is lock-free and allocation-free** on the
  no-panic path. This is enforced by `tests/zero_alloc.rs` and the
  measured numbers in `PERFORMANCE.md`.
- **Panic isolation**. A panic in one handler is caught by
  `std::panic::catch_unwind`, does not affect sibling handlers, and
  does not propagate out of `notify`. The `on_panic` callback receives
  a `PanicInfo` for every panicking handler.
- **Priority ordering**. Higher priority fires first. Equal priority
  fires in registration order (stable). This holds across registrations,
  unregistrations, and `clear`s.
- **`HandlerGuard` drop semantics**. Dropping the guard *attempts*
  to unregister via a `Weak<SyncRegistry<E>>` upgrade. If the registry
  has been dropped already, the drop is a no-op. The handler will *not*
  be re-registered on guard drop.
- **`HandlerId` uniqueness**. Within a single registry, every
  successful `register*` returns a previously-unused id, for the lifetime
  of that registry. Ids are **not** comparable across registries.
- **`Send + Sync`**. `SyncRegistry<E>`, `HandlerGuard<E>`,
  `AsyncRegistry<E>`, and `AsyncHandlerGuard<E>` are all `Send + Sync`
  for any `E: Send + Sync + 'static`.

#### Trait implementations

- `HandlerId: Copy + Clone + PartialEq + Eq + Hash + Debug + Display`
- `PanicInfo<'_>: Debug`
- `SyncRegistry<E>: Send + Sync + Debug + Default`
- `HandlerGuard<E>: Send + Sync + Debug`
- `AsyncRegistry<E>: Send + Sync + Debug + Default`
- `AsyncHandlerGuard<E>: Send + Sync + Debug`

A `Default` impl is **not** added to a type that doesn't already
have one in 1.0 — that would be a behavior addition users could begin
to depend on, and removing it later would break.

### Cargo metadata

| Property                | Value                       |
|-------------------------|-----------------------------|
| **Edition**             | `2024`                      |
| **MSRV**                | Rust `1.85` (frozen for 1.x)|
| **License**             | `Apache-2.0 OR MIT`         |

#### Feature flags

| Flag       | Default | Frozen for 1.x?       |
|------------|:-------:|-----------------------|
| `std`      | ✓       | yes                   |
| `sync`     | ✓       | yes                   |
| `async`    | —       | yes                   |
| `hybrid`   | —       | yes (alias of `sync` + `async`) |
| `metrics`  | —       | **reserved**; no items behind it yet |
| `dhat-heap`| —       | yes (dev-only)        |

Features are **additive**. Enabling a feature will never remove or
disable an item exposed by another feature.

### Performance contract

The numbers in [`PERFORMANCE.md`](./PERFORMANCE.md) define the *upper
bound* on dispatch latency. v1.x does **not** regress past any of:

- Sync notify, 1 handler, 1 thread — `<20 ns` (measured: 10.1 ns)
- Sync notify, 16 handlers, 1 thread — `<200 ns` (measured: 26.0 ns)
- Sync notify, 4 handlers, 16 threads contended — `<50 ns` (measured: 24.7 ns)
- Async notify (concurrent), 1 handler — `<500 ns` (measured: 177 ns)
- **Zero heap allocations** on the sync notify no-panic path
  (verified by `tests/zero_alloc.rs`)

A change that knowingly regresses any of these does not ship in a
`1.x.y` patch — it requires the breaking-change procedure (v2.0.0).

---

## What can still change in v1.x

The following are explicitly **not** frozen and may evolve within v1.x
without a major version bump.

### Additions

- New methods on existing types — **provided** the existing methods
  retain their signatures and semantics.
- New types (e.g., a future `HybridRegistry<E>`) behind new feature
  flags.
- New variants on `#[non_exhaustive]` enums.
- Performance improvements that exceed the contract.

### Implementation details

- The internal representation of `HandlerId` (currently `u64`).
- The internal storage layout (`ArcSwap<Vec<HandlerEntry<E>>>`).
- The exact algorithm used for priority-sorted insertion.
- The set of trait bounds on internal types.
- Whether `notify` uses `JoinAll` for the async case or some equivalent
  combinator.
- Internal allocation patterns on slow paths — provided the public
  hot-path zero-allocation guarantee holds.

If a downstream depends on any of these, it has done so without
contract support.

### Dependencies

- `arc-swap` may receive minor/patch bumps with no semver impact on
  registry-io.
- `futures-core` is **reserved**; not currently used directly.
- New dependencies may be added behind opt-in features.
- The `[dev-dependencies]` set may change freely.

---

## Versioning policy

### Numbered versions

| Bump  | Triggered by                                                |
|-------|-------------------------------------------------------------|
| MAJOR | Any change to the items in **Public API surface** above; any change to behavioral guarantees; any feature-flag removal |
| MINOR | New methods, types, or feature flags; performance improvements; new error variants behind `#[non_exhaustive]` |
| PATCH | Bug fixes, doc updates, internal refactors with no behavioral change, dependency bumps that don't break the public contract |

We do not ship `0.x.y` releases against the v1.x line. All 1.x
versions are stable. Pre-release flavors (alpha/beta/rc) of a *future*
2.0 may exist alongside 1.x.

### Deprecations

A deprecation is announced with `#[deprecated(since = "X.Y.Z", note = "use … instead")]`
in a **minor** release. Deprecated items are removed in the **next
major** release after deprecation, never before. Migration guidance
is documented in the corresponding release notes.

### Yank policy

A 1.x patch release will be yanked if:

- It fails to build under stable Rust on a supported platform.
- It introduces undefined behavior on a supported configuration.
- It breaks the **Public API surface** above in a way that escaped
  pre-publish review.

A yank is announced in a corresponding release-notes block. The
following release supersedes the yanked one with the fix.

---

## What's out of scope forever

These are not v1.x features and are not on the v2.0 roadmap either.

- **Cross-process delivery.** Use NATS, Redis, or another message
  broker.
- **Distributed registries.** Consensus is a different problem.
- **Persistent event log.** Use an event store.
- **Schema management.** Events are typed Rust values.
- **Authentication / authorization.** Anything with a handle to the
  registry can subscribe.

---

## How this contract is enforced

- **CI gates** — every push runs fmt, clippy strict, full test suite,
  doc with `-D warnings` on Linux/macOS/Windows against stable + MSRV.
- **`tests/zero_alloc.rs`** — guards the zero-allocation guarantee.
- **`tests/leak_check.rs`** — guards drop semantics across churn.
- **`tests/proptest_invariants.rs`** — guards the behavioral guarantees
  above with property-based randomization.
- **`tests/panic_isolation.rs` + `tests/async_panic.rs`** — guards
  panic-isolation semantics.
- **Benchmark suite** (`benches/`) — guards the performance contract.
  Pre-merge: any change touching `notify` must include a bench run; a
  regression `>5%` blocks the merge.
- **`cargo public-api diff`** — catches accidental signature changes
  between releases. Run before each v1.x.y publish.

---

## Reviewing this document

When something on the **frozen** list needs to change, the procedure is:

1. **Stop.** The change is a major version bump. Do not commit it on
   a 1.x branch.
2. Open a tracking issue stating exactly what would change and why.
3. Update this document on the v2.0 branch.
4. Hold the change until at least one full 1.x release cycle has
   communicated the deprecation.

There is no fast path. The cost of breaking semver after publishing a
stable crate is higher than any single feature.

---

<sub>registry-io v1.0.0 — Copyright © 2026 James Gober. Apache-2.0 OR MIT.</sub>

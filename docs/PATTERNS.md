# registry-io — Integration Patterns

This document captures the four canonical use cases `registry-io` was
built to serve. Each pattern names the **problem**, sketches the
**solution shape**, lists **why a registry beats the obvious
alternatives**, and links to a runnable example you can compare against
your own integration.

For the underlying API, see [`API.md`](./API.md). For cost numbers, see
[`PERFORMANCE.md`](./PERFORMANCE.md).

---

## Table of contents

- [Pattern 1 — Hot-reload notification](#pattern-1--hot-reload-notification)
- [Pattern 2 — Audit-log fan-out](#pattern-2--audit-log-fan-out)
- [Pattern 3 — Metric-event collection](#pattern-3--metric-event-collection)
- [Pattern 4 — Transaction state-change hooks](#pattern-4--transaction-state-change-hooks)
- [Choosing between sync and async](#choosing-between-sync-and-async)
- [Mistakes to avoid](#mistakes-to-avoid)

---

## Pattern 1 — Hot-reload notification

**Problem.** A single producer (a config file watcher, a control plane
push, an admin RPC) mutates a shared configuration value. Many
subscribers — connection pools, HTTP clients, rate-limiter parameters,
feature-flag caches — must re-derive their internal state the moment
the mutation lands. The mutation rate is low (seconds to minutes
apart); the *number of subscribers* is high (tens to hundreds).

**Solution shape.**

```text
Config { snapshot: ArcSwap<Snapshot>, on_change: Arc<SyncRegistry<Snapshot>> }

config.mutate(|snap| snap.field = new) {
    // 1. Atomically swap the new snapshot into ArcSwap.
    // 2. Call on_change.notify(&new_snapshot) — fans out to every subscriber.
}

subscriber.register(move |snap: &Snapshot| {
    // Re-derive subscriber-local state from snap.
});
```

**Why a registry beats the obvious alternatives.**

| Alternative                          | What you give up                                            |
|--------------------------------------|-------------------------------------------------------------|
| `mpsc::channel` per subscriber       | One allocation per event per subscriber; per-subscriber wiring|
| `tokio::sync::broadcast`             | Async-only; bounded; allocates per-receiver buffer          |
| `Arc<Mutex<Vec<Callback>>>`          | Lock contention on every notify and every (un)subscribe     |
| Polling the snapshot generation      | Latency proportional to poll interval                       |

**Runnable example:**
[`examples/pattern_hot_reload.rs`](../examples/pattern_hot_reload.rs).

---

## Pattern 2 — Audit-log fan-out

**Problem.** A privileged action produces an audit event. The event
must be **persisted** to multiple sinks — stdout for local
operators, an append-only file for forensics, a remote SIEM for
correlation, a metrics counter for dashboards. The producer must
**not** know how many sinks are attached, what kind they are, or
whether any of them is currently misbehaving.

**Solution shape.**

```text
let bus = Arc::new(SyncRegistry::<AuditEvent>::new());

struct FileSink     { _guard: HandlerGuard<AuditEvent>, /* … */ }
struct SiemSink     { _guard: HandlerGuard<AuditEvent>, /* … */ }
struct CounterSink  { _guard: HandlerGuard<AuditEvent>, /* … */ }

// Each sink registers via bus.register_guard(...) at construction time.
// Dropping the sink drops the guard, which deregisters cleanly.

audit_producer.emit(event) {
    bus.notify(&event);  // ~12 ns for 4 handlers
}
```

**Properties of interest.**

- **Panic isolation per sink.** A file-system error in the file sink
  doesn't stop the SIEM sink from shipping the event. `notify` wraps
  every handler in `catch_unwind`.
- **RAII attachment.** Sinks own their `HandlerGuard`. No explicit
  detach API needed — drop the sink, the registration is gone.
- **No central registry of sinks to maintain.** The bus is the
  registry.

**Runnable example:**
[`examples/pattern_audit_fanout.rs`](../examples/pattern_audit_fanout.rs).

---

## Pattern 3 — Metric-event collection

**Problem.** A request handler in the hot path needs to emit
fine-grained metric events: request started, request completed with
latency, error of kind X. The metric *consumer* — Prometheus, StatsD,
Datadog, a logging pipeline — must not be on the request's critical
path. A synchronous push to any of those adds milliseconds of jitter.

**Solution shape.**

```text
let bus = SyncRegistry::<MetricEvent>::new();

// Handler 1: lock-free in-process counters. Fires in ns.
bus.register(|evt| match evt {
    Started      => counters.started.fetch_add(1, …),
    Completed{us} => { counters.completed.fetch_add(1, …);
                       counters.latency_us.fetch_add(*us, …); }
    Error        => counters.errors.fetch_add(1, …),
});

// Handler 2: batcher for an out-of-band exporter.
bus.register(|evt| {
    pending_events.lock().unwrap().push(evt.clone());
});

// Out of band: an exporter task drains `pending_events` periodically.
```

**Properties of interest.**

- **Hot-path cost is bounded by the cheapest handler.** All handlers
  fire inline; the slow ones still cost what they cost. Keep
  `notify` handlers cheap. Offload to a worker pool from inside the
  handler if necessary.
- **Zero-allocation aggregator option.** Atomic counters in a struct
  that lives behind an `Arc` add no allocations per event.

**Runnable example:**
[`examples/pattern_metric_event.rs`](../examples/pattern_metric_event.rs).

---

## Pattern 4 — Transaction state-change hooks

**Problem.** A transaction manager transitions a `Tx` through
`Begun → Prepared → Committed | Aborted | RecoveredFromCrash`. Each
transition must trigger external work in a **specific order**: write
the WAL flush before invalidating the cache before shipping the
replication update before incrementing the metric counter. The
transaction manager itself must remain ignorant of any of those
downstream concerns.

**Solution shape.**

```text
let bus = SyncRegistry::<TransactionEvent>::new();

// Highest priority — WAL flush before anyone observes commit.
bus.register_with_priority(1000, |evt| journal.flush(evt.txid));

// Medium priority — invalidate caches after WAL is durable.
bus.register_with_priority(500, |evt| cache.invalidate(evt.txid));

// Default — ship replication update.
bus.register(|evt| replication.send(evt));

// Lowest priority — bump observability counters last.
bus.register_with_priority(-100, |evt| metrics.record(evt.transition));

// In the transaction manager:
tx_mgr.commit(txid) { bus.notify(&TransactionEvent{ txid, transition: Committed }); }
```

**Properties of interest.**

- **Priority is per-registration, not per-notify.** The producer fires
  one event; the registry orders the handlers.
- **Stable within priority bucket.** Two handlers at priority `500`
  run in the order they were registered.
- **Single dispatch, single allocation-free call.** Even with 5
  handlers, dispatch lands at ~15 ns.

**Runnable example:**
[`examples/pattern_transaction_hooks.rs`](../examples/pattern_transaction_hooks.rs).

---

## Choosing between sync and async

| If the handler does …                                  | Use                                |
|--------------------------------------------------------|------------------------------------|
| Atomic counter updates, in-memory cache invalidations  | `SyncRegistry`                     |
| `println!` / `write!` to a file the OS will buffer     | `SyncRegistry`                     |
| Anything `.await`-able you want to run **concurrently** | `AsyncRegistry::notify`            |
| Anything `.await`-able you want to run **in order**     | `AsyncRegistry::notify_sequential` |
| Network I/O directly                                   | Async **or** offload from sync     |

The performance gap is real: sync `notify` is ~10 ns per handler;
async concurrent dispatch is ~180 ns for 1 handler (dominated by the
boxed-future allocation). If your handlers are *async-fn* but never
actually `.await` real work, prefer `notify_sequential` over `notify`
— it skips the `JoinAll` allocation and lands at ~53 ns / 1 handler.

See [`PERFORMANCE.md`](./PERFORMANCE.md#choosing-a-dispatch-mode) for
measured numbers.

---

## Mistakes to avoid

- **Doing slow work in a sync handler.** Sync handlers run inline on
  the producer's thread. A network round-trip turns the producer's
  `notify` into a network-round-trip-long call. Either move to async
  or spawn from inside the handler onto a worker pool.
- **Registering inside a `notify` callback.** Supported — the new
  registration affects the *next* snapshot — but easy to misread.
  Hoist registrations to a setup phase if you can.
- **Calling `notify` recursively from a handler.** Also supported,
  also a footgun. There is no built-in cycle detection. If you need
  recursion, bound the depth yourself.
- **Capturing megabytes in a closure.** Each registration becomes an
  `Arc<dyn Fn>` heap allocation that owns its captures. Capture an
  `Arc<T>` to share, not a `T` by value, when `T` is large.
- **Re-using a `HandlerId` across registries.** Ids are unique within
  their issuing registry only. The numbers can collide between
  registries; never feed an id from registry A into registry B's
  `unregister`.

---

<sub>registry-io v0.8.0 — Copyright © 2026 James Gober. Apache-2.0 OR MIT.</sub>

# registry-io — API Reference

This document describes every public item in `registry-io 1.1.0`, with
parameter details, return values, and multiple code examples per use case.

For internal architecture notes, see [ARCHITECTURE.md](./ARCHITECTURE.md) (when
present). For performance characteristics, see [PERFORMANCE.md](./PERFORMANCE.md).

---

## Table of contents

- [Module: `registry_io`](#module-registry_io)
  - [`VERSION`](#version)
- [Type: `HandlerId`](#type-handlerid)
  - [`HandlerId::as_u64`](#handleridas_u64)
- [Type: `PanicInfo<'a>`](#type-panicinfoa)
  - [`PanicInfo::handler_id`](#panicinfohandler_id)
  - [`PanicInfo::payload`](#panicinfopayload)
  - [`PanicInfo::message`](#panicinfomessage)
- [Type: `SyncRegistry<E>`](#type-syncregistrye)
  - [`SyncRegistry::new`](#syncregistrynew)
  - [`SyncRegistry::with_capacity`](#syncregistrywith_capacity)
  - [`SyncRegistry::register`](#syncregistryregister)
  - [`SyncRegistry::register_with_priority`](#syncregistryregister_with_priority)
  - [`SyncRegistry::register_guard`](#syncregistryregister_guard)
  - [`SyncRegistry::register_guard_with_priority`](#syncregistryregister_guard_with_priority)
  - [`SyncRegistry::register_once`](#syncregistryregister_once) *(since 1.1.0)*
  - [`SyncRegistry::unregister`](#syncregistryunregister)
  - [`SyncRegistry::unregister_and_wait`](#syncregistryunregister_and_wait) *(since 1.1.0)*
  - [`SyncRegistry::clear`](#syncregistryclear)
  - [`SyncRegistry::clear_and_wait`](#syncregistryclear_and_wait) *(since 1.1.0)*
  - [`SyncRegistry::contains`](#syncregistrycontains)
  - [`SyncRegistry::handler_count`](#syncregistryhandler_count)
  - [`SyncRegistry::is_empty`](#syncregistryis_empty)
  - [`SyncRegistry::on_panic`](#syncregistryon_panic)
  - [`SyncRegistry::clear_panic_callback`](#syncregistryclear_panic_callback)
  - [`SyncRegistry::panic_count`](#syncregistrypanic_count) *(since 1.1.0)*
  - [`SyncRegistry::notify`](#syncregistrynotify)
- [Type: `HandlerGuard<E>`](#type-handlerguarde)
  - [`HandlerGuard::id`](#handlerguardid)
  - [`HandlerGuard::forget`](#handlerguardforget)
  - [`HandlerGuard` drop semantics](#handlerguard-drop-semantics)
- [Type: `AsyncRegistry<E>` *(feature: `async`)*](#type-asyncregistrye-feature-async)
  - [`AsyncRegistry::new`](#asyncregistrynew)
  - [`AsyncRegistry::with_capacity`](#asyncregistrywith_capacity)
  - [`AsyncRegistry::register`](#asyncregistryregister)
  - [`AsyncRegistry::register_with_priority`](#asyncregistryregister_with_priority)
  - [`AsyncRegistry::register_guard` / `register_guard_with_priority`](#asyncregistryregister_guard--register_guard_with_priority)
  - [`AsyncRegistry::unregister`](#asyncregistryunregister)
  - [`AsyncRegistry::clear`](#asyncregistryclear)
  - [`AsyncRegistry::contains`](#asyncregistrycontains)
  - [`AsyncRegistry::handler_count`](#asyncregistryhandler_count)
  - [`AsyncRegistry::is_empty`](#asyncregistryis_empty)
  - [`AsyncRegistry::on_panic`](#asyncregistryon_panic)
  - [`AsyncRegistry::clear_panic_callback`](#asyncregistryclear_panic_callback)
  - [`AsyncRegistry::notify` — concurrent dispatch](#asyncregistrynotify--concurrent-dispatch)
  - [`AsyncRegistry::notify_sequential` — sequential dispatch](#asyncregistrynotify_sequential--sequential-dispatch)
- [Type: `AsyncHandlerGuard<E>` *(feature: `async`)*](#type-asynchandlerguarde-feature-async)
- [Trait implementations](#trait-implementations)

---

## Module: `registry_io`

The crate root re-exports every public item. Import directly:

```rust
use registry_io::{HandlerGuard, HandlerId, PanicInfo, SyncRegistry, VERSION};
```

### `VERSION`

```rust
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
```

Crate version string populated at build time. Useful for diagnostics.

```rust
println!("running registry-io {}", registry_io::VERSION);
```

---

## Type: `HandlerId`

```rust
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct HandlerId(/* opaque */);
```

Opaque identifier returned by every `register*` call. Used to refer back to a
registration later (e.g. for [`SyncRegistry::unregister`]).

`HandlerId` is `Copy + Eq + Hash + Debug + Display`. The numeric representation
is intentionally hidden and may change between releases. **Do not** persist or
compare ids across registries — they are only valid for the registry that
issued them.

### `HandlerId::as_u64`

```rust
pub const fn as_u64(self) -> u64;
```

Returns the raw numeric value backing this id, for diagnostic use only.

**Returns:** the underlying `u64`.

**Example — log the id of a freshly registered handler:**

```rust
use registry_io::SyncRegistry;

let registry: SyncRegistry<()> = SyncRegistry::new();
let id = registry.register(|_| {});
println!("registered handler {}", id.as_u64());
```

**Example — round-trip through a diagnostic JSON payload:**

```rust
use registry_io::SyncRegistry;

let registry: SyncRegistry<()> = SyncRegistry::new();
let id = registry.register(|_| {});
let json = format!(r#"{{"handler_id":{}}}"#, id.as_u64());
assert!(json.contains("handler_id"));
```

---

## Type: `PanicInfo<'a>`

```rust
pub struct PanicInfo<'a> { /* opaque */ }
```

Snapshot passed to an `on_panic` callback when a handler invocation panics
inside [`SyncRegistry::notify`]. Borrowed because the panic payload is owned
by the registry only for the duration of the callback.

### `PanicInfo::handler_id`

```rust
pub fn handler_id(&self) -> HandlerId;
```

The [`HandlerId`] of the handler that panicked.

**Example — react to a specific failing handler:**

```rust
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use registry_io::SyncRegistry;

let registry: SyncRegistry<()> = SyncRegistry::new();
let failing_id = Arc::new(AtomicU64::new(0));
let sink = Arc::clone(&failing_id);
registry.on_panic(move |info| {
    sink.store(info.handler_id().as_u64(), Ordering::SeqCst);
});

let id = registry.register(|_| panic!("nope"));
registry.notify(&());
assert_eq!(failing_id.load(Ordering::SeqCst), id.as_u64());
```

### `PanicInfo::payload`

```rust
pub fn payload(&self) -> &(dyn Any + Send + 'static);
```

The raw panic payload, suitable for downcasting to a user-defined panic
type.

**Example — downcast a custom panic type:**

```rust
use std::sync::{Arc, Mutex};
use registry_io::SyncRegistry;

#[derive(Debug, PartialEq, Eq)]
struct MyErr(i32);

let registry: SyncRegistry<()> = SyncRegistry::new();
let captured: Arc<Mutex<Option<i32>>> = Arc::new(Mutex::new(None));
let sink = Arc::clone(&captured);
registry.on_panic(move |info| {
    if let Some(err) = info.payload().downcast_ref::<MyErr>() {
        *sink.lock().unwrap() = Some(err.0);
    }
});

let _ = registry.register(|_| std::panic::panic_any(MyErr(7)));
registry.notify(&());
assert_eq!(*captured.lock().unwrap(), Some(7));
```

### `PanicInfo::message`

```rust
pub fn message(&self) -> Option<&str>;
```

Best-effort extraction of the panic message. Returns `Some` when the payload
was a `&'static str` (from `panic!("literal")`) or a `String` (from
`panic!("{}", value)`), `None` for custom panic types.

**Example — log every panic message:**

```rust
use std::sync::{Arc, Mutex};
use registry_io::SyncRegistry;

let registry: SyncRegistry<()> = SyncRegistry::new();
let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
let sink = Arc::clone(&log);
registry.on_panic(move |info| {
    sink.lock().unwrap().push(info.message().unwrap_or("<opaque>").into());
});

let _ = registry.register(|_| panic!("alpha"));
let _ = registry.register(|_| panic!("beta {}", 2));
registry.notify(&());
assert_eq!(log.lock().unwrap().as_slice(), &["alpha".to_owned(), "beta 2".to_owned()]);
```

---

## Type: `SyncRegistry<E>`

```rust
pub struct SyncRegistry<E: Send + Sync + 'static> { /* opaque */ }
```

The core synchronous registry. Stores handlers as `Arc<dyn Fn(&E) + Send + Sync + 'static>`
in a list that `notify` borrows in place under a hazard-pointer announcement:
no lock, no reference-count traffic, no allocation. Writers build a new list
and swap it in; see [ARCHITECTURE.md](./ARCHITECTURE.md#storage-model).

`E` is the event type. Handlers receive `&E` (a borrow) so events don't need to
be `Clone`. The bound `E: Send + Sync + 'static` is required for the registry
to itself be `Send + Sync`.

### `SyncRegistry::new`

```rust
pub fn new() -> Self;
```

Construct an empty registry.

**Returns:** a fresh `SyncRegistry<E>` with no registered handlers.

**Example — minimal:**

```rust
use registry_io::SyncRegistry;

let registry: SyncRegistry<u32> = SyncRegistry::new();
assert!(registry.is_empty());
```

**Example — using turbofish:**

```rust
use registry_io::SyncRegistry;

let registry = SyncRegistry::<&'static str>::new();
let _ = registry.register(|s| println!("received {s}"));
registry.notify(&"hello");
```

### `SyncRegistry::with_capacity`

```rust
pub fn with_capacity(capacity: usize) -> Self;
```

Construct an empty registry whose internal `Vec` is pre-allocated to hold
`capacity` handlers. Slow-path optimization for registries with a known
steady-state size.

**Parameters:**
- `capacity: usize` — number of handler slots to pre-allocate.

**Example — pre-allocate when steady-state size is known:**

```rust
use registry_io::SyncRegistry;

let registry: SyncRegistry<u64> = SyncRegistry::with_capacity(64);
for _ in 0..32 {
    let _ = registry.register(|_| {});
}
```

### `SyncRegistry::register`

```rust
pub fn register<F>(&self, handler: F) -> HandlerId
where
    F: Fn(&E) + Send + Sync + 'static;
```

Register a handler at default priority (`0`). Returns a [`HandlerId`] usable
for later unregistration.

**Parameters:**
- `handler: F` — a closure that receives `&E` on every `notify` call. Must be
  `Fn + Send + Sync + 'static`.

**Returns:** an opaque `HandlerId` unique within this registry.

**Example — simple side-effect handler:**

```rust
use registry_io::SyncRegistry;

let registry: SyncRegistry<&str> = SyncRegistry::new();
let _id = registry.register(|s| println!("event: {s}"));
registry.notify(&"first");
```

**Example — accumulator with shared state:**

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use registry_io::SyncRegistry;

let registry: SyncRegistry<u64> = SyncRegistry::new();
let total = Arc::new(AtomicU64::new(0));
let sink = Arc::clone(&total);
let _ = registry.register(move |v| {
    let _ = sink.fetch_add(*v, Ordering::Relaxed);
});

registry.notify(&5);
registry.notify(&7);
assert_eq!(total.load(Ordering::Relaxed), 12);
```

### `SyncRegistry::register_with_priority`

```rust
pub fn register_with_priority<F>(&self, priority: i32, handler: F) -> HandlerId
where
    F: Fn(&E) + Send + Sync + 'static;
```

Register a handler with an explicit priority. On `notify`, handlers fire in
**descending priority order**; ties are broken in registration order.

**Parameters:**
- `priority: i32` — higher values fire first. `0` is the default used by
  `register`.
- `handler: F` — the handler closure (same bounds as `register`).

**Returns:** an opaque `HandlerId`.

**Example — high-priority logging hook fires before defaults:**

```rust
use std::sync::{Arc, Mutex};
use registry_io::SyncRegistry;

let registry: SyncRegistry<()> = SyncRegistry::new();
let trace = Arc::new(Mutex::new(Vec::<&'static str>::new()));

let t = Arc::clone(&trace);
let _ = registry.register_with_priority(100, move |_| t.lock().unwrap().push("audit"));
let t = Arc::clone(&trace);
let _ = registry.register(move |_| t.lock().unwrap().push("business"));

registry.notify(&());
assert_eq!(trace.lock().unwrap().as_slice(), &["audit", "business"]);
```

**Example — negative priority for cleanup handlers:**

```rust
use std::sync::{Arc, Mutex};
use registry_io::SyncRegistry;

let registry: SyncRegistry<()> = SyncRegistry::new();
let trace = Arc::new(Mutex::new(Vec::<&'static str>::new()));

let t = Arc::clone(&trace);
let _ = registry.register(move |_| t.lock().unwrap().push("work"));
let t = Arc::clone(&trace);
let _ = registry.register_with_priority(-50, move |_| t.lock().unwrap().push("cleanup"));

registry.notify(&());
assert_eq!(trace.lock().unwrap().as_slice(), &["work", "cleanup"]);
```

### `SyncRegistry::register_guard`

```rust
pub fn register_guard<F>(self: &Arc<Self>, handler: F) -> HandlerGuard<E>
where
    F: Fn(&E) + Send + Sync + 'static;
```

Register and return a RAII [`HandlerGuard`] that automatically unregisters
when dropped. Requires the registry to be wrapped in [`Arc`] so the guard
can hold a [`Weak`] reference.

**Parameters:**
- `handler: F` — handler closure.

**Returns:** a `HandlerGuard<E>`. While the guard is alive the handler stays
registered; dropping the guard removes it.

**Example — handler tied to a scope:**

```rust
use std::sync::Arc;
use registry_io::SyncRegistry;

let registry = Arc::new(SyncRegistry::<u32>::new());
{
    let _guard = registry.register_guard(|n| println!("scoped: {n}"));
    registry.notify(&1);
}
assert!(registry.is_empty());
```

**Example — guard returned from a builder function:**

```rust
use std::sync::Arc;
use registry_io::{HandlerGuard, SyncRegistry};

fn install_logger(registry: &Arc<SyncRegistry<String>>) -> HandlerGuard<String> {
    registry.register_guard(|msg| println!("[log] {msg}"))
}

let registry = Arc::new(SyncRegistry::<String>::new());
let _log_guard = install_logger(&registry);
registry.notify(&"hello".to_owned());
```

### `SyncRegistry::register_guard_with_priority`

```rust
pub fn register_guard_with_priority<F>(
    self: &Arc<Self>,
    priority: i32,
    handler: F,
) -> HandlerGuard<E>
where
    F: Fn(&E) + Send + Sync + 'static;
```

Combines [`register_with_priority`] and [`register_guard`].

**Example — auditing hook with priority 100:**

```rust
use std::sync::Arc;
use registry_io::SyncRegistry;

let registry = Arc::new(SyncRegistry::<&'static str>::new());
let _audit_guard = registry.register_guard_with_priority(100, |evt| {
    println!("AUDIT: {evt}");
});
registry.notify(&"login");
```

### `SyncRegistry::register_once`

*Since 1.1.0.*

```rust
pub fn register_once<F>(self: &Arc<Self>, handler: F) -> HandlerId
where
    F: FnOnce(&E) + Send + 'static;
```

Register a handler that runs at most once, then removes itself. The handler
may be `FnOnce`, so it can move captured state out. If several threads call
`notify` concurrently, exactly one of them runs it; the others skip it. It is
unregistered right after it runs, and also when it panics, so
[`contains`](#syncregistrycontains) reports it gone from then on. Registers
at the default priority (`0`).

The registry must be in an [`Arc`] so the handler can remove itself. The
handler holds only a `Weak` reference, so it does not keep the registry
alive.

**Parameters:**
- `handler: F` — a closure that receives `&E` on the first `notify` call that
  reaches it. Must be `FnOnce + Send + 'static` (no `Sync` needed).

**Returns:** the `HandlerId` of the one-shot registration. It can be passed to
`unregister` to cancel the handler before it runs.

**Example — run once, then gone:**

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use registry_io::SyncRegistry;

let registry = Arc::new(SyncRegistry::<u32>::new());
let seen = Arc::new(AtomicU32::new(0));
let s = Arc::clone(&seen);
let id = registry.register_once(move |n| {
    s.fetch_add(*n, Ordering::SeqCst);
});

registry.notify(&5);
registry.notify(&7);
assert_eq!(seen.load(Ordering::SeqCst), 5);
assert!(!registry.contains(id));
```

**Example — move state out of the closure:**

```rust
use std::sync::{Arc, mpsc};
use registry_io::SyncRegistry;

let registry = Arc::new(SyncRegistry::<String>::new());
let (tx, rx) = mpsc::channel::<String>();

// `tx` is moved into the handler and consumed by it.
let _ = registry.register_once(move |msg| {
    tx.send(msg.clone()).unwrap();
    drop(tx);
});

registry.notify(&"ready".to_owned());
assert_eq!(rx.recv().unwrap(), "ready");
assert!(rx.recv().is_err()); // sender dropped after the single run
```

### `SyncRegistry::unregister`

```rust
pub fn unregister(&self, id: HandlerId) -> bool;
```

Remove the handler identified by `id`.

**Parameters:**
- `id: HandlerId` — id returned from a previous `register*` call.

**Returns:** `true` if a handler was found and removed, `false` otherwise.
An unknown id swaps nothing and allocates nothing.

`notify` calls that start after `unregister` returns do not run the handler.
A `notify` already in progress on another thread may still be running it;
use [`unregister_and_wait`](#syncregistryunregister_and_wait) when that
matters. Calling `unregister` from inside a handler (including the handler
being removed) is supported and does not block.

**Example — remove a single handler:**

```rust
use registry_io::SyncRegistry;

let registry: SyncRegistry<()> = SyncRegistry::new();
let id = registry.register(|_| {});
assert!(registry.unregister(id));
assert!(!registry.unregister(id)); // already gone
```

**Example — unregister after a condition is met:**

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use registry_io::SyncRegistry;

let registry: SyncRegistry<u32> = SyncRegistry::new();
let count = Arc::new(AtomicUsize::new(0));
let sink = Arc::clone(&count);
let id = registry.register(move |_| {
    let _ = sink.fetch_add(1, Ordering::Relaxed);
});

for _ in 0..5 {
    registry.notify(&1);
}
if count.load(Ordering::Relaxed) >= 5 {
    let _ = registry.unregister(id);
}
```

### `SyncRegistry::unregister_and_wait`

*Since 1.1.0.*

```rust
pub fn unregister_and_wait(&self, id: HandlerId) -> bool;
```

Unregister a handler and wait until it has stopped running on every other
thread.

[`unregister`](#syncregistryunregister) guarantees that `notify` calls
*starting* after it returns will not run the handler, but a `notify` already
in progress on another thread may still be running it. This method closes
that window: when it returns `true`, the handler is not running anywhere
except, possibly, further up the calling thread's own stack (when called from
inside a handler, it does not wait for itself), and it will never run again.
Use it before tearing down anything the handler touches.

**Parameters:**
- `id: HandlerId` — id returned from a previous `register*` call.

**Returns:** `true` once the handler is removed and no other thread is running
it; `false` immediately, without waiting, if no handler with `id` was
registered.

**Blocking:** blocks the calling thread until in-flight dispatches on other
threads finish, so a handler that never returns makes this never return. It
spins briefly, then yields, then sleeps in steps of up to one millisecond.
Do not call it from async code; use `unregister` there. The wait is
conservative: if other threads register or unregister while it waits, it may
also wait for dispatches that started on those newer handler lists.

**Example — close a resource only after the handler is done with it:**

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use registry_io::SyncRegistry;

let registry: SyncRegistry<()> = SyncRegistry::new();
let resource_open = Arc::new(AtomicBool::new(true));
let r = Arc::clone(&resource_open);
let id = registry.register(move |_| assert!(r.load(Ordering::SeqCst)));

assert!(registry.unregister_and_wait(id));
// No thread can be inside the handler now; safe to close.
resource_open.store(false, Ordering::SeqCst);
registry.notify(&());
```

**Example — with another thread dispatching:**

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use registry_io::SyncRegistry;

let registry = Arc::new(SyncRegistry::<u64>::new());
let alive = Arc::new(AtomicBool::new(true));
let a = Arc::clone(&alive);
let id = registry.register(move |_| assert!(a.load(Ordering::SeqCst)));

let stop = Arc::new(AtomicBool::new(false));
let notifier = {
    let (registry, stop) = (Arc::clone(&registry), Arc::clone(&stop));
    thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            registry.notify(&1);
        }
    })
};

assert!(registry.unregister_and_wait(id));
alive.store(false, Ordering::SeqCst); // the handler can no longer observe this
stop.store(true, Ordering::Relaxed);
notifier.join().unwrap();
```

### `SyncRegistry::clear`

```rust
pub fn clear(&self);
```

Remove every registered handler. In-flight `notify` calls that loaded a
snapshot before `clear` finishes will still iterate over their snapshot to
completion.

**Example — clear after a test:**

```rust
use registry_io::SyncRegistry;

let registry: SyncRegistry<()> = SyncRegistry::new();
for _ in 0..10 {
    let _ = registry.register(|_| {});
}
registry.clear();
assert_eq!(registry.handler_count(), 0);
```

Clearing an already-empty registry swaps nothing and allocates nothing.

### `SyncRegistry::clear_and_wait`

*Since 1.1.0.*

```rust
pub fn clear_and_wait(&self);
```

Remove every handler and wait until none of them is still running on another
thread. The [`clear`](#syncregistryclear) counterpart of
[`unregister_and_wait`](#syncregistryunregister_and_wait), with the same
guarantee and the same blocking behavior: do not call it from async code. On
an empty registry it returns immediately.

**Example:**

```rust
use registry_io::SyncRegistry;

let registry: SyncRegistry<u8> = SyncRegistry::new();
let _ = registry.register(|_| {});
registry.clear_and_wait();
assert!(registry.is_empty());
```

### `SyncRegistry::contains`

```rust
pub fn contains(&self, id: HandlerId) -> bool;
```

Returns `true` if a handler with `id` is currently registered.

**Example:**

```rust
use registry_io::SyncRegistry;

let registry: SyncRegistry<()> = SyncRegistry::new();
let id = registry.register(|_| {});
assert!(registry.contains(id));
assert!(registry.unregister(id));
assert!(!registry.contains(id));
```

### `SyncRegistry::handler_count`

```rust
pub fn handler_count(&self) -> usize;
```

Snapshot the current number of handlers. `O(1)`.

**Example:**

```rust
use registry_io::SyncRegistry;

let registry: SyncRegistry<()> = SyncRegistry::new();
assert_eq!(registry.handler_count(), 0);
let _ = registry.register(|_| {});
let _ = registry.register(|_| {});
assert_eq!(registry.handler_count(), 2);
```

### `SyncRegistry::is_empty`

```rust
pub fn is_empty(&self) -> bool;
```

Equivalent to `self.handler_count() == 0`.

```rust
use registry_io::SyncRegistry;

let registry: SyncRegistry<()> = SyncRegistry::new();
assert!(registry.is_empty());
let _ = registry.register(|_| {});
assert!(!registry.is_empty());
```

### `SyncRegistry::on_panic`

```rust
pub fn on_panic<F>(&self, callback: F)
where
    F: Fn(&PanicInfo<'_>) + Send + Sync + 'static;
```

Install a callback invoked when a handler panics during `notify`. The previous
callback (if any) is replaced. Without an installed callback, panics are
**silently absorbed** — siblings still fire, but the panic is not observable.

A panic inside the callback itself is caught and discarded.

**Parameters:**
- `callback: F` — closure invoked once per panicking handler, on the thread
  that invoked `notify`.

**Example — count panics:**

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use registry_io::SyncRegistry;

let registry: SyncRegistry<()> = SyncRegistry::new();
let count = Arc::new(AtomicUsize::new(0));
let sink = Arc::clone(&count);
registry.on_panic(move |_| {
    let _ = sink.fetch_add(1, Ordering::Relaxed);
});

let _ = registry.register(|_| panic!("oops"));
let _ = registry.register(|_| {});
registry.notify(&());
assert_eq!(count.load(Ordering::Relaxed), 1);
```

**Example — log via your existing logger:**

```rust
use registry_io::SyncRegistry;

let registry: SyncRegistry<()> = SyncRegistry::new();
registry.on_panic(|info| {
    eprintln!(
        "handler {} panicked: {}",
        info.handler_id(),
        info.message().unwrap_or("<opaque>")
    );
});
```

### `SyncRegistry::clear_panic_callback`

```rust
pub fn clear_panic_callback(&self);
```

Remove any previously installed `on_panic` callback. Subsequent handler
panics during `notify` become silent again.

```rust
use registry_io::SyncRegistry;

let registry: SyncRegistry<()> = SyncRegistry::new();
registry.on_panic(|_| {});
registry.clear_panic_callback();
```

### `SyncRegistry::panic_count`

*Since 1.1.0.*

```rust
pub fn panic_count(&self) -> u64;
```

Number of handler panics caught by `notify` since the registry was created.
Counted whether or not an [`on_panic`](#syncregistryon_panic) callback is
installed, so it can back a health check or metric without one. Updated only
on the (cold) panic path, so it costs nothing while handlers behave.

**Returns:** the running total as a `u64`.

**Example — count without a callback:**

```rust
use registry_io::SyncRegistry;

std::panic::set_hook(Box::new(|_| {})); // keep the test output quiet
let registry: SyncRegistry<()> = SyncRegistry::new();
let _ = registry.register(|_| panic!("boom"));
registry.notify(&());
assert_eq!(registry.panic_count(), 1);
```

**Example — health check:**

```rust
use registry_io::SyncRegistry;

fn healthy<E: Send + Sync + 'static>(registry: &SyncRegistry<E>, budget: u64) -> bool {
    registry.panic_count() <= budget
}

let registry: SyncRegistry<u32> = SyncRegistry::new();
let _ = registry.register(|_| {});
registry.notify(&1);
assert!(healthy(&registry, 0));
```

### `SyncRegistry::notify`

```rust
pub fn notify(&self, event: &E);
```

Dispatch `event` to every registered handler. The hot path is **lock-free**
and **allocation-free** in the no-panic case. Handlers run inline on the
calling thread, in **priority order** (high → low), ties broken in
registration order.

Each handler invocation is wrapped in [`catch_unwind`] so that a panic in
one handler does **not** propagate to siblings or to the caller. If an
[`on_panic`](#syncregistryon_panic) callback is installed, it is invoked once
per panicking handler. Every caught panic also increments
[`panic_count`](#syncregistrypanic_count).

**Parameters:**
- `event: &E` — borrowed event. Handlers receive the same reference.

**Returns:** unit. Errors and panics are absorbed internally.

**Example — broadcasting a typed event:**

```rust
use registry_io::SyncRegistry;

#[derive(Debug)]
struct ConfigReloaded { keys_changed: usize }

let registry: SyncRegistry<ConfigReloaded> = SyncRegistry::new();
let _ = registry.register(|evt| {
    println!("config changed: {} keys", evt.keys_changed);
});

registry.notify(&ConfigReloaded { keys_changed: 4 });
```

**Example — fan-out from a hot loop:**

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use registry_io::SyncRegistry;

let registry: SyncRegistry<u32> = SyncRegistry::new();
let counter = Arc::new(AtomicU64::new(0));
let sink = Arc::clone(&counter);
let _ = registry.register(move |v| {
    let _ = sink.fetch_add(u64::from(*v), Ordering::Relaxed);
});

for i in 0..1_000 {
    registry.notify(&i);
}
assert_eq!(counter.load(Ordering::Relaxed), (0..1_000).sum::<u32>() as u64);
```

---

## Type: `HandlerGuard<E>`

```rust
#[must_use = "..."]
pub struct HandlerGuard<E: Send + Sync + 'static> { /* opaque */ }
```

RAII handle for a registered handler. Drop the guard to unregister. The
guard holds a [`Weak`] reference to the registry, so it doesn't keep the
registry alive on its own.

### `HandlerGuard::id`

```rust
pub fn id(&self) -> HandlerId;
```

Return the underlying [`HandlerId`].

```rust
use std::sync::Arc;
use registry_io::SyncRegistry;

let registry = Arc::new(SyncRegistry::<()>::new());
let guard = registry.register_guard(|_| {});
let id = guard.id();
drop(guard);
assert!(!registry.contains(id));
```

### `HandlerGuard::forget`

```rust
pub fn forget(self);
```

Consume the guard **without** unregistering the handler. The handler stays
alive until explicit `unregister` or registry drop.

```rust
use std::sync::Arc;
use registry_io::SyncRegistry;

let registry = Arc::new(SyncRegistry::<()>::new());
let guard = registry.register_guard(|_| {});
let id = guard.id();
guard.forget();
assert!(registry.contains(id));
assert!(registry.unregister(id));
```

### `HandlerGuard` drop semantics

Dropping a guard:

1. Upgrades the held `Weak<SyncRegistry<E>>`.
2. If the registry is still alive, calls `unregister(id)`.
3. If the registry has been dropped already, the drop is a no-op.

Guards are `!Clone` — ownership of a registration is unique.

---

## Type: `AsyncRegistry<E>` *(feature: `async`)*

```rust
pub struct AsyncRegistry<E: Send + Sync + 'static> { /* opaque */ }
```

The asynchronous counterpart to [`SyncRegistry`](#type-syncregistrye). Same
lock-free, hazard-protected storage; handlers return a `'static` future that
the registry polls in place, either concurrently or sequentially.

Each handler call is isolated: a panic while the handler builds its future
(the synchronous part of the closure) or while the future is polled is
caught, does not affect sibling handlers or the awaiting caller, and is
reported to `on_panic`. (Before 1.1.0 a panic while building the future
escaped `notify`.)

The four methods added to `SyncRegistry` in 1.1.0 (`unregister_and_wait`,
`clear_and_wait`, `register_once`, `panic_count`) have no async counterpart.

Path: `registry_io::r#async::AsyncRegistry` (re-exported at the crate root as
`registry_io::AsyncRegistry` when the `async` feature is enabled).

### `AsyncRegistry::new`

```rust
pub fn new() -> Self;
```

Construct an empty async registry.

```rust
# #[cfg(feature = "async")] {
use registry_io::r#async::AsyncRegistry;
let registry: AsyncRegistry<u32> = AsyncRegistry::new();
assert!(registry.is_empty());
# }
```

### `AsyncRegistry::with_capacity`

```rust
pub fn with_capacity(capacity: usize) -> Self;
```

Pre-allocate the internal `Vec` for `capacity` handlers.

```rust
# #[cfg(feature = "async")] {
use registry_io::r#async::AsyncRegistry;
let registry: AsyncRegistry<u64> = AsyncRegistry::with_capacity(64);
assert!(registry.is_empty());
# }
```

### `AsyncRegistry::register`

```rust
pub fn register<F, Fut>(&self, handler: F) -> HandlerId
where
    F: Fn(&E) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static;
```

Register an async handler at default priority `0`. The handler is a closure
that receives `&E` and returns a `'static` future. Clone what you need out
of `&E` before the inner `async move`:

```rust
# #[cfg(feature = "async")] async fn _doc() {
use registry_io::r#async::AsyncRegistry;
let registry: AsyncRegistry<String> = AsyncRegistry::new();
let _ = registry.register(|event| {
    let owned = event.clone();
    async move {
        let _ = owned.len();
    }
});
# }
```

### `AsyncRegistry::register_with_priority`

```rust
pub fn register_with_priority<F, Fut>(&self, priority: i32, handler: F) -> HandlerId
where
    F: Fn(&E) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static;
```

Same ordering rules as [`SyncRegistry::register_with_priority`]. In
concurrent dispatch priority controls the order futures are *spawned* into
the join, not the order they *resolve* — for execution-order guarantees,
use `notify_sequential`.

```rust
# #[cfg(feature = "async")] async fn _doc() {
use registry_io::r#async::AsyncRegistry;
let registry: AsyncRegistry<()> = AsyncRegistry::new();
let _ = registry.register_with_priority(100, |_| async move {});
let _ = registry.register(|_| async move {});
let _ = registry.register_with_priority(-10, |_| async move {});
assert_eq!(registry.handler_count(), 3);
# }
```

### `AsyncRegistry::register_guard` / `register_guard_with_priority`

```rust
pub fn register_guard<F, Fut>(self: &Arc<Self>, handler: F) -> AsyncHandlerGuard<E>;
pub fn register_guard_with_priority<F, Fut>(self: &Arc<Self>, priority: i32, handler: F) -> AsyncHandlerGuard<E>;
```

RAII variants. Drop the returned [`AsyncHandlerGuard`] to unregister; call
`forget()` to detach.

```rust
# #[cfg(feature = "async")] {
use std::sync::Arc;
use registry_io::r#async::AsyncRegistry;
let registry = Arc::new(AsyncRegistry::<u32>::new());
{
    let _guard = registry.register_guard(|_| async move {});
    assert_eq!(registry.handler_count(), 1);
}
assert_eq!(registry.handler_count(), 0);
# }
```

### `AsyncRegistry::unregister`

```rust
pub fn unregister(&self, id: HandlerId) -> bool;
```

Remove the async handler identified by `id`. Identical semantics to
[`SyncRegistry::unregister`].

**Parameters:**
- `id: HandlerId` — id returned from a previous `register*` call on this
  registry.

**Returns:** `true` if a handler was found and removed, `false` otherwise.

**Example — remove a registered handler:**

```rust
# #[cfg(feature = "async")] {
use registry_io::r#async::AsyncRegistry;

let registry: AsyncRegistry<()> = AsyncRegistry::new();
let id = registry.register(|_| async move {});
assert!(registry.unregister(id));
assert!(!registry.unregister(id));
# }
```

**Example — conditional removal based on age:**

```rust
# #[cfg(feature = "async")] async fn _doc() {
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use registry_io::r#async::AsyncRegistry;

let registry: AsyncRegistry<u32> = AsyncRegistry::new();
let count = Arc::new(AtomicUsize::new(0));
let sink = Arc::clone(&count);
let id = registry.register(move |_| {
    let sink = Arc::clone(&sink);
    async move {
        let _ = sink.fetch_add(1, Ordering::Relaxed);
    }
});

for _ in 0..3 { registry.notify(&1).await; }
if count.load(Ordering::Relaxed) >= 3 {
    let _ = registry.unregister(id);
}
# }
```

### `AsyncRegistry::clear`

```rust
pub fn clear(&self);
```

Remove every registered async handler. In-flight `notify*` calls that
already loaded their snapshot before `clear` finishes still run their
snapshot to completion.

**Example — reset between phases:**

```rust
# #[cfg(feature = "async")] {
use registry_io::r#async::AsyncRegistry;

let registry: AsyncRegistry<()> = AsyncRegistry::new();
for _ in 0..10 {
    let _ = registry.register(|_| async move {});
}
registry.clear();
assert_eq!(registry.handler_count(), 0);
# }
```

### `AsyncRegistry::contains`

```rust
pub fn contains(&self, id: HandlerId) -> bool;
```

Returns `true` if a handler with `id` is currently registered.

**Example:**

```rust
# #[cfg(feature = "async")] {
use registry_io::r#async::AsyncRegistry;

let registry: AsyncRegistry<()> = AsyncRegistry::new();
let id = registry.register(|_| async move {});
assert!(registry.contains(id));
assert!(registry.unregister(id));
assert!(!registry.contains(id));
# }
```

### `AsyncRegistry::handler_count`

```rust
pub fn handler_count(&self) -> usize;
```

Snapshot the current async handler count. `O(1)`.

```rust
# #[cfg(feature = "async")] {
use registry_io::r#async::AsyncRegistry;

let registry: AsyncRegistry<()> = AsyncRegistry::new();
assert_eq!(registry.handler_count(), 0);
let _ = registry.register(|_| async move {});
let _ = registry.register(|_| async move {});
assert_eq!(registry.handler_count(), 2);
# }
```

### `AsyncRegistry::is_empty`

```rust
pub fn is_empty(&self) -> bool;
```

Equivalent to `self.handler_count() == 0`.

```rust
# #[cfg(feature = "async")] {
use registry_io::r#async::AsyncRegistry;

let registry: AsyncRegistry<()> = AsyncRegistry::new();
assert!(registry.is_empty());
let _ = registry.register(|_| async move {});
assert!(!registry.is_empty());
# }
```

### `AsyncRegistry::on_panic`

```rust
pub fn on_panic<F>(&self, callback: F)
where
    F: Fn(&PanicInfo<'_>) + Send + Sync + 'static;
```

Install a callback invoked when an async handler's future panics during
`notify` or `notify_sequential`. Replaces any previous callback. Second-
order panics inside the callback itself are caught and discarded.

**Parameters:**
- `callback: F` — closure receiving a [`PanicInfo`] reference per
  panicking handler. Runs on whichever thread the runtime polled the
  failing future on.

**Example — counting panics:**

```rust
# #[cfg(feature = "async")] async fn _doc() {
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use registry_io::r#async::AsyncRegistry;

let registry: AsyncRegistry<()> = AsyncRegistry::new();
let count = Arc::new(AtomicUsize::new(0));
let sink = Arc::clone(&count);
registry.on_panic(move |_| {
    let _ = sink.fetch_add(1, Ordering::Relaxed);
});

let _ = registry.register(|_| async move { panic!("oops") });
let _ = registry.register(|_| async move {});
registry.notify(&()).await;
assert_eq!(count.load(Ordering::Relaxed), 1);
# }
```

**Example — log via existing logger:**

```rust
# #[cfg(feature = "async")] {
use registry_io::r#async::AsyncRegistry;

let registry: AsyncRegistry<()> = AsyncRegistry::new();
registry.on_panic(|info| {
    eprintln!(
        "async handler {} panicked: {}",
        info.handler_id(),
        info.message().unwrap_or("<opaque>")
    );
});
# }
```

### `AsyncRegistry::clear_panic_callback`

```rust
pub fn clear_panic_callback(&self);
```

Remove any previously installed panic callback. Subsequent panics during
`notify*` become silent again.

```rust
# #[cfg(feature = "async")] {
use registry_io::r#async::AsyncRegistry;

let registry: AsyncRegistry<()> = AsyncRegistry::new();
registry.on_panic(|_| {});
registry.clear_panic_callback();
# }
```

### `AsyncRegistry::notify` — concurrent dispatch

```rust
pub async fn notify(&self, event: &E);
```

Builds one future per handler and polls them all in place until every one has
finished. Total wall-clock equals the **slowest** handler when each handler
does real `.await` work. Panics are reported to `on_panic` after all handlers
have finished, in priority order.

Allocations per call: one per handler (its boxed future) plus one for the
join state when there is more than one handler; none for an empty registry.

```rust
# #[cfg(feature = "async")] async fn _doc() {
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use registry_io::r#async::AsyncRegistry;

let registry: AsyncRegistry<u32> = AsyncRegistry::new();
let total = Arc::new(AtomicU32::new(0));
for _ in 0..4 {
    let sink = Arc::clone(&total);
    let _ = registry.register(move |value| {
        let sink = Arc::clone(&sink);
        let v = *value;
        async move {
            sink.fetch_add(v, Ordering::Relaxed);
        }
    });
}
registry.notify(&10).await;
assert_eq!(total.load(Ordering::Relaxed), 40);
# }
```

### `AsyncRegistry::notify_sequential` — sequential dispatch

```rust
pub async fn notify_sequential(&self, event: &E);
```

Awaits each handler's future to completion before starting the next. Use
this when handlers must observe a happens-before relation with one another.

```rust
# #[cfg(feature = "async")] async fn _doc() {
use std::sync::{Arc, Mutex};
use registry_io::r#async::AsyncRegistry;

let registry: AsyncRegistry<()> = AsyncRegistry::new();
let log: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));

let l = Arc::clone(&log);
let _ = registry.register_with_priority(10, move |_| {
    let l = Arc::clone(&l);
    async move { l.lock().unwrap().push("first"); }
});
let l = Arc::clone(&log);
let _ = registry.register(move |_| {
    let l = Arc::clone(&l);
    async move { l.lock().unwrap().push("second"); }
});

registry.notify_sequential(&()).await;
assert_eq!(log.lock().unwrap().as_slice(), &["first", "second"]);
# }
```

---

## Type: `AsyncHandlerGuard<E>` *(feature: `async`)*

```rust
#[must_use = "..."]
pub struct AsyncHandlerGuard<E: Send + Sync + 'static> { /* opaque */ }
```

RAII handle for an async registration. Drop to unregister; `forget()` to
detach. Same drop-order safety as [`HandlerGuard`](#type-handlerguarde): the
guard holds a [`std::sync::Weak`] to the registry, so registry-before-guard
drop is a no-op.

### `AsyncHandlerGuard::id`

```rust
pub fn id(&self) -> HandlerId;
```

### `AsyncHandlerGuard::forget`

```rust
pub fn forget(self);
```

---

## Trait implementations

| Type                    | `Send` | `Sync` | `Debug` | `Default` | `Clone` |
|-------------------------|:------:|:------:|:-------:|:---------:|:-------:|
| `HandlerId`             | ✓      | ✓      | ✓       | ✗         | ✓ (`Copy`) |
| `PanicInfo<'_>`         | (n/a)  | (n/a)  | ✓       | ✗         | ✗       |
| `SyncRegistry<E>`       | ✓      | ✓      | ✓       | ✓         | ✗       |
| `HandlerGuard<E>`       | ✓      | ✓      | ✓       | ✗         | ✗       |
| `AsyncRegistry<E>`      | ✓      | ✓      | ✓       | ✓         | ✗       |
| `AsyncHandlerGuard<E>`  | ✓      | ✓      | ✓       | ✗         | ✗       |

All types upholding `Send + Sync` do so for any `E: Send + Sync + 'static`.

---

## Feature flags

| Flag       | Default | Description                                              |
|------------|:-------:|----------------------------------------------------------|
| `std`      | ✓       | Standard library. Required for sync / async registries. |
| `sync`     | ✓       | Enables `SyncRegistry`. Implies `std`.                  |
| `async`    | ✗       | Enables `AsyncRegistry` and `AsyncHandlerGuard`. Implies `std`. |
| `hybrid`   | ✗       | Activates both `sync` and `async`.                      |
| `metrics`  | ✗       | Reserved for built-in metrics integration.              |

---

<sub>registry-io v1.1.0 — Copyright © 2026 James Gober. Apache-2.0 OR MIT.</sub>

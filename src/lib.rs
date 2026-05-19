//! # registry-io
//!
//! High-performance event and callback registry primitive for Rust.
//!
//! `registry-io` provides a focused alternative to channel-based notification
//! when several components need to react to the same in-process event with
//! the lowest possible dispatch overhead. The hot path is **lock-free**,
//! **allocation-free**, and **panic-isolating**.
//!
//! # Design philosophy
//!
//! - **Sync-first.** The default registry runs handlers inline on the
//!   calling thread, with sub-microsecond dispatch overhead.
//! - **Lock-free reads.** Multiple threads can fire [`SyncRegistry::notify`]
//!   concurrently without serialization.
//! - **Zero allocation on the hot path.** `notify` walks an
//!   [`arc_swap::ArcSwap`] snapshot of `Arc<dyn Fn>` pointers and dispatches
//!   each one — no allocations along the no-panic path.
//! - **Panic isolation.** A panic in one handler is caught and does **not**
//!   stop sibling handlers or propagate to the caller.
//! - **Priority ordering.** Handlers may be registered with a priority value;
//!   higher priorities fire first, ties broken in registration order.
//! - **RAII unregistration.** [`HandlerGuard`] cleans up automatically.
//!
//! # Quick start
//!
//! ```
//! use std::sync::Arc;
//! use std::sync::atomic::{AtomicU32, Ordering};
//! use registry_io::SyncRegistry;
//!
//! let registry: SyncRegistry<u32> = SyncRegistry::new();
//! let counter = Arc::new(AtomicU32::new(0));
//!
//! let sink = Arc::clone(&counter);
//! let id = registry.register(move |value| {
//!     sink.fetch_add(*value, Ordering::Relaxed);
//! });
//!
//! registry.notify(&5);
//! registry.notify(&7);
//! assert_eq!(counter.load(Ordering::Relaxed), 12);
//!
//! assert!(registry.unregister(id));
//! ```
//!
//! # Feature flags
//!
//! - `std` (default) — enables the standard library. Required for sync and
//!   async registries.
//! - `sync` (default) — exposes [`SyncRegistry`]. Implies `std`.
//! - `async` — reserved for a future release; exposes `AsyncRegistry` for
//!   `async fn` handlers. Implies `std`.
//! - `hybrid` — activates both `sync` and `async`.
//!
//! # Out of scope
//!
//! `registry-io` is a local, in-process primitive. It is **not** a pub/sub
//! broker, **not** a message bus, and **not** a replacement for channels
//! when you need cross-process or cross-network delivery with backpressure.
//! See the project README for a list of when **not** to use it.
//!
//! # License
//!
//! Dual-licensed under Apache-2.0 OR MIT.

#![doc(html_root_url = "https://docs.rs/registry-io/0.9.0")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]
#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(unused_must_use)]
#![deny(unused_results)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::todo)]
#![deny(clippy::unimplemented)]
#![deny(clippy::print_stdout)]
#![deny(clippy::print_stderr)]
#![deny(clippy::dbg_macro)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![deny(clippy::missing_safety_doc)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

mod handler_id;

#[cfg(any(feature = "sync", feature = "async"))]
mod panic;

#[cfg(feature = "async")]
mod future_ext;

#[cfg(feature = "sync")]
pub mod sync;

#[cfg(feature = "async")]
#[path = "async_registry/mod.rs"]
pub mod r#async;

pub use handler_id::HandlerId;

#[cfg(any(feature = "sync", feature = "async"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "sync", feature = "async"))))]
pub use panic::PanicInfo;

#[cfg(feature = "sync")]
#[cfg_attr(docsrs, doc(cfg(feature = "sync")))]
pub use sync::{HandlerGuard, SyncRegistry};

#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub use r#async::{AsyncHandlerGuard, AsyncRegistry};

/// Crate version string, populated by Cargo at build time.
///
/// # Examples
///
/// ```
/// // VERSION is the canonical place to read the running crate version,
/// // for diagnostic logging or version-gated behavior.
/// assert!(!registry_io::VERSION.is_empty());
/// assert!(registry_io::VERSION.starts_with("0.") || registry_io::VERSION.starts_with("1."));
/// ```
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

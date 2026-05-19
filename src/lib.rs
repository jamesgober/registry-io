//! # registry-io
//!
//! HIGH-PERFORMANCE EVENT REGISTRY PRIMITIVE
//!
//! Sync-first event/callback registry with optional async support. Lock-free reads, zero-allocation hot path, sub-50ns notify target. Foundation primitive for portfolio crates needing fast in-process notification.
//!
//! # Design philosophy
//!
//! $name is a focused primitive for fast in-process event notification. It is
//! deliberately **NOT** a distributed messaging system, NOT a pub/sub broker, and
//! NOT a replacement for channels when you need cross-thread async delivery with
//! backpressure. It IS the right tool when you need:
//!
//! - Multiple handlers responding to the same event
//! - Sub-microsecond dispatch overhead
//! - Lock-free reads under contention
//! - Zero-allocation hot path
//! - Optional async support without forcing async on sync users
//!
//! # Status
//!
//! Early scaffolding. Public API not yet defined. See [the repository](https://github.com/jamesgober/registry-io)
//! and .dev/ROADMAP.md for the milestone plan.
//!
//! # License
//!
//! Dual-licensed under Apache-2.0 OR MIT.

#![doc(html_root_url = "https://docs.rs/registry-io")]
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

/// Crate version string, populated by Cargo at build time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
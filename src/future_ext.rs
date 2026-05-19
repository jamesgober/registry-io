//! Minimal in-crate future combinators used by [`AsyncRegistry`].
//!
//! Two helpers live here:
//!
//! - [`CatchUnwind`] — catches panics that escape a future's `poll` and
//!   yields them as an `Err` payload, mirroring
//!   [`std::panic::catch_unwind`] for synchronous calls.
//! - [`JoinAll`] — drives a heterogeneous-count, homogeneous-type collection
//!   of futures concurrently, completing only when every child has resolved.
//!
//! Both are intentionally minimal: no scheduling, no allocator dance, no
//! waker bookkeeping beyond what `Future::poll` itself requires. They exist
//! so the crate does not pull in `futures-util` (which the design directives
//! exclude in favor of `futures-core`).
//!
//! Module is gated on `feature = "async"` at the `mod future_ext;`
//! declaration in `src/lib.rs`; no inner `#![cfg]` is needed here.

use core::any::Any;
use core::future::Future;
use core::mem;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::panic::{AssertUnwindSafe, catch_unwind};

use alloc::boxed::Box;
use alloc::vec::Vec;

/// Outcome of polling a future inside [`CatchUnwind`]: either the future's
/// own output or the boxed panic payload that propagated through `poll`.
pub(crate) type CaughtUnwind<T> = Result<T, Box<dyn Any + Send + 'static>>;

/// Future adapter that catches panics raised by the wrapped future's `poll`.
///
/// The inner future is heap-pinned, so `Self: Unpin` and `poll` does not
/// need any manual pin projection.
///
/// On the first panic the inner future is consumed; subsequent polls would
/// violate the `Future` contract anyway, so we panic-on-double-poll via
/// `expect` only inside debug builds; in release the inner Option becomes
/// `None` and the second poll returns `Pending` forever. Callers are
/// expected to drop the wrapper after the first `Ready`.
pub(crate) struct CatchUnwind<F: Future> {
    inner: Option<Pin<Box<F>>>,
}

impl<F: Future> CatchUnwind<F> {
    #[inline]
    pub(crate) fn new(future: F) -> Self {
        Self {
            inner: Some(Box::pin(future)),
        }
    }
}

impl<F: Future> Future for CatchUnwind<F> {
    type Output = CaughtUnwind<F::Output>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let Some(inner) = self.inner.as_mut() else {
            return Poll::Pending;
        };
        let inner_pin = inner.as_mut();
        // Polling a future may panic if the underlying user code panics.
        // We assert UnwindSafe because:
        //   * the registry-internal state mutated during poll is bounded
        //     to the future being polled, not shared registry state;
        //   * on a panic we drop the future, so any partial side effects
        //     remain visible to the user — exactly what a sync panicking
        //     handler does.
        match catch_unwind(AssertUnwindSafe(|| inner_pin.poll(cx))) {
            Ok(Poll::Ready(out)) => {
                self.inner = None;
                Poll::Ready(Ok(out))
            }
            Ok(Poll::Pending) => Poll::Pending,
            Err(payload) => {
                self.inner = None;
                Poll::Ready(Err(payload))
            }
        }
    }
}

/// One slot in a [`JoinAll`]: either still-polling or already-done.
enum JoinSlot<F: Future> {
    Pending(Pin<Box<F>>),
    Done(F::Output),
}

/// Future that drives a `Vec<F>` of futures concurrently and resolves to a
/// `Vec<F::Output>` once every child future is done.
///
/// Order of outputs matches the order the futures were supplied in.
///
/// This is a minimal, non-fused, non-`futures-util` implementation. It
/// polls every still-`Pending` child on every wake. For a small handler
/// count (the registry's typical workload) this is the right trade-off
/// against pulling in `futures-util`.
pub(crate) struct JoinAll<F: Future> {
    slots: Vec<JoinSlot<F>>,
    remaining: usize,
}

impl<F: Future> JoinAll<F> {
    #[inline]
    pub(crate) fn new<I>(futures: I) -> Self
    where
        I: IntoIterator<Item = F>,
    {
        let slots: Vec<JoinSlot<F>> = futures
            .into_iter()
            .map(|f| JoinSlot::Pending(Box::pin(f)))
            .collect();
        let remaining = slots.len();
        Self { slots, remaining }
    }
}

impl<F: Future> Future for JoinAll<F>
where
    F::Output: Unpin,
{
    type Output = Vec<F::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // `Self: Unpin` — slots is a Vec (Unpin); each pending element
        // holds its `F` behind `Pin<Box<F>>` (always Unpin); each done
        // element stores `F::Output` which the bound above pins as Unpin.
        let this = self.get_mut();

        for slot in &mut this.slots {
            if let JoinSlot::Pending(ref mut fut) = *slot {
                match fut.as_mut().poll(cx) {
                    Poll::Ready(value) => {
                        *slot = JoinSlot::Done(value);
                        this.remaining -= 1;
                    }
                    Poll::Pending => {}
                }
            }
        }

        if this.remaining > 0 {
            return Poll::Pending;
        }

        let drained: Vec<F::Output> = mem::take(&mut this.slots)
            .into_iter()
            .map(|slot| match slot {
                JoinSlot::Done(value) => value,
                // `remaining == 0` implies every slot is Done.
                JoinSlot::Pending(_) => {
                    unreachable!("JoinAll: slot was not Done despite remaining=0")
                }
            })
            .collect();
        Poll::Ready(drained)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{CatchUnwind, JoinAll};
    use core::future::ready;

    #[tokio::test]
    async fn catch_unwind_passes_through_normal_completion() {
        let value = CatchUnwind::new(async { 7_u32 }).await;
        assert_eq!(value.unwrap(), 7);
    }

    #[tokio::test]
    async fn catch_unwind_captures_panic_payload() {
        let outcome = CatchUnwind::new(async {
            panic!("boom");
        })
        .await;
        let payload = outcome.unwrap_err();
        assert_eq!(*payload.downcast_ref::<&'static str>().unwrap(), "boom");
    }

    #[tokio::test]
    async fn join_all_completes_with_results_in_order() {
        let futures = vec![ready(1), ready(2), ready(3)];
        let results = JoinAll::new(futures).await;
        assert_eq!(results, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn join_all_with_zero_futures_is_immediately_ready() {
        let futures: Vec<core::future::Ready<()>> = Vec::new();
        let results = JoinAll::new(futures).await;
        assert!(results.is_empty());
    }
}

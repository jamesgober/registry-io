//! Panic-isolating helpers for async dispatch.
//!
//! Handler futures are already boxed (`Pin<Box<dyn Future>>`) by the
//! registry, which makes them `Unpin`. The helpers here poll them in place
//! instead of boxing them again: one allocation per handler (the handler's
//! own future) is the floor for type-erased async handlers, and these
//! helpers add none.

use core::any::Any;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::panic::{AssertUnwindSafe, catch_unwind};

use alloc::boxed::Box;

/// The payload of a caught panic.
pub(crate) type Payload = Box<dyn Any + Send + 'static>;

/// A type-erased handler future.
pub(crate) type BoxFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Calls `make` (the synchronous part of a handler, which builds its
/// future) inside an unwind boundary. A handler can panic before it ever
/// returns a future; that panic is isolated exactly like one during
/// `poll`.
#[inline]
pub(crate) fn start(make: impl FnOnce() -> BoxFuture) -> Result<BoxFuture, Payload> {
    catch_unwind(AssertUnwindSafe(make))
}

/// Polls `fut` once inside an unwind boundary.
#[inline]
pub(crate) fn poll_caught(fut: &mut BoxFuture, cx: &mut Context<'_>) -> Poll<Result<(), Payload>> {
    match catch_unwind(AssertUnwindSafe(|| fut.as_mut().poll(cx))) {
        Ok(Poll::Ready(())) => Poll::Ready(Ok(())),
        Ok(Poll::Pending) => Poll::Pending,
        Err(payload) => Poll::Ready(Err(payload)),
    }
}

/// Awaits one handler future, isolating panics.
pub(crate) struct CatchUnwind {
    inner: Option<BoxFuture>,
}

impl CatchUnwind {
    #[inline]
    pub(crate) fn new(future: BoxFuture) -> Self {
        Self {
            inner: Some(future),
        }
    }
}

impl Future for CatchUnwind {
    type Output = Result<(), Payload>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // A completed `CatchUnwind` is never polled by this crate; if it
        // were, staying pending is the conservative answer.
        let Some(inner) = self.inner.as_mut() else {
            return Poll::Pending;
        };
        let outcome = poll_caught(inner, cx);
        if outcome.is_ready() {
            // Drop the finished (or panicked) future immediately so its
            // resources are released before the caller continues.
            self.inner = None;
        }
        outcome
    }
}

/// One handler's progress inside a concurrent dispatch.
pub(crate) enum Slot {
    /// Still running.
    Running(BoxFuture),
    /// Finished normally.
    Done,
    /// Panicked, either while building its future or while being polled.
    Panicked(Payload),
}

impl Slot {
    /// Builds the slot for a handler whose future-building step produced
    /// `started`.
    #[inline]
    pub(crate) fn new(started: Result<BoxFuture, Payload>) -> Self {
        match started {
            Ok(fut) => Self::Running(fut),
            Err(payload) => Self::Panicked(payload),
        }
    }

    /// Polls a running slot once. Returns `true` when the slot is no longer
    /// running.
    #[inline]
    pub(crate) fn poll(&mut self, cx: &mut Context<'_>) -> bool {
        let Self::Running(fut) = self else {
            return true;
        };
        match poll_caught(fut, cx) {
            Poll::Pending => false,
            Poll::Ready(Ok(())) => {
                *self = Self::Done;
                true
            }
            Poll::Ready(Err(payload)) => {
                *self = Self::Panicked(payload);
                true
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use core::task::Waker;

    fn cx() -> Context<'static> {
        Context::from_waker(Waker::noop())
    }

    #[test]
    fn start_isolates_panic_while_building_future() {
        let r = start(|| panic!("before future"));
        assert!(r.is_err());
        let ok = start(|| Box::pin(async {}));
        assert!(ok.is_ok());
    }

    #[test]
    fn slot_reports_completion_and_panics() {
        let mut cx = cx();
        let mut done = Slot::new(Ok(Box::pin(async {})));
        assert!(done.poll(&mut cx));
        assert!(matches!(done, Slot::Done));

        let mut boom = Slot::new(Ok(Box::pin(async { panic!("in poll") })));
        assert!(boom.poll(&mut cx));
        assert!(matches!(boom, Slot::Panicked(_)));

        let mut early = Slot::new(start(|| panic!("early")));
        assert!(early.poll(&mut cx));
        assert!(matches!(early, Slot::Panicked(_)));
    }

    #[test]
    fn catch_unwind_drops_future_on_completion() {
        let mut cx = cx();
        let mut f = CatchUnwind::new(Box::pin(async {}));
        assert!(matches!(
            Pin::new(&mut f).poll(&mut cx),
            Poll::Ready(Ok(()))
        ));
        assert!(f.inner.is_none());
        assert!(Pin::new(&mut f).poll(&mut cx).is_pending());
    }
}

use std::{
    future::Future,
    task::{Context, Poll},
};

pub trait CancelFuture: Future {
    fn poll_cancel(&mut self, cx: &mut Context<'_>) -> Poll<()>;
}

/// A blanket implementation so all Futures are cancellable.
///
/// We rely on specialization to override this for futures that need to
/// implement custom cancellation logic. By default, we do the default behavior,
/// which is just to run synchronous destructors.
impl<F: Future> CancelFuture for F {
    default fn poll_cancel(&mut self, _cx: &mut Context<'_>) -> Poll<()> {
        Poll::Ready(())
    }
}

/// A macro to create a cancellable async block.
macro_rules! async_cancel {
    ($body:block) => {
        |cx: &mut core::task::Context<'_>| {
            // FIXME: need to stash the context somewhere
            $body
        }
    };
}

/// A macro to await a cancellable future.
macro_rules! awaitc {
    ($f:expr) => {
        let mut f = pin!(f);
        let mut cancelled = false;
        loop {
            if cancelled {

            }
            match f.as_mut().poll(cx) {
                Poll::Ready(()) => return Poll::Ready(()),
                Poll::Pending => {}
            }
            let (cx, should_cancel) 
        }
    };
}
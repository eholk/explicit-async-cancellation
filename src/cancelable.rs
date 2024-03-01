use std::{
    mem,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

use futures::future::Either;
use pin_project::pin_project;

/// A version of core::future::Future that supports explicit cancellation
pub trait Future {
    type Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>;

    fn poll_cancel(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        // By default we preserve the existing cancellation behavior, which is that
        // we don't do anything and we let synchronous destructors do the cleanup.
        Poll::Ready(())
    }
}

/// A compatibility shim to let us use standard Rust futures as cancellable futures
// impl<F: core::future::Future> Future for F {
//     type Output = F::Output;

//     fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
//         core::future::Future::poll(self, cx)
//     }

//     fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
//         Poll::Ready(())
//     }
// }

impl<O, G> Future for G
where
    G: core::ops::Coroutine<PollState, Yield = (), Return = CancelState<O>>,
{
    type Output = O;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.resume(PollState {
            cx: unsafe { mem::transmute(cx) },
            is_cancelled: false,
        }) {
            std::ops::CoroutineState::Yielded(()) => Poll::Pending,
            std::ops::CoroutineState::Complete(CancelState::Complete(v)) => Poll::Ready(v),
            std::ops::CoroutineState::Complete(CancelState::Cancelled) => panic!("cancelled"),
        }
    }

    fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        match self.resume(PollState {
            cx: unsafe { mem::transmute(cx) },
            is_cancelled: true,
        }) {
            std::ops::CoroutineState::Yielded(()) => Poll::Pending,
            std::ops::CoroutineState::Complete(CancelState::Complete(_)) => {
                panic!("future completed after being cancelled")
            }
            std::ops::CoroutineState::Complete(CancelState::Cancelled) => Poll::Ready(()),
        }
    }
}

pub enum CancelState<T> {
    Complete(T),
    Cancelled,
}

pub fn future_from_coroutine<O, G>(gen: G) -> impl Future<Output = O>
where
    G: core::ops::Coroutine<PollState, Yield = (), Return = CancelState<O>>,
{
    gen
}

macro_rules! async_cancel {
    ($body:block) => {
        $crate::cancelable::future_from_coroutine(
            #[allow(unreachable_code)]
            static move |poll_state: $crate::cancelable::PollState| {
                unsafe {
                    $crate::cancelable::save_poll_state(poll_state);
                }
                return $crate::cancelable::CancelState::Complete($body);
                // Add a yield to force this to be a Coroutine
                yield;
            },
        )
    };
}

#[derive(Clone, Copy)]
pub struct PollState {
    pub cx: NonNull<Context<'static>>,
    pub is_cancelled: bool,
}

static mut POLL_STATE: PollState = PollState {
    cx: NonNull::dangling(),
    is_cancelled: false,
};

pub unsafe fn save_poll_state(state: PollState) {
    POLL_STATE = state;
}

pub unsafe fn get_poll_state() -> PollState {
    POLL_STATE
}

/// A version of await that handles cancellation
macro_rules! awaitc {
    ($f:expr) => {{
        let mut f = core::pin::pin!($f);
        loop {
            let $crate::cancelable::PollState { cx, is_cancelled } =
                unsafe { $crate::cancelable::get_poll_state() };

            let cx: &mut core::task::Context<'_> = unsafe { &mut *cx.as_ptr() };

            if is_cancelled {
                match f.as_mut().poll_cancel(cx) {
                    core::task::Poll::Ready(()) => {
                        return $crate::cancelable::CancelState::Cancelled
                    }
                    _ => {}
                }
            } else {
                match f.as_mut().poll(cx) {
                    core::task::Poll::Ready(v) => {
                        break v;
                    }
                    _ => {}
                }
            }

            unsafe {
                $crate::cancelable::save_poll_state(yield);
            }
        }
    }};
}

pub fn ready<T>(t: T) -> impl Future<Output = T> {
    struct Ready<T>(Option<T>);

    impl<T> Future for Ready<T> {
        type Output = T;

        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            unsafe { Poll::Ready(self.get_unchecked_mut().0.take().unwrap()) }
        }
    }

    Ready(Some(t))
}

pub fn pending() -> impl Future<Output = !> {
    struct Pending;

    impl Future for Pending {
        type Output = !;

        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Pending
        }
    }

    Pending
}

pub trait FutureExt: Future + Sized {
    /// Adds a handler to be run when a future is cancelled
    ///
    /// This is primarily for testing and demonstrating cancellation behavior,
    /// rather than something that's meant to be the normal way to handle
    /// cancellation.
    fn on_cancel<H: Future<Output = ()>>(self, hook: H) -> impl Future<Output = Self::Output> {
        #[pin_project]
        struct OnCancel<F, H> {
            #[pin]
            future: F,
            #[pin]
            hook: Option<H>,
        }

        impl<F, H> Future for OnCancel<F, H>
        where
            F: Future,
            H: Future<Output = ()>,
        {
            type Output = F::Output;

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                self.project().future.as_mut().poll(cx)
            }

            fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                let mut this = self.project();
                // if our cancellation hook isn't completed, poll it
                match this.hook.as_mut().as_pin_mut() {
                    Some(hook) => match hook.poll(cx) {
                        // SAFETY: we're not moving, just overwriting.
                        Poll::Ready(()) => unsafe { *this.hook.get_unchecked_mut() = None },
                        Poll::Pending => return Poll::Pending,
                    },
                    None => {}
                }

                // now cancel the inner future
                this.future.as_mut().poll_cancel(cx)
            }
        }

        OnCancel {
            future: self,
            hook: Some(hook),
        }
    }

    fn race<Other: Future>(
        self,
        other: Other,
    ) -> impl Future<Output = Either<Self::Output, Other::Output>> {
        #[pin_project]
        struct Race<A: Future, B: Future> {
            #[pin]
            a: A,
            a_result: Option<A::Output>,
            a_cancelled: bool,
            #[pin]
            b: B,
            b_result: Option<B::Output>,
            b_cancelled: bool,
        }

        impl<A: Future, B: Future> Future for Race<A, B> {
            type Output = Either<A::Output, B::Output>;

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                let mut this = self.project();
                debug_assert!(!*this.a_cancelled);
                debug_assert!(!*this.b_cancelled);
                loop {
                    match (&this.a_result, &this.b_result) {
                        // neither is done so give both a chance to run
                        (None, None) => match this.a.as_mut().poll(cx) {
                            Poll::Ready(v) => {
                                *this.a_result = Some(v);
                            }
                            Poll::Pending => match this.b.as_mut().poll(cx) {
                                Poll::Ready(v) => {
                                    *this.b_result = Some(v);
                                }
                                Poll::Pending => return Poll::Pending,
                            },
                        },
                        // a is finished so cancel b
                        (Some(_), None) => match this.b.poll_cancel(cx) {
                            Poll::Ready(()) => {
                                *this.b_cancelled = true;
                                return Poll::Ready(Either::Left(this.a_result.take().unwrap()));
                            }
                            Poll::Pending => return Poll::Pending,
                        },
                        // b is finished so cancel a
                        (None, Some(_)) => match this.a.poll_cancel(cx) {
                            Poll::Ready(()) => {
                                *this.a_cancelled = true;
                                return Poll::Ready(Either::Right(this.b_result.take().unwrap()));
                            }
                            Poll::Pending => return Poll::Pending,
                        },
                        (Some(_), Some(_)) => panic!("wat"),
                    }
                }
            }

            fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                let mut this = self.project();

                if this.a_result.is_none() && !*this.a_cancelled {
                    match this.a.as_mut().poll_cancel(cx) {
                        Poll::Ready(()) => {
                            *this.a_cancelled = true;
                        }
                        Poll::Pending => {}
                    }
                }

                if this.b_result.is_none() && !*this.b_cancelled {
                    match this.b.as_mut().poll_cancel(cx) {
                        Poll::Ready(()) => {
                            *this.b_cancelled = true;
                        }
                        Poll::Pending => {}
                    }
                }

                if (this.a_result.is_some() || *this.a_cancelled)
                    && (this.b_result.is_some() || *this.b_cancelled)
                {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }
        }

        Race {
            a: self,
            a_result: None,
            a_cancelled: false,
            b: other,
            b_result: None,
            b_cancelled: false,
        }
    }
}

impl<F: Future> FutureExt for F {}

pub fn poll_fn<T, F: FnMut(&mut Context<'_>) -> Poll<T>>(f: F) -> impl Future<Output = T> {
    struct PollFn<T, F: FnMut(&mut Context<'_>) -> Poll<T>>(F);

    impl<T, F: FnMut(&mut Context<'_>) -> Poll<T>> Future for PollFn<T, F> {
        type Output = T;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            (unsafe { self.get_unchecked_mut() }.0)(cx)
        }
    }

    PollFn(f)
}

#[cfg(test)]
mod test {
    use std::{cell::RefCell, panic::catch_unwind, sync::atomic::AtomicBool};

    use crate::executor::Executor;

    use super::*;

    #[test]
    fn exec_cancel() {
        let mut cancelled = false;
        {
            let cancelled = &mut cancelled;
            let mut exec = Executor::new(async_cancel!({
                awaitc!(pending().on_cancel(async_cancel!({ *cancelled = true })));
            }));
            let _ = exec.poll();
        }
        assert!(cancelled);
    }

    #[test]
    fn race_cancel_a() {
        let mut cancelled = false;
        {
            let cancelled = &mut cancelled;
            let exec = Executor::new(async_cancel!({
                awaitc!(async_cancel!({ 42 })
                    .race(pending().on_cancel(async_cancel!({ *cancelled = true }))))
            }));
            let result = exec.run();
            assert!(matches!(result, Either::Left(42)));
        }
        assert!(cancelled);
    }

    #[test]
    fn race_cancel_b() {
        let mut cancelled = false;
        {
            let cancelled = &mut cancelled;
            let exec = Executor::new(async_cancel!({
                awaitc!(pending()
                    .on_cancel(async_cancel!({ *cancelled = true }))
                    .race(async_cancel!({ 42 })))
            }));
            let result = exec.run();
            assert!(matches!(result, Either::Right(42)));
        }
        assert!(cancelled);
    }

    #[test]
    fn race_cancel_both() {
        let mut cancelled_a = false;
        let mut cancelled_b = false;
        {
            let cancelled_a = &mut cancelled_a;
            let cancelled_b = &mut cancelled_b;
            let mut exec = Executor::new(async_cancel!({
                awaitc!(pending()
                    .on_cancel(async_cancel!({ *cancelled_a = true }))
                    .race(pending().on_cancel(async_cancel!({ *cancelled_b = true }))));
            }));
            let _ = exec.poll();
            let _ = exec.poll();
            let _ = exec.poll();
        }
        assert!(cancelled_a);
        assert!(cancelled_b);
    }

    #[test]
    fn cancel_while_canceling() {
        let mut b_cancel_start = None;
        let mut b_cancel_end = None;
        let mut race_cancel_start = None;
        let mut root_cancel_start = None;
        {
            let b_cancel_start = &mut b_cancel_start;
            let b_cancel_end = &mut b_cancel_end;
            let race_cancel_start = &mut race_cancel_start;
            let root_cancel_start = &mut root_cancel_start;

            let clock = &RefCell::new(0);
            let done = &RefCell::new(false);

            let tic = || {
                let mut clock = clock.borrow_mut();
                *clock += 1;
                *clock - 1
            };
            let root = async_cancel!({
                let a = async_cancel!({ 42 });
                let b = pending().on_cancel(
                    poll_fn(|_| {
                        if b_cancel_start.is_none() {
                            *b_cancel_start = Some(tic());
                        }
                        if *done.borrow() {
                            *b_cancel_end = Some(tic());
                            Poll::Ready(())
                        } else {
                            Poll::Pending
                        }
                    })
                    .on_cancel(async_cancel!({
                        panic!("cancelling the cancellation is not allowed")
                    })),
                );
                awaitc!(a.race(b).on_cancel(async_cancel!({
                    *race_cancel_start = Some(tic());
                })))
            })
            .on_cancel(async_cancel!({
                *root_cancel_start = Some(tic());
            }));
            let mut executor = Executor::new(root);
            let _ = executor.poll();
            let _ = executor.poll();
            let _ = executor.poll();
            *done.borrow_mut() = true;
        }

        // B could start cancelling first.
        assert!(b_cancel_start < race_cancel_start);
        // we cancel outside-in, so the root cancels before the race
        assert!(root_cancel_start < race_cancel_start);
        // but b still starts cancelling before the root
        assert!(b_cancel_start < root_cancel_start);
        // but b doesn't finish cancelling until we've started cancelling everything else
        // (i.e. we are in the executor's shutdown path)
        assert!(b_cancel_end > root_cancel_start);
    }

    #[test]
    fn cancel_while_unwinding() {
        let did_cancel = AtomicBool::new(false);
        let did_cancel = &did_cancel;

        let exec = Executor::new(async_cancel!({ panic!("expected panic") }).on_cancel(
            async_cancel!({
                did_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }),
        ));

        let result = catch_unwind(|| exec.run());

        assert!(result.is_err());
        assert!(did_cancel.load(std::sync::atomic::Ordering::Relaxed));
    }
}

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
    G: core::ops::Generator<PollState, Yield = (), Return = CancelState<O>>,
{
    type Output = O;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.resume(PollState {
            cx: unsafe { mem::transmute(cx) },
            is_cancelled: false,
        }) {
            std::ops::GeneratorState::Yielded(()) => Poll::Pending,
            std::ops::GeneratorState::Complete(CancelState::Complete(v)) => Poll::Ready(v),
            std::ops::GeneratorState::Complete(CancelState::Cancelled) => panic!("cancelled"),
        }
    }

    fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        match self.resume(PollState {
            cx: unsafe { mem::transmute(cx) },
            is_cancelled: true,
        }) {
            std::ops::GeneratorState::Yielded(()) => Poll::Pending,
            std::ops::GeneratorState::Complete(CancelState::Complete(_)) => {
                panic!("future completed after being cancelled")
            }
            std::ops::GeneratorState::Complete(CancelState::Cancelled) => Poll::Ready(()),
        }
    }
}

pub enum CancelState<T> {
    Complete(T),
    Cancelled,
}

pub fn future_from_generator<O, G>(gen: G) -> impl Future<Output = O>
where
    G: core::ops::Generator<PollState, Yield = (), Return = CancelState<O>>,
{
    gen
}

macro_rules! async_cancel {
    ($body:block) => {
        $crate::cancelable::future_from_generator(
            #[allow(unreachable_code)]
            static move |poll_state: $crate::cancelable::PollState| {
                unsafe {
                    $crate::cancelable::save_poll_state(poll_state);
                }
                return $crate::cancelable::CancelState::Complete($body);
                // Add a yield to force this to be a generator
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

pub trait FutureExt: Future {
    fn on_cancel<H: FnOnce()>(self, hook: H) -> impl Future<Output = Self::Output>;

    fn race<Other: Future>(
        self,
        other: Other,
    ) -> impl Future<Output = Either<Self::Output, Other::Output>>;
}

impl<F: Future> FutureExt for F {
    fn on_cancel<H: FnOnce()>(self, hook: H) -> impl Future<Output = Self::Output> {
        struct OnCancel<F, H> {
            future: F,
            hook: Option<H>,
        }

        impl<F, H> Future for OnCancel<F, H>
        where
            F: Future,
            H: FnOnce(),
        {
            type Output = F::Output;

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                unsafe { self.map_unchecked_mut(|this| &mut this.future).poll(cx) }
            }

            fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                unsafe {
                    self.as_mut().get_unchecked_mut().hook.take().map(|f| f());
                    self.map_unchecked_mut(|this| &mut this.future)
                        .poll_cancel(cx)
                }
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
            #[pin]
            b: B,
            b_result: Option<B::Output>,
        }

        impl<A: Future, B: Future> Future for Race<A, B> {
            type Output = Either<A::Output, B::Output>;

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                let mut this = self.project();
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
                                return Poll::Ready(Either::Left(this.a_result.take().unwrap()))
                            }
                            Poll::Pending => return Poll::Pending,
                        },
                        // b is finished so cancel a
                        (None, Some(_)) => match this.a.poll_cancel(cx) {
                            Poll::Ready(()) => {
                                return Poll::Ready(Either::Right(this.b_result.take().unwrap()))
                            }
                            Poll::Pending => return Poll::Pending,
                        },
                        (Some(_), Some(_)) => panic!("wat"),
                    }
                }
            }

            fn poll_cancel(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                unimplemented!()
            }
        }

        Race {
            a: self,
            a_result: None,
            b: other,
            b_result: None,
        }
    }
}

#[cfg(test)]
mod test {
    use crate::executor::Executor;

    use super::*;

    #[test]
    fn exec_cancel() {
        let mut cancelled = false;
        {
            let cancelled = &mut cancelled;
            let mut exec = Executor::new(async_cancel!({
                awaitc!(pending().on_cancel(|| *cancelled = true));
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
            let mut exec = Executor::new(async_cancel!({
                awaitc!(async_cancel!({ 42 }).race(pending().on_cancel(|| *cancelled = true)));
            }));
            let _ = exec.poll();
        }
        assert!(cancelled);
    }

    #[test]
    fn race_cancel_b() {
        let mut cancelled = false;
        {
            let cancelled = &mut cancelled;
            let mut exec = Executor::new(async_cancel!({
                awaitc!(pending()
                    .on_cancel(|| *cancelled = true)
                    .race(async_cancel!({ 42 })));
            }));
            let _ = exec.poll();
        }
        assert!(cancelled);
    }
}

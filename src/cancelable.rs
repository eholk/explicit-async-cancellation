use std::{
    mem,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

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

            fn poll_cancel(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                unsafe {
                    self.get_unchecked_mut().hook.take().unwrap()();
                }
                Poll::Ready(())
            }
        }

        OnCancel {
            future: self,
            hook: Some(hook),
        }
    }
}

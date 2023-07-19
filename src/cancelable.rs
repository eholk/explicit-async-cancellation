use std::{
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

/// A version of core::future::Future that supports explicit cancellation
pub trait Future {
    type Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>;

    fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // By default we preserve the existing cancellation behavior, which is that
        // we don't do anything and we let synchronous destructors do the cleanup.
        Poll::Ready(())
    };
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
        todo!()
    }

    fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        todo!()
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
            |poll_state: $crate::cancelable::PollState| {
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
    cx: NonNull<Context<'static>>,
    is_cancelled: bool,
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

macro_rules! awaitc {
    ($f:expr) => {
        let mut f = core::pin::pin!($f);
        loop {
            let $crate::cancelable::PollState { cx, is_cancelled } = unsafe {
                $crate::cancelable::get_poll_state()
            };

            let cx: &mut Context<'_> = &mut *cx.as_ptr();



            unsafe {
                $crate::cancelable::save_poll_state(yield);
            }
        }
    };
}

pub fn ready<T>(t: T) -> impl Future<Output = T> {
    struct Ready<T>(T);

    impl<T> Future for Ready<T> {
        type Output = T;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Ready(self.0)
        }
    }

    Ready(t)
}
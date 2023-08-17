use std::{pin::Pin, task::Poll};

use crate::cancelable::Future;

/// A simple executor that runs a single root future.
///
/// Normally runs to completion, but can be cancelled.
pub struct Executor<F: Future> {
    task: Option<Pin<Box<F>>>,
    complete: bool,
}

impl<F: Future> Executor<F> {
    pub fn new(future: F) -> Self {
        Self {
            task: Some(Box::pin(future)),
            complete: false,
        }
    }

    pub fn poll(&mut self) -> Poll<F::Output> {
        let mut cx = std::task::Context::from_waker(futures::task::noop_waker_ref());

        match self.task.as_mut().unwrap().as_mut().poll(&mut cx) {
            Poll::Ready(v) => {
                self.complete = true;
                Poll::Ready(v)
            }
            Poll::Pending => Poll::Pending,
        }
    }

    pub fn run(mut self) -> F::Output {
        loop {
            if let Poll::Ready(v) = self.poll() {
                return v;
            }
        }
    }
}

impl<F: Future> Drop for Executor<F> {
    fn drop(&mut self) {
        if !self.complete && let Some(mut task) = self.task.take() {
            let mut cx = std::task::Context::from_waker(futures::task::noop_waker_ref());

            while let Poll::Pending = task.as_mut().poll_cancel(&mut cx) {}
        }
    }
}

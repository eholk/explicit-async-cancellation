use std::{pin::Pin, task::Poll};

use crate::cancelable::Future;

/// A simple executor that runs a single root future.
///
/// Normally runs to completion, but can be cancelled.
pub struct Executor<F> {
    task: Pin<Box<F>>,
}

impl<F: Future> Executor<F> {
    pub fn new(future: F) -> Self {
        Self {
            task: Box::pin(future),
        }
    }

    pub fn poll(&mut self) -> Poll<F::Output> {
        let mut cx = std::task::Context::from_waker(futures::task::noop_waker_ref());
        
        self.task.as_mut().poll(&mut cx)
    } 

    pub fn run(mut self) -> F::Output {
        loop {
            if let Poll::Ready(v) = self.poll() {
                return v;
            }
        }
    }
}

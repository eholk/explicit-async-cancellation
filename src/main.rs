#![feature(
    coroutines,
    coroutine_trait,
    never_type,
    let_chains
)]

use std::{cell::RefCell, task::Poll};

use cancelable::{ready, Future};

use crate::{
    cancelable::{pending, poll_fn, FutureExt},
    executor::Executor,
};

#[macro_use]
mod cancelable;
mod executor;
mod iocp;

fn main() {
    let done = &RefCell::new(false);
    let root_task = async_cancel!({
        let a = async_cancel!({ 42 });
        let mut cancel_started = false;
        let b = pending().on_cancel(poll_fn(|_| {
            if !cancel_started {
                println!("begin cancelling `b`");
                cancel_started = true;
            }
            if *done.borrow() {
                println!("cancellation of `b` complete");
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }));

        awaitc!(a.race(b).on_cancel(async_cancel!({
            println!("cancelling `race` future");
        })));
    })
    .on_cancel(async_cancel!({
        println!("cancelling root future");
    }));

    let mut executor = Executor::new(root_task);
    let _ = executor.poll();
    let _ = executor.poll();
    let _ = executor.poll();
    *done.borrow_mut() = true;
}

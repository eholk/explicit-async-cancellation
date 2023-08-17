#![feature(
    generators,
    generator_trait,
    return_position_impl_trait_in_trait,
    never_type,
    let_chains
)]

use std::task::Poll;

use cancelable::{ready, Future};

use crate::cancelable::{pending, FutureExt};

#[macro_use]
mod cancelable;
mod executor;
mod iocp;

fn main() {
    let fut = async_cancel!({
        let world = awaitc!(ready("world"));
        println!("Hello, {world}!");
        awaitc!(pending()
            .on_cancel(|| println!("cancelled"))
            .race(async_cancel!({ 42 })));
    });

    let mut executor = executor::Executor::new(fut);
    let Poll::Pending = executor.poll() else {
        return;
    };
    let Poll::Pending = executor.poll() else {
        return;
    };
    let Poll::Pending = executor.poll() else {
        return;
    };
}

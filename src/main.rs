#![feature(
    generators,
    generator_trait,
    return_position_impl_trait_in_trait,
    never_type
)]

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
        awaitc!(pending().on_cancel(|| println!("cancelled")));
    });

    let mut executor = executor::Executor::new(fut);
    executor.poll();
    executor.poll();
    executor.poll();
}

#![feature(generators, generator_trait)]

use cancelable::{ready, Future};

#[macro_use]
mod cancelable;
mod iocp;

fn main() {
    let fut = async_cancel!({
        let world = awaitc!(ready("world"));
        println!("Hello, {world}!");
    });
}

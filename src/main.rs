#![feature(generators, generator_trait)]

#[macro_use]
mod cancelable;
mod iocp;

fn main() {
    let fut = async_cancel!({
        println!("Hello, world!");
    });

}

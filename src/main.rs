#![feature(specialization)]
#![allow(incomplete_features)]

#[macro_use]
mod cancelable;
mod iocp;

fn main() {
    let fut = async_cancel!({
        println!("Hello, world!");
    });

    println!("Hello, world!");
}

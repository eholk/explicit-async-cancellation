# Async Cancellation Experiments

This repo experiments with augmenting the `Future` trait with a `poll_cancel` method, which gives us a low level building block for a number of cancellation behaviors.

This is best thought of as a parallel implementation of Rust's `async`/`await` system. We provide an `async_cancel!` macro, which works like an `async` block but it has added support for cancellation. Similarly, we have `awaitc!` which works like `await` but can be cancelled.

The crate also provides simple executors and future combinators to show how various cancellation scenarios compose.

## TODO

The following are things we'd still like to try out:

* Cancellation during cancellation
* Cancellation and panic
* Race combinator
* Leaf futures, like completion-based IO

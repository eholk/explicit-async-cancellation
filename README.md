# Async Cancellation Experiments

This repo experiments with augmenting the `Future` trait with a `poll_cancel` method, which gives us a low level building block for a number of cancellation behaviors.

This is best thought of as a parallel implementation of Rust's `async`/`await` system. We provide an `async_cancel!` macro, which works like an `async` block but it has added support for cancellation. Similarly, we have `awaitc!` which works like `await` but can be cancelled.

The crate also provides simple executors and future combinators to show how various cancellation scenarios compose.

## TODO

The following are things we'd still like to try out:

* Cancellation during cancellation
  * See notes below
* Cancellation and panic
  * This probably isn't meaningful without a way to spawn. Basically, we treat the executor as a TaskGroup and cancel all the other tasks if one fails.
* Leaf futures, like completion-based IO

## Cancellation during Cancellation

This is an open question and this repo gives us some room to play around with it.

The place to look is in `OnCancel::poll_cancel` in `cancelable.rs`. As it stands right now, there's no way to run the cancellation hook's cancellation.

```rust
f.on_cancel(
    async_cancel!({ println!("f is being cancelled") }).on_cancel(async_cancel!({
        // there's no way to run this
        println!("f's cancellation is being cancelled")
    })),
);
```

There are a few reasons for this.

First, the `OnCancel::poll_cancel` code never calls `poll_cancel` on the cancellation hook future. This is basically because we can't tell how many levels deep `poll_cancel` is meant to apply.

There's a higher level question though, which is how do you initiate cancellation of a cancellation.
The way we do this for example, if the executor shuts down while a cancellation future is being run.
In this repo, we can get into this situation like this:

```rust
let mut executor = Executor::new(async_cancel!({
    awaitc!(
        async_cancel!({ 42 }).race(pending().on_cancel(async_cancel!({
            awaitc!(pending());
        })))
    )
}));
let _ = executor.poll();
let _ = executor.poll();
let _ = executor.poll();
drop(executor);
```

What happens here is that we race `async { 42 }` with `pending()`. The `async { 42 }` block completes immediately, so we then have to cancel the `pending()` future. The cancellation handler is another `pending()` future that will never finish.
We poll the executor a few times for good measure to make sure we get thoroughly stuck, and then when we drop the executor, it will cancel the root future and we'll should keep waiting on the second `pending()` to finish.

Note that this doesn't actually work, and it's not really even clear what behavior we want.
You can see the `cancel_cancel` test in `cancelable.rs` if you want to poke around at this.

//! Create shared sequentially updated structures in async code.
//!
//! See [`actor!`] for primary documentation.

/// Create a sequentially updating structure.
///
/// Usage:
/// ```
/// sequential::actor! {
///     // This macro defines the struct in an anonymous module (`const _: () = {...};`),
///     // you may include any extra items you want to be in that module as long as:
///     //  1) You include the structure definition and corresponding impl block.
///     //  2) Any extra structure definitions or impl blocks go after the main ones.
///     //     Only #[action] methods in the first impl block that appears to be for the main
///     //     structure will be detected, any others will most likely trigger errors.
///     use std::ops;
///     // The structure itself.
///     struct MyStruct {
///         // ...
/// #       state: usize
///     }
///     // A corresponding `impl` block.
///     impl MyStruct {
///         // This impl block must have a constructor named `new`
///         fn new(/*...*/) -> Self {
///             // ...
/// #           MyStruct { state: 3 }
///         }
///         // You can add any helper functions you want as well,
///         // but only methods with the `#[action]` attribute will be visible outside the
///         // anonymous module.
///         // ...
///         // #[action] methods must be async functions:
///         #[action]
///         async fn action(&mut self, i: usize) -> usize {
///             //...
/// #           ops::AddAssign::add_assign(&mut self.state, i); // yes, `ops` is in scope.
/// #           self.state
///         }
///         // They can also generate multiple values:
///         #[action(multi)]
///         async fn count_now(&self) -> usize {
///             for i in 1..=10 {
///                 yield i;
///             }
///         }
///         #[action(generator)]
///         async fn count_later(&self) -> usize {
///             for i in 1..10 {
///                 yield i;
///             }
///         }
///     }
/// }
/// ```
/// The above invocation will generate a struct definition `MyStructHandle` with the following
/// methods and associated functions:
///
///   * `MyStructHandle::new(/*...*/) -> Self`
///   * `MyStructHandle::action(&self, i: usize) -> usize`
///   * `MyStructHandle::count_now(&self) -> impl futures::Stream<Item = usize>`
///   * `MyStructHandle::count_later(self) -> sequential::ResponseStream<Self, usize>`
///
/// `MyStructHandle` also implements `Clone`.
///
/// Note that the `count_later` method consumes the handle. This is meant to remind
/// users that all methods on `MyStructHandle` will hang until either all values from the
/// [`ResponseStream`] have been consumed or the `ResponseStream` has been dropped. The
/// `MyStructHandle` can be recovered by calling [`ResponseStream::unwrap`].
///
/// The `new` method spins up a thread which holds a copy of `MyStruct` and an unbounded channel
/// receiver. The other methods each send a message to the channel with the arguments to the
/// method and a channel sender for the return value; the corresponding on `MyStruct`
/// is called, and the result is sent back to the calling thread.
///
/// For `#[action(multi)]` methods, the values are made available when `yield` is called, and the
/// thread continues producing them as fast as it can unless the channel has been closed.
///
/// For `#[action(generator)]` methods, the thread polled when a value is requested, and stalls
/// until either the stream is dropped, or the last value is produced. (This is why other method
/// calls hang).
pub use sequential_macros::actor;

pub use futures;
pub use tokio;
pub use tokio_stream;

/// The return value for an `#[action(generator)]`
///
/// Implements `futures::Stream<Item = I>` and can be `unwrap`-ed to get a `T`.
///
/// See [`actor!`] for more information.
pub struct ResponseStream<T, I> {
    self_: T,
    stream: tokio::sync::mpsc::Receiver<Option<I>>,
}
impl<T, I> ::futures::Stream for ResponseStream<T, I> {
    type Item = I;
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        ctx: &mut futures::task::Context,
    ) -> futures::task::Poll<Option<Self::Item>> {
        use futures::task::Poll::*;
        let mut u = unsafe { self.map_unchecked_mut(|me| &mut me.stream) };
        loop {
            match u.poll_recv(ctx) {
                Ready(Some(Some(v))) => return Ready(Some(v)),
                Ready(Some(None)) => continue,
                Ready(None) => return Ready(None),
                Pending => return Pending,
            }
        }
    }
}
impl<T, I> ResponseStream<T, I> {
    #[doc(hidden)]
    pub fn new(self_: T, stream: tokio::sync::mpsc::Receiver<Option<I>>) -> Self {
        Self { self_, stream }
    }
    /// Return the original `actor` for this response stream.
    pub fn unwrap(self) -> T {
        self.self_
    }
}

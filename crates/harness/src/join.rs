//! Waiting for several futures at once, on the task that waits: no spawning, so they may
//! borrow, and every one is polled until it is done.

use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::task::Poll;

/// A future that may borrow for `'a`.
pub(crate) type Running<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The output of every future, in their order, once all are done.
pub(crate) async fn all<T>(mut futures: Vec<Running<'_, T>>) -> Vec<T> {
    let mut outputs: Vec<Option<T>> = futures.iter().map(|_| None).collect();
    poll_fn(|cx| {
        let mut waiting = false;
        for (future, output) in futures.iter_mut().zip(outputs.iter_mut()) {
            // A finished future is never polled again.
            if output.is_none() {
                match future.as_mut().poll(cx) {
                    Poll::Ready(value) => *output = Some(value),
                    Poll::Pending => waiting = true,
                }
            }
        }
        if waiting {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    })
    .await;
    outputs.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_output_comes_back_in_order() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a current-thread runtime always builds");
        let outputs = runtime.block_on(async {
            let (sender, receiver) = tokio::sync::oneshot::channel::<u32>();
            let first: Running<'_, u32> = Box::pin(async move { receiver.await.unwrap_or(0) + 1 });
            let second: Running<'_, u32> = Box::pin(async move {
                let _ = sender.send(41);
                2
            });
            all(vec![first, second]).await
        });
        assert_eq!(outputs, [42, 2]);
    }
}

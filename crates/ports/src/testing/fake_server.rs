//! A model provider's fake server: [`ScriptedHttp`] answering each request with the next
//! scripted reply in the provider's wire format, and keeping what it was asked as the provider's
//! tests read it. Each provider brings only its replies, through [`ScriptedReply`].

use std::io;
use std::marker::PhantomData;
use std::time::Duration;

use super::http::Received;
use super::scripted::{ScriptedHttp, Written};
use crate::conformance::Listening;

/// A provider's scripted reply: how it goes on the wire, and what its server keeps of a request.
pub trait ScriptedReply: 'static {
    /// A request the server received, as the provider's tests read it.
    type Received: From<Received>;

    /// The path the provider's base URL ends in, such as `/v1`, or nothing.
    const BASE: &'static str;

    /// The reply as it goes on the wire.
    fn written(self) -> Written;
}

/// The running server, on 127.0.0.1 only. Dropping it stops the server and waits for its
/// thread.
pub struct FakeServer<R> {
    url: String,
    server: ScriptedHttp,
    replies: PhantomData<fn(R)>,
}

impl<R: ScriptedReply> FakeServer<R> {
    /// Starts a server that answers with `replies`, in order. After the last one, every request
    /// gets a 500 saying the script ran out.
    pub fn start(replies: Vec<R>) -> io::Result<Self> {
        let server = ScriptedHttp::start(replies.into_iter().map(R::written).collect())?;
        Ok(Self {
            url: format!("{}{}", server.url(), R::BASE),
            server,
            replies: PhantomData,
        })
    }

    /// The base URL to configure, such as `http://127.0.0.1:53211/v1`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The requests received so far, in order.
    pub fn received(&self) -> Vec<R::Received> {
        self.server
            .received()
            .into_iter()
            .map(R::Received::from)
            .collect()
    }

    /// Waits until `count` requests have arrived, or `limit` has passed; whether they did.
    pub fn wait_for_requests(&self, count: usize, limit: Duration) -> bool {
        self.server.wait_for_requests(count, limit)
    }
}

impl<R: ScriptedReply> Listening for FakeServer<R> {
    fn heard(&self, limit: Duration) -> bool {
        self.wait_for_requests(1, limit)
    }
}

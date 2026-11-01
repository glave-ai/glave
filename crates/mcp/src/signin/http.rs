//! What sign-in's discovery, registration and tokens go out through: rmcp's OAuth client with
//! glave's rule on what it reaches ([`Reachable`]), at every request and every redirect, since
//! a server's metadata and its redirects each name the next address.

use std::time::Duration;

use http::{Request, Response};
use reqwest::redirect::Policy;
use rmcp::transport::auth::{
    OAuthHttpClient, OAuthHttpClientError, OAuthHttpClientFuture, OAuthHttpRedirectPolicy,
    OAuthHttpRequest,
};

use crate::reachable::Reachable;

/// How long one request may take: rmcp's own limit.
const WITHIN: Duration = Duration::from_secs(30);
/// How many redirects one request may follow: reqwest's own limit.
const REDIRECTS: usize = 10;
/// The most of an answer that is read: rmcp's own limit.
const MOST_BYTES: usize = 1024 * 1024;

/// rmcp's OAuth client, under `reachable`.
pub(super) struct Guarded {
    reachable: Reachable,
    /// For what follows redirects, each checked: registering, as rmcp asks.
    follows: reqwest::Client,
    /// For the rest.
    stops: reqwest::Client,
}

impl Guarded {
    pub(super) fn new(reachable: Reachable) -> Result<Self, reqwest::Error> {
        let client = |redirect: Policy| {
            reachable
                .client()
                .timeout(WITHIN)
                .redirect(redirect)
                .build()
        };
        let follow = Policy::custom(move |attempt| {
            if let Some(refused) = reachable.refuses(attempt.url().as_str()) {
                attempt.error(refused)
            } else if attempt.previous().len() >= REDIRECTS {
                attempt.stop()
            } else {
                attempt.follow()
            }
        });
        Ok(Self {
            reachable,
            follows: client(follow)?,
            stops: client(Policy::none())?,
        })
    }

    /// Sends `request`, following redirects only where `follow` says, and reads the answer.
    async fn send(
        &self,
        request: Request<Vec<u8>>,
        follow: bool,
    ) -> Result<Response<Vec<u8>>, OAuthHttpClientError> {
        let request = reqwest::Request::try_from(request)?;
        if let Some(refused) = self.reachable.refuses(request.url().as_str()) {
            return Err(Box::new(refused));
        }
        let client = if follow { &self.follows } else { &self.stops };
        let mut answer = client.execute(request).await?;
        let mut response = Response::builder()
            .status(answer.status())
            .version(answer.version());
        for (name, value) in answer.headers() {
            response = response.header(name, value);
        }
        let mut body = Vec::new();
        while let Some(chunk) = answer.chunk().await? {
            if chunk.len() > MOST_BYTES.saturating_sub(body.len()) {
                return Err(format!("its answer is over {MOST_BYTES} bytes").into());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(response.body(body)?)
    }
}

impl OAuthHttpClient for Guarded {
    fn execute(&self, request: OAuthHttpRequest) -> OAuthHttpClientFuture<'_> {
        let follow = matches!(request.redirect_policy, OAuthHttpRedirectPolicy::Follow);
        Box::pin(self.send(request.request, follow))
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use glave_ports::testing::FakeHttp;

    use super::*;
    use crate::reachable::Refused;

    fn get(url: &str) -> Request<Vec<u8>> {
        Request::get(url).body(Vec::new()).unwrap()
    }

    fn refused(error: &OAuthHttpClientError) -> bool {
        let error: &(dyn Error + 'static) = &**error;
        std::iter::successors(Some(error), |&error| error.source())
            .any(|error| error.downcast_ref::<Refused>().is_some())
    }

    #[test]
    fn a_metadata_address_is_refused_before_anything_is_sent() {
        glave_ports::block_on!(async {
            for reachable in [Reachable::AlsoPrivate, Reachable::OnlyPublic] {
                let guarded = Guarded::new(reachable).unwrap();
                for url in [
                    "http://169.254.169.254/",
                    "http://[::ffff:169.254.169.254]/",
                ] {
                    let error = guarded.send(get(url), true).await.err().unwrap();
                    assert!(refused(&error), "{url}: {error}");
                }
            }
        });
    }

    #[test]
    fn a_redirect_is_followed_only_when_asked_and_only_to_what_may_be_reached() {
        let there = FakeHttp::always(200, "there").unwrap();
        let to_there = there.url.clone();
        let here = FakeHttp::answering(move |_, path| match path {
            "/there" => (307, to_there.clone()),
            _ => (307, "http://169.254.169.254/".to_owned()),
        })
        .unwrap();
        glave_ports::block_on!(async {
            let guarded = Guarded::new(Reachable::AlsoPrivate).unwrap();
            let url = format!("{}/there", here.url);
            let stopped = guarded.send(get(&url), false).await.unwrap();
            assert_eq!(stopped.status(), 307, "as it came");
            let followed = guarded.send(get(&url), true).await.unwrap();
            assert_eq!(followed.body(), b"there");
            let url = format!("{}/metadata", here.url);
            let error = guarded.send(get(&url), true).await.err().unwrap();
            assert!(refused(&error), "{error}");
        });
        assert_eq!(there.received().len(), 1, "followed once of the two asked");
    }
}

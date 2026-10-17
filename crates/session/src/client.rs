//! A client of a session server over HTTP (`specs/session.md`, "Transports"): each request a
//! JSON-RPC `POST` signed in with a bearer token, and a turn followed as server-sent events,
//! its notifications then its response, as the same [`TurnItem`]s a turn in-process gives. The
//! token goes only in the `Authorization` header, never in a URL, a log or an error.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use glave_ports::SseDecoder;
use glave_ports::auth::safe_to_send;
use serde_json::Value;

use crate::protocol::{ApiError, Notification, Reply, Request, code};
use crate::rpc::{message, outcome};
use crate::turn::TurnItem;

/// How long connecting may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// A session server elsewhere, and the token that signs this client in.
pub struct Remote {
    url: String,
    token: String,
    http: reqwest::Client,
    next_id: AtomicU64,
}

impl Remote {
    /// The server at `url` (https, or http only to this computer), signed in with `token`.
    pub fn new(url: &str, token: String) -> Result<Self, ApiError> {
        let url = url.trim().trim_end_matches('/').to_owned();
        if !safe_to_send(&url, &[]) {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                format!(
                    "the server {url:?} is not https (http only to this computer): your sign-in \
                     goes there"
                ),
            ));
        }
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|e| ApiError::new(code::INTERNAL, format!("could not start HTTP: {e}")))?;
        Ok(Self {
            url,
            token,
            http,
            next_id: AtomicU64::new(1),
        })
    }

    /// Where the server is.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Answers `request`, which must not be one that streams (`turn/start`, `goal/start`).
    pub async fn request(&self, request: &Request) -> Result<Reply, ApiError> {
        let response = self.send(request).await?;
        let text = response.text().await.map_err(|e| self.unreachable(&e))?;
        reply(&text)
    }

    /// Starts `request` (`turn/start` or `goal/start`) and follows it.
    pub async fn start(&self, request: &Request) -> Result<RemoteTurn, ApiError> {
        let response = self.send(request).await?;
        let streams = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"));
        if !streams {
            // Refused before it started, or queued to a turn already running: a JSON answer.
            let text = response.text().await.map_err(|e| self.unreachable(&e))?;
            return Err(match reply(&text) {
                Ok(Reply::Queued { .. }) => ApiError::queued_elsewhere(),
                Ok(_) => ApiError::new(code::INTERNAL, "the server did not stream the turn"),
                Err(refused) => refused,
            });
        }
        Ok(RemoteTurn {
            response,
            decoder: SseDecoder::default(),
            events: VecDeque::new(),
            ended: false,
            url: self.url.clone(),
        })
    }

    /// Posts `request` as JSON-RPC; an HTTP failure becomes the error it carries.
    async fn send(&self, request: &Request) -> Result<reqwest::Response, ApiError> {
        let message = message(self.next_id.fetch_add(1, Ordering::Relaxed), request);
        let response = self
            .http
            .post(format!("{}/", self.url))
            .bearer_auth(&self.token)
            .header("glave-method", request.method())
            .json(&message)
            .send()
            .await
            .map_err(|e| self.unreachable(&e))?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            let text = response.text().await.unwrap_or_default();
            let said = reply(&text).err().map(|error| error.message);
            return Err(ApiError::new(
                code::NO_SIGN_IN,
                format!(
                    "{} did not accept your sign-in{}: set GLAVE_SERVER_TOKEN to a current token",
                    self.url,
                    said.map(|said| format!(" ({said})")).unwrap_or_default()
                ),
            ));
        }
        Ok(response)
    }

    fn unreachable(&self, error: &reqwest::Error) -> ApiError {
        // reqwest names the URL, never the token, which travels only in a header.
        ApiError::new(
            code::INTERNAL,
            format!("could not reach the server at {}: {error}", self.url),
        )
    }
}

/// A turn on the server, followed as it runs.
pub struct RemoteTurn {
    response: reqwest::Response,
    decoder: SseDecoder,
    /// Events decoded and not yet read.
    events: VecDeque<String>,
    ended: bool,
    url: String,
}

impl RemoteTurn {
    /// The next item, or `None` after the ending.
    pub async fn next(&mut self) -> Option<TurnItem> {
        if self.ended {
            return None;
        }
        let item = loop {
            if let Some(data) = self.events.pop_front() {
                match item(&data) {
                    Some(item) => break item,
                    None => continue,
                }
            }
            match self.response.chunk().await {
                Ok(Some(chunk)) => match self.decoder.push(&chunk) {
                    Ok(events) => self.events.extend(events),
                    Err(error) => {
                        break TurnItem::Ended(Err(ApiError::new(
                            code::INTERNAL,
                            format!(
                                "could not read the turn's stream from {}: {error}",
                                self.url
                            ),
                        )));
                    }
                },
                Ok(None) => {
                    if let Some(last) = self.decoder.finish() {
                        self.events.push_back(last);
                        continue;
                    }
                    break TurnItem::Ended(Err(ApiError::new(
                        code::INTERNAL,
                        format!("{} closed the turn's stream before it ended", self.url),
                    )));
                }
                Err(error) => {
                    break TurnItem::Ended(Err(ApiError::new(
                        code::INTERNAL,
                        format!("lost the turn's stream from {}: {error}", self.url),
                    )));
                }
            }
        };
        self.ended = matches!(item, TurnItem::Ended(_));
        Some(item)
    }
}

/// What one event says: a notification, or the turn's response. Nothing for an event that is
/// neither, such as a comment.
fn item(data: &str) -> Option<TurnItem> {
    let Ok(Value::Object(mut fields)) = serde_json::from_str::<Value>(data) else {
        return None;
    };
    if fields.contains_key("method") {
        fields.remove("jsonrpc");
        let note: Notification = serde_json::from_value(Value::Object(fields)).ok()?;
        return Some(TurnItem::Note(Arc::new(note)));
    }
    let ended = match outcome(&Value::Object(fields)) {
        Ok(Reply::Turn { turn }) => Ok(turn),
        Ok(other) => Err(ApiError::new(
            code::INTERNAL,
            format!("the turn ended with {other:?}"),
        )),
        Err(error) => Err(error),
    };
    Some(TurnItem::Ended(ended))
}

/// The reply in a JSON-RPC response's text, or its error.
fn reply(text: &str) -> Result<Reply, ApiError> {
    let response: Value = serde_json::from_str(text).map_err(|e| {
        ApiError::new(
            code::INTERNAL,
            format!("the server's answer is not JSON-RPC: {e}"),
        )
    })?;
    outcome(&response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_or_this_computer_gets_a_sign_in() {
        assert!(Remote::new("http://glave.example.com", "t".into()).is_err());
        assert!(Remote::new("http://127.0.0.1:8080", "t".into()).is_ok());
    }

    #[test]
    fn events_are_notifications_then_the_ending() {
        let thread = glave_event::ThreadId::new(jiff::Timestamp::UNIX_EPOCH, [1; 10]).unwrap();
        let sent = Arc::new(Notification::Live {
            thread,
            update: crate::protocol::Update::Text { text: "hi".into() },
        });
        let Some(TurnItem::Note(note)) = item(&crate::rpc::notify(&sent)) else {
            panic!("a notification")
        };
        assert!(matches!(
            note.as_ref(),
            Notification::Live { update: crate::protocol::Update::Text { text }, .. } if text == "hi"
        ));
        assert!(item("not json").is_none());
        let failed = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32001,"message":"no thread"}}"#;
        let Some(TurnItem::Ended(Err(error))) = item(failed) else {
            panic!("an ending")
        };
        assert_eq!(error.code, code::NO_THREAD);
    }
}

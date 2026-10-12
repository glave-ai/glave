//! A client of a session server over HTTP (`specs/session.md`, "Transports"): each request a
//! JSON-RPC `POST` signed in with a bearer token, and a turn followed as server-sent events,
//! its notifications then its response, as the same [`TurnItem`]s a turn in-process gives. The
//! token goes only in the `Authorization` header, never in a URL, a log or an error.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use crate::protocol::{ApiError, Notification, Reply, Request, code};
use crate::turn::TurnItem;

/// The longest one server-sent event may be, in bytes.
const MAX_EVENT: usize = 16 * 1024 * 1024;

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
        if !safe(&url) {
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
            // Refused before it started: a JSON answer with the error.
            let text = response.text().await.map_err(|e| self.unreachable(&e))?;
            return Err(reply(&text).err().unwrap_or_else(|| {
                ApiError::new(code::INTERNAL, "the server did not stream the turn")
            }));
        }
        Ok(RemoteTurn {
            response,
            buffer: Vec::new(),
            ended: false,
            url: self.url.clone(),
        })
    }

    /// Posts `request` as JSON-RPC; an HTTP failure becomes the error it carries.
    async fn send(&self, request: &Request) -> Result<reqwest::Response, ApiError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut message = serde_json::to_value(request)
            .map_err(|e| ApiError::new(code::INTERNAL, format!("could not send: {e}")))?;
        if let Value::Object(fields) = &mut message {
            fields.insert("jsonrpc".into(), "2.0".into());
            fields.insert("id".into(), id.into());
            fields
                .entry("params")
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
        }
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
    buffer: Vec<u8>,
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
            if let Some(data) = self.event() {
                match item(&data) {
                    Some(item) => break item,
                    None => continue,
                }
            }
            match self.response.chunk().await {
                Ok(Some(chunk)) if self.buffer.len().saturating_add(chunk.len()) > MAX_EVENT => {
                    break TurnItem::Ended(Err(ApiError::new(
                        code::INTERNAL,
                        format!("the server sent an event longer than {MAX_EVENT} bytes"),
                    )));
                }
                Ok(Some(chunk)) => self.buffer.extend_from_slice(&chunk),
                Ok(None) => {
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

    /// The data of the next whole event in the buffer, taken out of it.
    fn event(&mut self) -> Option<String> {
        let end = self.buffer.windows(2).position(|pair| pair == b"\n\n")?;
        let block: Vec<u8> = self.buffer.drain(..end.saturating_add(2)).collect();
        let block = String::from_utf8_lossy(&block);
        let data: Vec<&str> = block
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(|data| data.strip_prefix(' ').unwrap_or(data))
            .collect();
        Some(data.join("\n"))
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
    let ended = match reply(&Value::Object(fields).to_string()) {
        Ok(Reply::Turn { turn }) => Ok(turn),
        Ok(other) => Err(ApiError::new(
            code::INTERNAL,
            format!("the turn ended with {other:?}"),
        )),
        Err(error) => Err(error),
    };
    Some(TurnItem::Ended(ended))
}

/// The reply in a JSON-RPC response, or its error.
fn reply(text: &str) -> Result<Reply, ApiError> {
    let message: Value = serde_json::from_str(text).map_err(|e| {
        ApiError::new(
            code::INTERNAL,
            format!("the server's answer is not JSON-RPC: {e}"),
        )
    })?;
    if let Some(error) = message.get("error") {
        return Err(serde_json::from_value(error.clone()).unwrap_or_else(|_| {
            ApiError::new(code::INTERNAL, format!("the server failed: {error}"))
        }));
    }
    let result = message.get("result").cloned().unwrap_or_else(|| json!({}));
    serde_json::from_value(result).map_err(|e| {
        ApiError::new(
            code::INTERNAL,
            format!("the server's answer is not one glave knows: {e}"),
        )
    })
}

/// Whether `url` is one glave sends a sign-in to: https, or http to this computer.
fn safe(url: &str) -> bool {
    let local = ["http://localhost", "http://127.0.0.1", "http://[::1]"]
        .iter()
        .any(|start| {
            url.strip_prefix(start)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
        });
    url.starts_with("https://") || local
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_or_this_computer_gets_a_sign_in() {
        assert!(safe("https://glave.example.com"));
        assert!(safe("http://127.0.0.1:8080"));
        assert!(safe("http://localhost"));
        assert!(!safe("http://glave.example.com"));
        assert!(!safe("http://localhost.evil.com"));
        assert!(Remote::new("http://glave.example.com", "t".into()).is_err());
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

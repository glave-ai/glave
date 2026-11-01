//! The person's `mcp.json` as glave changes it, which it does only when they turn web search
//! on, or off (`connected/search.rs`): one remote server added, or the
//! one glave added taken out again. Everything else stays as the person wrote it, in its order,
//! each server's text as it was; only the spacing between them is glave's. A crash leaves the
//! old file or the new one, whole, at the place a link points to when the file is one. Changes
//! are made one at a time in this process, each read and written whole off the async runtime;
//! another process writing the file at the same moment (a second glave with no server to ask,
//! or the person in their editor) may have its change or this one lost, as with any file two
//! programs write.

use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use glave_ports::settings::mcp::{Edited, Members, ServerConfig};
use glave_session::protocol::{ApiError, code};
use serde_json::value::RawValue;
use serde_json::{Value, json};

/// What turning off web search did to the person's settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Removed {
    /// The server glave added there is gone.
    Gone,
    /// None is at its address.
    NotThere,
    /// One is, which the person set up themselves, so it stays.
    Theirs,
}

/// What changes the person's `mcp.json`, one change at a time.
#[derive(Debug, Clone, Default)]
pub(crate) struct Writer {
    one_at_a_time: Arc<Mutex<()>>,
}

impl Writer {
    /// Adds a remote server at `url` to the settings in `home`, called `name`, or `name-2` and on
    /// when the person calls another server that, unless a server is at that address already:
    /// what the settings call the one there.
    pub(crate) async fn add(&self, home: &Path, name: &str, url: &str) -> Result<String, ApiError> {
        let (name, url) = (name.to_owned(), url.to_owned());
        self.changed(home, move |servers, configs| {
            if let Some((there, _)) = configs.iter().find(|(_, config)| at(config, &url)) {
                return Ok((there.clone(), false));
            }
            let (mut free, mut tried) = (name.clone(), 1);
            while servers.get(&free).is_some() {
                tried += 1;
                free = format!("{name}-{tried}");
            }
            let written = RawValue::from_string(ours(&url).to_string())
                .map_err(|error| ApiError::internal(&error))?;
            servers.set(&free, written);
            Ok((free, true))
        })
        .await
    }

    /// Takes out of the settings in `home` the server at `url` glave added there, as glave wrote
    /// it; one the person set up themselves at that address stays.
    pub(crate) async fn remove(&self, home: &Path, url: &str) -> Result<Removed, ApiError> {
        let url = url.to_owned();
        self.changed(home, move |servers, configs| {
            let there: Vec<&String> = configs
                .iter()
                .filter(|(_, config)| at(config, &url))
                .map(|(name, _)| name)
                .collect();
            if there.is_empty() {
                return Ok((Removed::NotThere, false));
            }
            let written = |name: &String| {
                servers.get(name).is_some_and(|raw| {
                    serde_json::from_str::<Value>(raw.get()).ok() == Some(ours(&url))
                })
            };
            if !there.iter().all(|name| written(name)) {
                return Ok((Removed::Theirs, false));
            }
            servers.retain(|name| !there.iter().any(|there| there.as_str() == name));
            Ok((Removed::Gone, true))
        })
        .await
    }

    /// Reads the settings in `home`, lets `change` change their servers, seeing what each is,
    /// and writes them back when it says it did: what it answers.
    async fn changed<T: Send + 'static>(
        &self,
        home: &Path,
        change: impl FnOnce(&mut Members, &[(String, ServerConfig)]) -> Result<(T, bool), ApiError>
        + Send
        + 'static,
    ) -> Result<T, ApiError> {
        let path = home.join(super::FILE);
        let one_at_a_time = Arc::clone(&self.one_at_a_time);
        let written = tokio::task::spawn_blocking(move || {
            let _alone = one_at_a_time.lock().unwrap_or_else(PoisonError::into_inner);
            rewrite(&path, change)
        });
        written.await.map_err(|error| ApiError::internal(&error))?
    }
}

/// The server glave writes for `url`.
fn ours(url: &str) -> Value {
    json!({"type": "http", "url": url})
}

/// Whether `config` is a remote server at `url`, as glave keeps addresses.
fn at(config: &ServerConfig, url: &str) -> bool {
    match config {
        ServerConfig::Http { url: there, .. } => {
            let there = glave_mcp::address(there);
            there.is_some() && there == glave_mcp::address(url)
        }
        ServerConfig::Stdio { .. } => false,
    }
}

/// [`Writer::changed`]'s work, on a thread that may block.
fn rewrite<T>(
    path: &Path,
    change: impl FnOnce(&mut Members, &[(String, ServerConfig)]) -> Result<(T, bool), ApiError>,
) -> Result<T, ApiError> {
    // A link is followed, so the file it points to is the one written.
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(unreadable(&path, &error)),
    };
    let mut edited = Edited::read(&text).map_err(|error| unreadable(&path, &error))?;
    let configs = if text.trim().is_empty() {
        Vec::new()
    } else {
        let configs = glave_ports::settings::mcp::parse(&text, &super::environment())
            .map_err(|error| unreadable(&path, &error))?;
        configs.into_iter().collect()
    };
    let (answer, changed) = change(&mut edited.servers, &configs)?;
    if !changed {
        return Ok(answer);
    }
    let written = edited.text().map_err(|error| ApiError::internal(&error))?;
    glave_ports::file::replace(&path, written.as_bytes()).map_err(|e| ApiError::internal(&e))?;
    Ok(answer)
}

/// Why the settings at `path` could not be changed: they do not read. The person is told in
/// plain words; the log says where and why.
fn unreadable(path: &Path, error: &dyn fmt::Display) -> ApiError {
    tracing::warn!("could not read {}: {error}", path.display());
    ApiError::new(
        code::NOT_AVAILABLE,
        "your own connections' settings can't be read, so glave changed nothing: fix them, then \
         try again",
    )
}

#[cfg(test)]
mod tests;

//! How much each connection was used (`specs/session.md`, "Connections" and "Web search"): the
//! calls the person's agent asked of its tools since this month began (UTC), counted from the
//! log, a view over it with nothing more kept. A connection's tools reach the model as
//! `mcp__<name>__<tool>` after the name the person's settings give its server, so its calls are
//! the `tool.call.requested` events naming one, in the threads of every project the person owns,
//! archived ones too, that changed this month. The count reads every event of those threads,
//! so it has a time limit: one that runs out, or fails, leaves the connections without a count.

use std::collections::BTreeMap;
use std::time::Duration;

use glave_event::{At, Body, Payload as _, RawEvent, ToolCallRequested};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, Scope, Standing, ThreadQuery};
use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::connections::Searching;
use crate::protocol::{ApiError, ConnectionInfo, SearchInfo, Usage};
use crate::server::Server;
use crate::words::Sentence;

/// Views, or events, read from the store at a time.
const PAGE: usize = 1_000;
/// The longest a count may take, so a list of connections is never held up long by it.
const COUNTING: Duration = Duration::from_secs(5);

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Gives each of `connections` in the person's settings how much it was used.
    pub(crate) async fn used(&self, caller: &Caller, connections: &mut [ConnectionInfo]) {
        let servers: Vec<&str> = connections
            .iter()
            .filter_map(|connection| connection.name.as_deref())
            .collect();
        let Some(calls) = self.counted(caller, &servers).await else {
            return;
        };
        for connection in connections.iter_mut() {
            let Some(name) = &connection.name else {
                continue;
            };
            let calls = calls.get(name).copied().unwrap_or(0);
            let sentence = Sentence::CallsThisMonth { calls }.into();
            connection.usage = Some(Usage { calls, sentence });
        }
    }

    /// Web search as the person sees it, from `searching`: never theirs to choose when there is
    /// none; else whether it is on, who searches and what it costs, and, while it is on, how
    /// much it searched this month.
    pub(crate) async fn search_info(
        &self,
        caller: &Caller,
        searching: Option<Searching>,
    ) -> SearchInfo {
        let Some(Searching { on, name }) = searching else {
            return SearchInfo::Never;
        };
        let usage = if on {
            let counted = self.counted(caller, &[name.as_str()]).await;
            counted.map(|calls| {
                let searches = calls.get(&name).copied().unwrap_or(0);
                let sentence = Sentence::SearchesThisMonth { searches }.into();
                Usage {
                    calls: searches,
                    sentence,
                }
            })
        } else {
            None
        };
        SearchInfo::Chosen {
            on,
            detail: Sentence::SearchDetail.into(),
            usage,
        }
    }

    /// The calls asked this month of the tools of each of `servers`; none when there are no
    /// servers, or the count fails or takes longer than [`COUNTING`], which the log says.
    async fn counted(&self, caller: &Caller, servers: &[&str]) -> Option<BTreeMap<String, u64>> {
        if servers.is_empty() {
            return None;
        }
        match tokio::time::timeout(COUNTING, self.calls(caller, servers)).await {
            Ok(Ok(calls)) => Some(calls),
            Ok(Err(error)) => {
                tracing::warn!(
                    "could not count the calls to connections: {}",
                    error.message
                );
                None
            }
            Err(_) => {
                tracing::warn!("counting the calls to connections took over {COUNTING:?}");
                None
            }
        }
    }

    /// The calls asked this month of the tools of each of `servers`, by the name the person's
    /// settings give it. A project that cannot be read is left out, and the log says so.
    async fn calls(
        &self,
        caller: &Caller,
        servers: &[&str],
    ) -> Result<BTreeMap<String, u64>, ApiError> {
        let since = month_start(self.inner.harness.clock().now())?;
        let prefixes: Vec<(String, &str)> = servers
            .iter()
            .map(|server| (glave_mcp::exposed(server, ""), *server))
            .collect();
        let mut calls = BTreeMap::new();
        for project in self.projects_of(caller).await? {
            let scope = match self.authorized(caller, project).await {
                Ok((_, scope)) => scope,
                Err(error) => {
                    tracing::warn!("calls in project {project} not counted: {}", error.message);
                    continue;
                }
            };
            for standing in [Standing::Active, Standing::Archived] {
                self.tally_threads(&scope, standing, since, &prefixes, &mut calls)
                    .await?;
            }
        }
        Ok(calls)
    }

    /// Adds to `calls` those in the threads of `standing` in `scope` that changed at or after
    /// `since`, newest first, so the first older than that ends it.
    async fn tally_threads(
        &self,
        scope: &Scope,
        standing: Standing,
        since: At,
        prefixes: &[(String, &str)],
        calls: &mut BTreeMap<String, u64>,
    ) -> Result<(), ApiError> {
        let log = self.inner.harness.log();
        let mut query = ThreadQuery {
            standing,
            limit: PAGE,
            after: None,
            open: false,
            parent: None,
        };
        loop {
            let views = log.thread_views(scope, &query).await?;
            for view in &views {
                if view.updated_at < since {
                    return Ok(());
                }
                let mut after = None;
                loop {
                    let events = log.read(scope, view.thread, after, PAGE).await?;
                    tally(&events, since, prefixes, calls);
                    match events.last() {
                        Some(last) if events.len() == PAGE => after = Some(last.seq),
                        _ => break,
                    }
                }
            }
            match views.last() {
                Some(last) if views.len() == PAGE => query.after = Some(last.cursor()),
                _ => return Ok(()),
            }
        }
    }
}

/// When this month began, in UTC, at `now`.
fn month_start(now: Timestamp) -> Result<At, ApiError> {
    let first = now.to_zoned(TimeZone::UTC).date().first_of_month();
    let start = first
        .to_zoned(TimeZone::UTC)
        .map_err(|error| ApiError::internal(&error))?;
    Ok(At::from(start.timestamp()))
}

/// Adds to `calls` each call in `events` at or after `since` of a tool whose name starts with
/// one of `prefixes`, to its server; the longest prefix wins, as a server's own name may hold
/// another's.
fn tally(
    events: &[RawEvent],
    since: At,
    prefixes: &[(String, &str)],
    calls: &mut BTreeMap<String, u64>,
) {
    let asked = events
        .iter()
        .filter(|event| event.kind.as_str() == ToolCallRequested::TYPE && event.at >= since)
        .filter_map(
            |event| match glave_event::decode(event.clone()).map(|it| it.body) {
                Ok(Body::ToolCallRequested(requested)) => Some(requested.tool),
                _ => None,
            },
        );
    for tool in asked {
        let server = prefixes
            .iter()
            .filter(|(prefix, _)| tool.starts_with(prefix.as_str()))
            .max_by_key(|(prefix, _)| prefix.len());
        if let Some((_, server)) = server {
            *calls.entry((*server).to_owned()).or_insert(0) += 1;
        }
    }
}

#[cfg(test)]
mod tests;

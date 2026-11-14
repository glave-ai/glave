//! Web search as the person sees it (`specs/session.md`, "Web search"): glave's own `web_search`,
//! searching through the service the server's settings name with the server's key, which is
//! the person on this computer's to turn on and off in Settings (`search/get`, `search/set`). It
//! is on until they turn it off; their choice is kept in their settings ledger (`search.set`),
//! read before the first turn and handed to the harness, which offers the tool from each thread's
//! next turn. Anyone else's threads search wherever the server does, never theirs to choose.

use glave_event::{Body, SearchSet};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};

use crate::protocol::{ApiError, Reply, SearchInfo, Usage, code};
use crate::providers::local;
use crate::server::Server;
use crate::words::Sentence;

/// The service the server searches through, as the person is told.
const SERVICE: &str = "Brave Search";

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Why web search is not `caller`'s to choose, if it is not: the server searches nothing,
    /// or they are not the person on this computer.
    fn search_refused(&self, caller: &Caller) -> Option<ApiError> {
        if !self.inner.harness.can_search() {
            return Some(ApiError::new(code::NOT_AVAILABLE, Sentence::SearchNotHere));
        }
        (!local(caller)).then(|| ApiError::new(code::NOT_ALLOWED, Sentence::SearchOnItsComputer))
    }

    /// Reads the person on this computer's choice from their settings ledger and hands it to
    /// the harness, the first time it is needed: on with none, or one this glave cannot read. One
    /// that could not be read is read again next time.
    pub(crate) async fn read_search_choice(&self) -> bool {
        let read = self.inner.searching.get_or_try_init(|| async {
            // Glave's own read, for the person on this computer, whose switch it is.
            let on = match self.setting::<SearchSet>(&Caller::default()).await? {
                Some(Body::SearchSet(set)) => set.on,
                _ => true,
            };
            self.inner.harness.set_searching(on);
            Ok::<_, ApiError>(())
        });
        if let Err(error) = read.await {
            tracing::warn!(%error, "could not read whether the person searches the web");
        }
        self.inner.harness.searching()
    }

    /// Answers `search/get`, counting this month's searches when `usage` asks: never the
    /// caller's to choose, with why, where `search/set` would refuse them.
    pub(crate) async fn get_search(&self, caller: &Caller, usage: bool) -> Result<Reply, ApiError> {
        if let Some(refused) = self.search_refused(caller) {
            return Ok(Reply::Search {
                search: SearchInfo::Never {
                    reason: refused.message,
                },
            });
        }
        let on = self.read_search_choice().await;
        let search = self.search_info(caller, on, usage).await;
        Ok(Reply::Search { search })
    }

    /// Answers `search/set`, with this month's searches, for Settings, which turned it.
    pub(crate) async fn set_search(&self, caller: &Caller, on: bool) -> Result<Reply, ApiError> {
        if let Some(refused) = self.search_refused(caller) {
            return Err(refused);
        }
        // Read first, so a read of the ledger from before this choice never overrides it.
        self.read_search_choice().await;
        self.keep_setting(caller, &SearchSet { on }).await?;
        self.inner.harness.set_searching(on);
        let search = self.search_info(caller, on, true).await;
        Ok(Reply::Search { search })
    }

    /// Web search as the person sees it: whether it is on, who searches, and, while it is on
    /// and when `usage` asks, how often it searched this month, which reads the month's threads.
    async fn search_info(&self, caller: &Caller, on: bool, usage: bool) -> SearchInfo {
        let usage = if on && usage {
            self.searches_this_month(caller)
                .await
                .map(|searches| Usage {
                    calls: searches,
                    sentence: Sentence::SearchesThisMonth { searches }.into(),
                })
        } else {
            None
        };
        SearchInfo::Chosen {
            on,
            detail: Sentence::SearchDetail {
                service: SERVICE.to_owned(),
            }
            .into(),
            usage,
        }
    }
}

//! The shares' list (`specs/events.md`): one stream of glave's own, in the server's own
//! organization's scope, of each link by the SHA-256 of its id alone with the organization and
//! project that hold its share, listed as the link is made (`share.placed`) and unlisted once
//! it stops (`share.unplaced`). A server keeps what it read of it in memory, reading on from
//! where it last read when asked for a link it doesn't hold, so serving a link finds its share
//! without knowing whose it is.

use std::collections::HashMap;
use std::sync::PoisonError;

use glave_event::{Body, Payload as _, RawEvent, Seq, SharePlaced, ShareUnplaced};
use glave_ports::stream::{Reserved, catch_up, in_order};
use glave_ports::{BlobStore, EventLog, Located, ModelProvider, OrgId, ProjectId, Scope};

use crate::protocol::ApiError;
use crate::server::Server;

/// The shares' list as far as this server has read it.
#[derive(Debug, Default)]
pub(crate) struct Links {
    next: u64,
    held: HashMap<String, Located>,
}

impl Links {
    /// Takes in the event at the next `seq`.
    fn take(&mut self, raw: RawEvent) {
        match in_order(&mut self.next, raw).map(|(_, body)| body) {
            Some(Body::SharePlaced(placed)) => {
                let at = Located {
                    org: OrgId::from_uuid(placed.org),
                    project: ProjectId::from_uuid(placed.project),
                };
                self.held.insert(placed.link, at);
            }
            Some(Body::ShareUnplaced(unplaced)) => {
                self.held.remove(&unplaced.link);
            }
            _ => {}
        }
    }
}

/// The kinds the list holds, which each writer numbers its event after.
const LISTED: [&str; 2] = [SharePlaced::TYPE, ShareUnplaced::TYPE];

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Lists the link whose id hashes to `key` as held by the project `at`.
    pub(crate) async fn place_link(&self, key: &str, at: Located) -> Result<(), ApiError> {
        let placed = SharePlaced {
            link: key.to_owned(),
            org: at.org.as_uuid(),
            project: at.project.as_uuid(),
        };
        let scope = self.links_scope();
        self.keep_latest(&scope, Reserved::Links.id(), &LISTED, &placed)
            .await?;
        self.read_links().await
    }

    /// Unlists the link whose id hashes to `key`.
    pub(crate) async fn unplace_link(&self, key: &str) -> Result<(), ApiError> {
        let unplaced = ShareUnplaced {
            link: key.to_owned(),
        };
        let scope = self.links_scope();
        self.keep_latest(&scope, Reserved::Links.id(), &LISTED, &unplaced)
            .await?;
        self.read_links().await
    }

    /// Where the link whose id hashes to `key` is held, read on from where this server last
    /// read when it holds none.
    pub(crate) async fn find_link(&self, key: &str) -> Result<Option<Located>, ApiError> {
        if let Some(at) = self.held_links(|links| links.held.get(key).copied()) {
            return Ok(Some(at));
        }
        self.read_links().await?;
        Ok(self.held_links(|links| links.held.get(key).copied()))
    }

    /// How many links the list holds now.
    pub(crate) async fn links_held(&self) -> Result<usize, ApiError> {
        self.read_links().await?;
        Ok(self.held_links(|links| links.held.len()))
    }

    /// Reads whatever was added to the list since this server last looked.
    async fn read_links(&self) -> Result<(), ApiError> {
        let after = self.held_links(|links| Seq::new(links.next.saturating_sub(1)).ok());
        let log = self.inner.harness.log();
        catch_up(
            log,
            &self.links_scope(),
            Reserved::Links.id(),
            after,
            |page| {
                self.held_links(|links| page.into_iter().for_each(|raw| links.take(raw)));
            },
        )
        .await?;
        Ok(())
    }

    /// Where the list is: the server's own organization's scope, glave's own, every
    /// organization's ids in it and nothing more.
    fn links_scope(&self) -> Scope {
        Scope::of_organization(self.inner.own)
    }

    fn held_links<R>(&self, look: impl FnOnce(&mut Links) -> R) -> R {
        let mut links = (self.inner.links)
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        look(&mut links)
    }
}

//! The shares' list (`specs/events.md`): one stream of glave's own, in the server's own
//! organization's scope, of each link by the SHA-256 of its id alone with the organization and
//! project that hold its share and the person who shares it, listed as the link is made
//! (`share.placed`) and unlisted once it stops (`share.unplaced`). A server keeps what it read
//! of it in memory, reading on from where it last read when asked for a link it doesn't hold,
//! so serving a link finds its share without knowing whose it is. It keeps every listed link's
//! key, 32 bytes each, so a link never made is answered without reading the store; where each
//! is, at most [`MOST_HELD`] of them: past that it lets some go, and a listed link it let go of
//! is looked for in the whole list, which only someone holding a live link can ask for. How
//! many links each person holds is counted exactly, whatever it let go.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::PoisonError;

use glave_event::{Body, Payload as _, PrincipalId, RawEvent, Seq, SharePlaced, ShareUnplaced};
use glave_ports::stream::{Reserved, catch_up, in_order};
use glave_ports::{BlobStore, EventLog, Located, ModelProvider, OrgId, ProjectId, Scope};

use crate::protocol::ApiError;
use crate::server::Server;

/// The most links a server holds where they are in memory; past them, it lets some go.
const MOST_HELD: usize = 100_000;

/// The shares' list as far as this server has read it: where each link is, at most
/// [`MOST_HELD`] of them, and how many each person holds, all of theirs.
#[derive(Debug, Default)]
pub(crate) struct Links {
    next: u64,
    /// By the SHA-256 of each link's id, so letting the first go lets go of one at random.
    held: BTreeMap<String, Located>,
    /// Every listed link's key, held or let go.
    listed: HashSet<[u8; 32]>,
    each: HashMap<PrincipalId, usize>,
}

impl Links {
    /// Takes in the event at the next `seq`.
    fn take(&mut self, raw: RawEvent) {
        match in_order(&mut self.next, raw).map(|(_, body)| body) {
            Some(Body::SharePlaced(placed)) => {
                *self.each.entry(placed.owner).or_default() += 1;
                if let Some(key) = key_bytes(&placed.link) {
                    self.listed.insert(key);
                }
                let at = located(&placed);
                self.hold(placed.link, at);
            }
            Some(Body::ShareUnplaced(unplaced)) => {
                if let Some(count) = self.each.get_mut(&unplaced.owner) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        self.each.remove(&unplaced.owner);
                    }
                }
                if let Some(key) = key_bytes(&unplaced.link) {
                    self.listed.remove(&key);
                }
                self.held.remove(&unplaced.link);
            }
            _ => {}
        }
    }

    /// Holds where the link `key` is, letting one go past [`MOST_HELD`].
    fn hold(&mut self, key: String, at: Located) {
        self.held.insert(key, at);
        if self.held.len() > MOST_HELD {
            self.held.pop_first();
        }
    }

    /// Whether the link `key` is listed, held or not.
    fn lists(&self, key: &str) -> bool {
        key_bytes(key).is_some_and(|key| self.listed.contains(&key))
    }
}

/// A link's key, its id's SHA-256 written in hex, as its 32 bytes.
fn key_bytes(key: &str) -> Option<[u8; 32]> {
    let mut bytes = [0u8; 32];
    if key.len() != 64 {
        return None;
    }
    for (byte, pair) in bytes.iter_mut().zip(key.as_bytes().chunks(2)) {
        let pair = std::str::from_utf8(pair).ok()?;
        *byte = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(bytes)
}

/// Where `placed`'s share is.
fn located(placed: &SharePlaced) -> Located {
    Located {
        org: OrgId::from_uuid(placed.org),
        project: ProjectId::from_uuid(placed.project),
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
    /// Lists the link whose id hashes to `key` as held by the project `at`, shared by `owner`,
    /// and answers how many links `owner` holds now, this one among them.
    pub(crate) async fn place_link(
        &self,
        key: &str,
        at: Located,
        owner: PrincipalId,
    ) -> Result<usize, ApiError> {
        let placed = SharePlaced {
            link: key.to_owned(),
            org: at.org.as_uuid(),
            project: at.project.as_uuid(),
            owner,
        };
        let scope = self.links_scope();
        self.keep_latest(&scope, Reserved::Links.id(), &LISTED, &placed)
            .await?;
        self.read_links().await?;
        Ok(self.held_links(|links| links.each.get(&owner).copied().unwrap_or_default()))
    }

    /// Unlists the link whose id hashes to `key`, shared by `owner`.
    pub(crate) async fn unplace_link(&self, key: &str, owner: PrincipalId) -> Result<(), ApiError> {
        let unplaced = ShareUnplaced {
            link: key.to_owned(),
            owner,
        };
        let scope = self.links_scope();
        self.keep_latest(&scope, Reserved::Links.id(), &LISTED, &unplaced)
            .await?;
        self.read_links().await
    }

    /// Where the link whose id hashes to `key` is held, read on from where this server last
    /// read when it holds none, and looked for in the whole list only when it is listed and was
    /// let go of: a link never made, or stopped, reads nothing more.
    pub(crate) async fn find_link(&self, key: &str) -> Result<Option<Located>, ApiError> {
        if let Some(at) = self.held_links(|links| links.held.get(key).copied()) {
            return Ok(Some(at));
        }
        self.read_links().await?;
        let (found, listed) =
            self.held_links(|links| (links.held.get(key).copied(), links.lists(key)));
        match found {
            None if listed => {
                let found = self.look_for_link(key).await?;
                if let Some(at) = found {
                    self.held_links(|links| links.hold(key.to_owned(), at));
                }
                Ok(found)
            }
            found => Ok(found),
        }
    }

    /// Where the link whose id hashes to `key` is, read from the whole list, keeping nothing
    /// else of it.
    async fn look_for_link(&self, key: &str) -> Result<Option<Located>, ApiError> {
        let mut found = None;
        let log = self.inner.harness.log();
        catch_up(
            log,
            &self.links_scope(),
            Reserved::Links.id(),
            None,
            |page| {
                for raw in page {
                    match glave_event::decode(raw).map(|event| event.body) {
                        Ok(Body::SharePlaced(placed)) if placed.link == key => {
                            found = Some(located(&placed));
                        }
                        Ok(Body::ShareUnplaced(unplaced)) if unplaced.link == key => found = None,
                        _ => {}
                    }
                }
            },
        )
        .await?;
        Ok(found)
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

#[cfg(test)]
mod tests {
    use glave_event::{At, EventId, Header, Payload, encode};
    use jiff::Timestamp;

    use super::*;

    fn person(n: u8) -> PrincipalId {
        PrincipalId::new(Timestamp::UNIX_EPOCH, [n; 10]).unwrap()
    }

    /// The `seq`th event of the list, holding `payload`.
    fn listed<T: Payload>(seq: u64, payload: &T) -> RawEvent {
        let at = At::from(Timestamp::UNIX_EPOCH);
        let mut random = [0; 10];
        random[..8].copy_from_slice(&seq.to_be_bytes());
        let header = Header {
            id: EventId::new(Timestamp::UNIX_EPOCH, random).unwrap(),
            thread: Reserved::Links.id(),
            seq: Seq::new(seq).unwrap(),
            at,
        };
        encode(header, payload).unwrap()
    }

    fn placed(n: usize, owner: PrincipalId) -> SharePlaced {
        SharePlaced {
            link: format!("{n:064x}"),
            org: uuid::Uuid::nil(),
            project: uuid::Uuid::nil(),
            owner,
        }
    }

    #[test]
    fn past_its_room_the_list_lets_links_go_and_still_counts_each_person_s_all() {
        let (ada, grace) = (person(1), person(2));
        let mut links = Links::default();
        let mut seq = 0;
        for n in 0..=MOST_HELD {
            seq += 1;
            let owner = if n % 2 == 0 { ada } else { grace };
            links.take(listed(seq, &placed(n, owner)));
        }
        assert_eq!(links.held.len(), MOST_HELD, "one let go");
        assert!(
            links.lists(&format!("{:064x}", 0)),
            "let go of, still listed"
        );
        assert!(
            !links.lists(&format!("{:064x}", MOST_HELD + 1)),
            "never made"
        );
        assert_eq!(links.each.get(&ada), Some(&(MOST_HELD / 2 + 1)));
        assert_eq!(links.each.get(&grace), Some(&(MOST_HELD / 2)));
        // Unlisting counts one fewer, whether it was held or let go.
        for n in [0, 2] {
            seq += 1;
            let unplaced = ShareUnplaced {
                link: format!("{n:064x}"),
                owner: ada,
            };
            links.take(listed(seq, &unplaced));
        }
        assert_eq!(links.each.get(&ada), Some(&(MOST_HELD / 2 - 1)));
        assert!(!links.lists(&format!("{:064x}", 0)), "unlisted");
        assert!(!links.held.contains_key(&format!("{:064x}", 2)));
    }
}

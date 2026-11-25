//! Sharing a file at a link of its own (`specs/session.md`, "Sharing"): `share/get` and
//! `share/set` over the shares ledger of the project that holds the file. A share holds a
//! frozen copy of the file and its preview under a holder of its own, as a Library item holds
//! its picture, so it outlives the chat; its link's id, 128 random bits, is listed by its
//! SHA-256 alone in the shares' list (`links`), where a server serving the link finds it.
//! Served only where the server's configuration names a share domain, never at glave's own
//! origin nor under its name (`http/shared.rs`).

mod links;
mod people;

pub(crate) use self::links::Links;
pub(crate) use self::people::PROOF_FOR;

use std::collections::{BTreeMap, HashMap};

use glave_event::{
    At, Audience, Body, MadePreview, Payload as _, ShareId, ShareSet, ShareStopped, SharedFrom,
    ThreadId, decode,
};
use glave_ports::host::PublicOrigin;
use glave_ports::stream::{Reserved, catch_up};
use glave_ports::{BlobStore, Caller, EventLog, Located, ModelProvider, Scope, digest};

use crate::files::not_here;
use crate::protocol::{ApiError, Reply, ShareChoice, ShareInfo, ShareOf, ShareSetting, Told, code};
use crate::server::Server;
use crate::words::Sentence;

/// The most links the shares' list holds on a store, which each server keeps in memory: past
/// it, sharing is refused, saying so.
const MOST_LINKS: usize = 100_000;

/// The most live links one person holds, so no one fills the store's room alone.
const MOST_EACH: usize = 1_000;

/// Where shared files are served: each at a name of its own under the share domain
/// (`[serve] shares`), and where reports of a shared page go (`[serve] report`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareDomain {
    pub origin: PublicOrigin,
    pub report: Option<String>,
}

impl ShareDomain {
    /// The link of the share whose id is `id`, at `port` where the domain names none
    /// (`http://localhost`, at whichever port the server answers).
    pub(crate) fn link(&self, id: &str, port: Option<u16>) -> String {
        let origin = self.origin.as_str();
        let (scheme, authority) = origin.split_once("://").unwrap_or(("https", origin));
        let named = authority.contains(':');
        match port {
            Some(port) if !named && self.origin.host() == "localhost" => {
                format!("{scheme}://{id}.{authority}:{port}/")
            }
            _ => format!("{scheme}://{id}.{authority}/"),
        }
    }

    /// What a request's `Host` names of this domain: `None` where it is not the domain nor a
    /// name under it; else the id of a share, where it is one label written as a link's id
    /// directly under the domain, or an empty id, which no share has, for any other name there,
    /// the bare domain among them. `localhost` alone, which glave's own pages may share, claims
    /// only the names under it.
    pub(crate) fn claims<'a>(&self, host: &'a str) -> Option<&'a str> {
        let host = host.trim_end_matches('.');
        let authority = self.origin.authority();
        let named = if authority.contains(':') {
            host
        } else {
            match host.rsplit_once(':') {
                Some((name, port)) if port.bytes().all(|b| b.is_ascii_digit()) => name,
                _ => host,
            }
        };
        let lower = named.to_ascii_lowercase();
        if lower == authority {
            return (self.origin.host() != "localhost").then_some("");
        }
        let under = lower.strip_suffix(authority)?.strip_suffix('.')?;
        let id = named.get(..under.len()).filter(|id| is_link_id(id));
        Some(id.unwrap_or_default())
    }
}

/// The letters a link's id is written in: RFC 4648's base32, lowercase.
const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// How long a link's id is: 128 bits, five to a letter.
const ID_LETTERS: usize = 26;

/// Whether `id` is written as a link's id is.
fn is_link_id(id: &str) -> bool {
    id.len() == ID_LETTERS && id.bytes().all(|b| ALPHABET.contains(&b))
}

/// A new link's id: 128 random bits from the operating system, in [`ALPHABET`].
fn new_link_id() -> Result<String, ApiError> {
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|error| ApiError::internal(&error))?;
    let bits = u128::from_be_bytes(random);
    Ok((0..ID_LETTERS)
        .map(|at| {
            let shift = (ID_LETTERS - 1 - at) * 5;
            let letter = usize::try_from((bits >> shift) & 31).unwrap_or_default();
            char::from(ALPHABET.get(letter).copied().unwrap_or(b'a'))
        })
        .collect())
}

/// What the shares' list keeps of a link: the SHA-256 of its id, in hex.
pub(crate) fn link_key(id: &str) -> String {
    digest(id.as_bytes()).as_str().to_owned()
}

/// Who may open a share, in a sentence.
pub(crate) fn said(audience: &Audience) -> Told {
    match audience {
        Audience::Anyone => Sentence::SharedAnyone.into(),
        Audience::People { people } => Sentence::SharedPeople {
            people: people.len() as u64,
        }
        .into(),
    }
}

/// A file the caller may share, as it is now: where it is, what it is of, and its copy.
struct Shareable {
    scope: Scope,
    from: SharedFrom,
    name: String,
    file_name: String,
    kind: glave_event::AssetKind,
    blob: glave_event::BlobRef,
    preview: Option<MadePreview>,
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The share domain, where the server has one.
    pub(crate) fn share_domain(&self) -> Result<&ShareDomain, ApiError> {
        (self.inner.shares.as_ref())
            .ok_or_else(|| ApiError::new(code::NOT_AVAILABLE, Sentence::NoShares))
    }

    /// Whether the share domain is served over HTTPS, so a proof's cookie goes only there.
    pub(crate) fn shares_securely(&self) -> bool {
        (self.inner.shares.as_ref())
            .is_some_and(|domain| domain.origin.as_str().starts_with("https://"))
    }

    /// Who besides themselves `caller` may share with here, as `server/discover` says.
    pub(crate) fn share_choices(&self, caller: &Caller) -> Vec<ShareChoice> {
        if caller.is_no_one() || self.inner.shares.is_none() {
            return Vec::new();
        }
        // Specific people prove themselves with a code glave emails them.
        let people = self.inner.mail.is_some().then_some(ShareChoice::People);
        people.into_iter().chain([ShareChoice::Anyone]).collect()
    }

    /// The live shares of `scope`'s shares ledger, each with when it was last set.
    pub(crate) async fn shares_in(
        &self,
        scope: &Scope,
    ) -> Result<BTreeMap<ShareId, (At, ShareSet)>, ApiError> {
        let mut shares = BTreeMap::new();
        let log = self.inner.harness.log();
        catch_up(log, scope, Reserved::Shares.id(), None, |page| {
            for raw in page {
                let at = raw.at;
                match decode(raw).map(|event| event.body) {
                    Ok(Body::ShareSet(set)) => {
                        shares.insert(set.share, (at, set));
                    }
                    Ok(Body::ShareStopped(stopped)) => {
                        shares.remove(&stopped.share);
                    }
                    _ => {}
                }
            }
        })
        .await?;
        Ok(shares)
    }

    /// Who each shared file of `scope` is shared with, in a sentence, by what it is of.
    pub(crate) async fn shared_sentences(
        &self,
        scope: &Scope,
    ) -> Result<HashMap<SharedFrom, Told>, ApiError> {
        let shares = self.shares_in(scope).await?;
        Ok(shares
            .into_values()
            .map(|(_, set)| (set.of, said(&set.audience)))
            .collect())
    }

    /// Answers `share/get`: `of`'s share, where the caller shares it.
    pub(crate) async fn get_share(&self, caller: &Caller, of: &ShareOf) -> Result<Reply, ApiError> {
        let shareable = self.shareable(caller, of).await?;
        let shares = self.shares_in(&shareable.scope).await?;
        let found = shares
            .into_values()
            .find(|(_, set)| set.of == shareable.from);
        Ok(match found {
            Some((at, set)) => Reply::Share {
                share: self.share_info(at, &set),
            },
            None => Reply::Done {},
        })
    }

    /// Answers `share/set`: shares the file `setting` names, changes who may open it, moves it
    /// to its newer version, or stops it.
    pub(crate) async fn set_share(
        &self,
        caller: &Caller,
        setting: ShareSetting,
    ) -> Result<Reply, ApiError> {
        self.share_domain()?;
        let mut setting = setting;
        if let Some(Audience::People { people }) = &mut setting.audience {
            if self.inner.mail.is_none() {
                return Err(ApiError::new(code::NOT_AVAILABLE, Sentence::ShareNoPeople));
            }
            *people = people::listed(people)?;
        }
        let shareable = self.shareable(caller, &setting.of).await?;
        let scope = shareable.scope;
        let shares = self.shares_in(&scope).await?;
        // Two requests racing may each have made a share of the file: the first is kept, and
        // any other stopped with it, so one Only you always stops every one.
        let mut live = (shares.into_values()).filter(|(_, set)| set.of == shareable.from);
        let current = live.next();
        let extra: Vec<ShareSet> = live.map(|(_, set)| set).collect();
        let from = shareable.from.clone();
        let Some(audience) = setting.audience else {
            self.stop_shares(&scope, |of| *of == from).await?;
            return Ok(Reply::Done {});
        };
        for set in &extra {
            self.stop_share(&scope, set).await?;
        }
        let set = match current {
            None => self.new_share(caller, shareable, audience).await?,
            Some((_, before)) if setting.update && before.blob.sha256 != shareable.blob.sha256 => {
                let set = self
                    .held_copy(&shareable, before.share, before.link.clone(), audience)
                    .await?;
                self.record_share(&scope, &set).await?;
                self.release_holder(&scope, before.holder).await?;
                set
            }
            Some((_, before)) => {
                let set = ShareSet { audience, ..before };
                self.record_share(&scope, &set).await?;
                set
            }
        };
        let now = At::from(self.inner.harness.clock().now());
        Ok(Reply::Share {
            share: self.share_info(now, &set),
        })
    }

    /// Shares `shareable` for the first time: its link listed first, so a failure after leaves
    /// a link that opens nothing, then its copy held and the share recorded.
    async fn new_share(
        &self,
        caller: &Caller,
        shareable: Shareable,
        audience: Audience,
    ) -> Result<ShareSet, ApiError> {
        let theirs = self.libraries(caller).await?;
        let theirs: Vec<Located> = theirs.iter().map(|scope| scope.located()).collect();
        let (listed, each) = self.links_held(&theirs).await?;
        if listed >= MOST_LINKS || each >= MOST_EACH {
            let most = if each >= MOST_EACH {
                MOST_EACH
            } else {
                MOST_LINKS
            };
            return Err(ApiError::new(
                code::LIMIT,
                Sentence::TooManyShares { most: most as u64 },
            ));
        }
        let harness = &self.inner.harness;
        let share = ShareId::new(harness.clock().now(), harness.entropy().bytes()?)?;
        let link = new_link_id()?;
        let key = link_key(&link);
        self.place_link(&key, shareable.scope.located()).await?;
        let made = async {
            let set = self.held_copy(&shareable, share, link, audience).await?;
            self.record_share(&shareable.scope, &set).await?;
            Ok::<_, ApiError>(set)
        };
        match made.await {
            Ok(set) => Ok(set),
            Err(error) => {
                // Unlisted again, so a share that failed counts against nothing.
                if self.unplace_link(&key).await.is_err() {
                    tracing::warn!("a link a failed share listed stays listed, opening nothing");
                }
                Err(error)
            }
        }
    }

    /// Stops every live share in `scope` whose file `stops` picks: a chat deleted, an item
    /// taken out of the Library, or Only you.
    pub(crate) async fn stop_shares(
        &self,
        scope: &Scope,
        stops: impl Fn(&SharedFrom) -> bool,
    ) -> Result<(), ApiError> {
        let shares = self.shares_in(scope).await?;
        for (_, set) in shares.into_values().filter(|(_, set)| stops(&set.of)) {
            self.stop_share(scope, &set).await?;
        }
        Ok(())
    }

    /// Answers `thread/delete`: deletes it, then stops the shares of every file it made.
    pub(crate) async fn delete_thread(
        &self,
        caller: &Caller,
        thread: ThreadId,
    ) -> Result<Reply, ApiError> {
        let reply = self
            .change(caller, thread, glave_harness::Change::Delete)
            .await?;
        let scope = self.scope_of(thread)?;
        let made_here =
            |of: &SharedFrom| matches!(of, SharedFrom::File { thread: at, .. } if *at == thread);
        self.stop_shares(&scope, made_here).await?;
        Ok(reply)
    }

    /// `share` of `shareable` as it is now, its copy and preview held under a new holder.
    async fn held_copy(
        &self,
        shareable: &Shareable,
        share: ShareId,
        link: String,
        audience: Audience,
    ) -> Result<ShareSet, ApiError> {
        let harness = &self.inner.harness;
        let holder = ThreadId::new(harness.clock().now(), harness.entropy().bytes()?)?;
        let preview = shareable.preview.as_ref().map(|preview| &preview.blob);
        for blob in std::iter::once(&shareable.blob).chain(preview) {
            let held = harness
                .blobs()
                .refer(&shareable.scope, &blob.sha256, holder)
                .await;
            if !held.map_err(|error| ApiError::internal(&error))? {
                return Err(not_here());
            }
        }
        Ok(ShareSet {
            share,
            link,
            of: shareable.from.clone(),
            name: shareable.name.clone(),
            file_name: shareable.file_name.clone(),
            kind: shareable.kind,
            blob: shareable.blob.clone(),
            preview: shareable.preview.clone(),
            holder,
            audience,
        })
    }

    /// Stops `set`: its copy let go of first, then the share recorded stopped, so its link
    /// opens nothing, then the link unlisted. A failure leaves the share live, and stopping it
    /// again finishes.
    async fn stop_share(&self, scope: &Scope, set: &ShareSet) -> Result<(), ApiError> {
        self.release_holder(scope, set.holder).await?;
        let stopped = ShareStopped { share: set.share };
        self.keep_latest(scope, Reserved::Shares.id(), &SHARES, &stopped)
            .await?;
        self.unplace_link(&link_key(&set.link)).await
    }

    async fn record_share(&self, scope: &Scope, set: &ShareSet) -> Result<(), ApiError> {
        self.keep_latest(scope, Reserved::Shares.id(), &SHARES, set)
            .await
    }

    async fn release_holder(&self, scope: &Scope, holder: ThreadId) -> Result<(), ApiError> {
        let released = self.inner.harness.blobs().release(scope, holder).await;
        released.map_err(|error| ApiError::internal(&error))
    }

    /// `set` as a client is told of it, last set `at`.
    fn share_info(&self, at: At, set: &ShareSet) -> ShareInfo {
        let port = self.inner.reached.get().and_then(|reached| {
            let at = reached.at.as_deref()?;
            at.rsplit_once(':')?.1.parse().ok()
        });
        let link = (self.inner.shares.as_ref())
            .map(|domain| domain.link(&set.link, port))
            .unwrap_or_default();
        ShareInfo {
            audience: set.audience.clone(),
            link,
            shared_at: at,
            sha256: set.blob.sha256.clone(),
            said: said(&set.audience),
        }
    }

    /// The file `of` names, when the caller may share it: a file one of their chat's turns made
    /// and kept, or an item of their Library. Anyone else's is "no longer here".
    async fn shareable(&self, caller: &Caller, of: &ShareOf) -> Result<Shareable, ApiError> {
        match of {
            ShareOf::File { thread, sha256 } => {
                let scope = self.scope_of(*thread)?;
                let (made, file) = self.made_kept(*thread, sha256).await?;
                let glave_event::Kept::Blob(blob) = made.kept else {
                    return Err(not_here());
                };
                Ok(Shareable {
                    scope,
                    from: SharedFrom::File {
                        thread: *thread,
                        path: made.path,
                    },
                    name: file.name,
                    file_name: file.file_name,
                    kind: file.kind,
                    blob,
                    preview: made.read.and_then(|read| read.preview),
                })
            }
            ShareOf::Item { project, asset } => {
                let (scope, kept) = self.item(caller, *project, *asset).await?;
                Ok(Shareable {
                    scope,
                    from: SharedFrom::Item { asset: kept.asset },
                    name: kept.name,
                    file_name: kept.file_name,
                    kind: kept.kind,
                    blob: kept.blob,
                    preview: None,
                })
            }
        }
    }

    /// The live share whose link's id is `id`, and its scope: found through the shares' list,
    /// then the ledger of the project that holds it, for serving its link to anyone.
    pub(crate) async fn share_at(&self, id: &str) -> Result<Option<(Scope, ShareSet)>, ApiError> {
        let Some(Located { org, project }) = self.find_link(&link_key(id)).await? else {
            return Ok(None);
        };
        // glave's own: serving a link reads the one share it names, for whoever has it.
        let scope = Scope::glaves_own(org, project);
        // A project removed, or one whose records can no longer be opened (erased), holds no
        // live share: its links open nothing, as one never made.
        let at = Located { org, project };
        let readable = match self.removed(at).await {
            Ok(removed) => (!removed).then_some(()),
            Err(_) => None,
        };
        let shares = match readable {
            Some(()) => self.shares_in(&scope).await.unwrap_or_default(),
            None => BTreeMap::new(),
        };
        let found = shares.into_values().find(|(_, set)| set.link == id);
        Ok(found.map(|(_, set)| (scope, set)))
    }
}

/// Every kind the shares ledger holds, which each writer numbers its event after.
const SHARES: [&str; 4] = [
    ShareSet::TYPE,
    ShareStopped::TYPE,
    glave_event::ShareCodeSent::TYPE,
    glave_event::ShareOpened::TYPE,
];

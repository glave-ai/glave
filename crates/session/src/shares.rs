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
pub(crate) use self::people::{Ledgers, PROOF_FOR};

use std::collections::{BTreeMap, HashMap};

use glave_event::{
    At, Audience, MadePreview, Payload as _, Principal, PrincipalId, ShareId, ShareSet,
    ShareStopped, SharedFrom, ThreadId,
};
use glave_ports::host::PublicOrigin;
use glave_ports::stream::Reserved;
use glave_ports::{BlobStore, Caller, EventLog, Located, ModelProvider, OrgId, Scope, digest};

use crate::files::not_here;
use crate::projects::MOST_PROJECTS;
use crate::protocol::{ApiError, Reply, ShareChoice, ShareInfo, ShareOf, ShareSetting, Told, code};
use crate::server::Server;
use crate::words::Sentence;

/// The most live links one person holds, in every organization together.
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
    /// The port is split off before a name's final dot is, so `id.domain.:443` is the domain's
    /// as `id.domain:443` is; and a name under the domain at another port than one it names is
    /// claimed too, opening nothing, so no name there ever reaches glave's own pages.
    pub(crate) fn claims<'a>(&self, host: &'a str) -> Option<&'a str> {
        let (name, port) = match host.rsplit_once(':') {
            Some((name, port)) if port.bytes().all(|b| b.is_ascii_digit()) => (name, Some(port)),
            _ => (host, None),
        };
        let name = name.trim_end_matches('.');
        let domain = self.origin.host();
        let lower = name.to_ascii_lowercase();
        let id = if lower == domain {
            if domain == "localhost" {
                return None;
            }
            ""
        } else {
            let under = lower.strip_suffix(domain)?.strip_suffix('.')?;
            name.get(..under.len())
                .filter(|id| is_link_id(id))
                .unwrap_or_default()
        };
        let wanted = (self.origin.authority().strip_prefix(domain))
            .and_then(|named| named.strip_prefix(':'));
        match wanted {
            Some(wanted) if port != Some(wanted) => Some(""),
            _ => Some(id),
        }
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
    /// The person sharing it, who owns its project.
    owner: PrincipalId,
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

    /// The live shares of `scope`'s shares ledger, each with when it was last set, read on from
    /// where this server last read it.
    pub(crate) async fn shares_in(
        &self,
        scope: &Scope,
    ) -> Result<BTreeMap<ShareId, (At, ShareSet)>, ApiError> {
        let ledger = self.share_ledger(scope).await?;
        Ok((ledger.live.into_iter())
            .map(|(share, live)| (share, (live.at, live.set)))
            .collect())
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
            None => self.new_share(shareable, audience).await?,
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
        shareable: Shareable,
        audience: Audience,
    ) -> Result<ShareSet, ApiError> {
        let harness = &self.inner.harness;
        let share = ShareId::new(harness.clock().now(), harness.entropy().bytes()?)?;
        let link = new_link_id()?;
        let key = link_key(&link);
        let owner = shareable.owner;
        // Counted once listed, so two made at once on any servers both count: past the limit,
        // each is unlisted again and refused.
        let held = self.place_link(&key, shareable.scope.located(), owner);
        let made = async {
            if held.await? > MOST_EACH {
                return Err(ApiError::new(
                    code::LIMIT,
                    Sentence::TooManyShares {
                        most: MOST_EACH as u64,
                    },
                ));
            }
            let set = self.held_copy(&shareable, share, link, audience).await?;
            self.record_share(&shareable.scope, &set).await?;
            Ok::<_, ApiError>(set)
        };
        match made.await {
            Ok(set) => Ok(set),
            Err(error) => {
                // Unlisted again, so a share that failed counts against nothing.
                if self.unplace_link(&key, owner).await.is_err() {
                    tracing::warn!("a link a failed share listed stays listed, opening nothing");
                }
                Err(error)
            }
        }
    }

    /// Stops the shares of every project `person` owns in `org`, as they are taken out of it.
    pub(crate) async fn stop_shares_of(
        &self,
        org: OrgId,
        person: PrincipalId,
    ) -> Result<(), ApiError> {
        let owner = Principal::person(person);
        for project in self.inner.projects.list(org, &owner, MOST_PROJECTS).await? {
            // glave's own: what they shared stops, whoever takes them out.
            let scope = Scope::glaves_own(org, project.id);
            self.stop_shares(&scope, |_| true).await?;
        }
        Ok(())
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
        // Its shares stop first: a failure after leaves the chat, deleted again to finish, and
        // never a share of a chat already gone.
        let scope = self.scope_of(thread)?;
        let made_here =
            |of: &SharedFrom| matches!(of, SharedFrom::File { thread: at, .. } if *at == thread);
        self.stop_shares(&scope, made_here).await?;
        self.change(caller, thread, glave_harness::Change::Delete)
            .await
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
            owner: shareable.owner,
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
        self.unplace_link(&link_key(&set.link), set.owner).await
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
        let owner = self.person_of(caller)?;
        match of {
            ShareOf::File { thread, sha256 } => {
                let scope = self.scope_of(*thread)?;
                let (made, file) = self.made_kept(*thread, sha256).await?;
                let glave_event::Kept::Blob(blob) = made.kept else {
                    return Err(not_here());
                };
                Ok(Shareable {
                    scope,
                    owner,
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
                    owner,
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
        if !is_link_id(id) {
            return Ok(None);
        }
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
        let live = match readable {
            Some(()) => (self.share_ledger(&scope).await)
                .map(|ledger| ledger.live)
                .unwrap_or_default(),
            None => BTreeMap::new(),
        };
        let Some(set) = (live.into_values().map(|live| live.set)).find(|set| set.link == id) else {
            return Ok(None);
        };
        // Someone taken out of the organization shares nothing there any more, even before
        // their shares are stopped; a people ledger that can't be read opens nothing.
        let member = matches!(self.removed_from(org, set.owner).await, Ok(false));
        Ok(member.then_some((scope, set)))
    }
}

/// Every kind the shares ledger holds, which each writer numbers its event after.
const SHARES: [&str; 4] = [
    ShareSet::TYPE,
    ShareStopped::TYPE,
    glave_event::ShareLinkSent::TYPE,
    glave_event::ShareOpened::TYPE,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn at(origin: &str) -> ShareDomain {
        let origin = PublicOrigin::parse(origin).unwrap();
        ShareDomain {
            origin,
            report: None,
        }
    }

    #[test]
    fn every_name_under_the_share_domain_is_claimed_however_it_is_written() {
        let id = "abcdefghijklmnopqrstuvwxyz";
        let domain = at("https://share.example.net");
        for host in [
            format!("{id}.share.example.net"),
            format!("{id}.share.example.net."),
            format!("{id}.share.example.net:443"),
            format!("{id}.share.example.net.:8443"),
            format!("{id}.SHARE.example.net"),
        ] {
            assert_eq!(domain.claims(&host), Some(id), "{host}");
        }
        for host in [
            "share.example.net.:8443",
            "login.share.example.net.:1",
            "x.share.example.net.",
        ] {
            assert_eq!(domain.claims(host), Some(""), "{host}");
        }
        for host in [
            "glave.example.net",
            "share.example.net.evil.test",
            "example.net:443",
        ] {
            assert_eq!(domain.claims(host), None, "{host}");
        }
        // A domain that names its port claims every name under it, at any port, opening a link
        // only at its own.
        let ported = at("https://share.example.org:8443");
        let at_its_port = format!("{id}.share.example.org.:8443");
        assert_eq!(ported.claims(&at_its_port), Some(id));
        let elsewhere = format!("{id}.share.example.org:9000");
        assert_eq!(ported.claims(&elsewhere), Some(""));
        // Bare `localhost` is glave's own; the names under it are shares'.
        let here = at("http://localhost");
        assert_eq!(here.claims("localhost:8080"), None);
        assert_eq!(here.claims("localhost.:8080"), None);
        assert_eq!(here.claims(&format!("{id}.localhost:8080")), Some(id));
    }
}

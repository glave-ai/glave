//! A person's push notifications (`specs/session.md`, "Push notifications"), as Settings reads
//! and changes them: the browsers that take theirs, each kept in their personal project's push
//! ledger sealed under a key of its own and erased when it is forgotten, at most
//! [`MOST_DEVICES`]; whose browser each endpoint is, so one browser is never two people's; and
//! the one key the store's servers sign every push with, made the first time one is needed and
//! kept sealed in the server's own organization's push ledger, where the first one recorded holds.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use glave_event::{
    At, Body, CredentialId, EventId, Header, Payload, Principal, PrincipalId, PushClaimed,
    PushKeyMade, PushSubscribed, PushUnsubscribed, Seq, ThreadId, decode, encode,
};
use glave_ports::stream::{Reserved, append_retrying, catch_up};
use glave_ports::{
    BlobStore, Caller, Erasable, Eraser, EventLog, ModelProvider, ProjectKind, Scope,
};
use glave_push::{Message, PushError, Pusher, Sent, VapidKey};
use sha2::{Digest as _, Sha256};

use crate::protocol::{ApiError, PushDevice, PushInfo, PushSubscription, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

/// The most browsers one person's notifications go to; one more forgets the oldest.
const MOST_DEVICES: usize = 16;

/// The longest name a browser is kept under, in characters.
const LONGEST_NAME: usize = 80;

/// The push ledger's events in a personal project, each numbered after the latest of them.
const KEPT: [&str; 2] = [PushSubscribed::TYPE, PushUnsubscribed::TYPE];

/// How many pushes in a row may fail before a browser is forgotten as if it were gone.
const FAILURES: u8 = 3;

/// The browsers whose last pushes failed that are counted, past which the oldest count goes.
const MOST_COUNTED: usize = 4_096;

/// The most of a person's projects looked through for their personal one, which is made first.
const PROJECTS_LOOKED: usize = 64;

/// What pushes a server's notifications: the browsers' push services, through `pusher`, and the
/// keys of forgotten browsers, destroyed through `eraser`.
pub struct Pushing {
    pusher: Pusher,
    eraser: Arc<dyn Eraser>,
    failing: Failing,
}

impl Pushing {
    /// Pushes through `pusher`, erasing forgotten browsers through `eraser`.
    pub fn new(pusher: Pusher, eraser: Arc<dyn Eraser>) -> Self {
        Self {
            pusher,
            eraser,
            failing: Failing::default(),
        }
    }

    /// Sends `message` to `device`: whether its push service took it, or it is to be forgotten,
    /// gone or failing [`FAILURES`] times in a row.
    pub(crate) async fn send(
        &self,
        key: &VapidKey,
        device: &PushSubscribed,
        message: Message<'_>,
        now: jiff::Timestamp,
    ) -> Result<Sent, PushError> {
        let sent =
            match self
                .pusher
                .subscription(&device.endpoint, &device.p256dh, device.auth.expose())
            {
                Ok(to) => self.pusher.send(key, &to, message, now).await,
                // Kept before glave read subscriptions as it does now: no push reaches it.
                Err(_) => Ok(Sent::Gone),
            };
        self.failing.counted(device.device, sent)
    }
}

/// Each browser's pushes that failed in a row, while it is under [`FAILURES`], for at most
/// [`MOST_COUNTED`] browsers.
#[derive(Default)]
struct Failing(Mutex<BTreeMap<CredentialId, u8>>);

impl Failing {
    /// `sent`, a push to `device`, counted: gone once it is the [`FAILURES`]th failure in a row.
    fn counted(
        &self,
        device: CredentialId,
        sent: Result<Sent, PushError>,
    ) -> Result<Sent, PushError> {
        let mut failing = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if sent.is_ok() {
            failing.remove(&device);
            return sent;
        }
        let count = failing.entry(device).or_default();
        *count = count.saturating_add(1);
        if *count >= FAILURES {
            failing.remove(&device);
            return Ok(Sent::Gone);
        }
        if failing.len() > MOST_COUNTED {
            failing.pop_first();
        }
        sent
    }
}

/// A browser that takes the person's notifications, as its `push.subscribed` keeps it.
pub(crate) struct Device {
    pub(crate) added: At,
    pub(crate) subscribed: PushSubscribed,
}

/// The stream that says whose browser `endpoint` is, named by its hash, never the endpoint.
fn claim_of(endpoint: &str) -> Result<ThreadId, ApiError> {
    let digest: [u8; 32] = Sha256::digest(endpoint.as_bytes()).into();
    let random = digest.first_chunk::<10>().copied().unwrap_or_default();
    Ok(ThreadId::new(jiff::Timestamp::UNIX_EPOCH, random)?)
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// What pushes this server's notifications; refused where it pushes none.
    fn pushing(&self) -> Result<&Pushing, ApiError> {
        self.inner
            .push
            .as_deref()
            .ok_or_else(|| ApiError::new(code::NOT_AVAILABLE, Sentence::NoPush))
    }

    /// Answers `push/get`: never, with why, where the server pushes none.
    pub(crate) async fn push_info(
        &self,
        caller: &Caller,
        endpoint: Option<&str>,
    ) -> Result<Reply, ApiError> {
        if self.inner.push.is_none() {
            return Ok(Reply::Push {
                push: PushInfo::Never {
                    reason: Sentence::NoPush.into(),
                },
            });
        }
        self.offered(caller, endpoint).await
    }

    /// Answers `push/subscribe`: the browser kept as the caller's, unless kept already as it is
    /// now, taken from anyone whose it was, and the oldest forgotten past the most kept.
    pub(crate) async fn subscribe(
        &self,
        caller: &Caller,
        browser: PushSubscription,
    ) -> Result<Reply, ApiError> {
        let PushSubscription {
            endpoint,
            p256dh,
            auth,
            device: name,
        } = browser;
        self.pushing()?
            .pusher
            .subscription(&endpoint, &p256dh, auth.expose())
            .map_err(|refusal| {
                let reason = refusal.to_string();
                ApiError::new(code::INVALID_PARAMS, Sentence::PushRefused { reason })
            })?;
        let name = name.trim();
        if name.is_empty() || name.chars().count() > LONGEST_NAME {
            return Err(ApiError::new(code::INVALID_PARAMS, Sentence::DeviceUnnamed));
        }
        let person = self.person_of(caller)?;
        let (_, personal) = self.personal_project(caller).await?;
        let kept = self.devices(&personal, person).await?;
        let same = kept
            .iter()
            .find(|kept| kept.subscribed.endpoint == endpoint);
        if let Some(same) = same.filter(|same| {
            same.subscribed.p256dh == p256dh
                && same.subscribed.auth == auth
                && same.subscribed.name == name
        }) {
            let device = same.subscribed.device;
            self.claim(&endpoint, person, device).await?;
            return self.offered(caller, Some(&endpoint)).await;
        }
        let mut others: Vec<&Device> = kept
            .iter()
            .filter(|other| other.subscribed.endpoint != endpoint)
            .collect();
        others.sort_by_key(|other| other.added);
        let over = (others.len() + 1).saturating_sub(MOST_DEVICES);
        let forgotten = same.into_iter().chain(others.into_iter().take(over));
        for old in forgotten
            .map(|old| old.subscribed.device)
            .collect::<Vec<_>>()
        {
            self.forget_device(&personal, old, false).await?;
        }
        let now = self.inner.harness.clock().now();
        let device = CredentialId::new(now, self.inner.harness.entropy().bytes()?)?;
        let subscribed = PushSubscribed {
            device,
            endpoint,
            p256dh,
            auth,
            name: name.to_owned(),
        };
        self.keep_latest(&personal, Reserved::Push.id(), &KEPT, &subscribed)
            .await?;
        self.claim(&subscribed.endpoint, person, device).await?;
        self.offered(caller, Some(&subscribed.endpoint)).await
    }

    /// Answers `push/remove`.
    pub(crate) async fn remove_device(
        &self,
        caller: &Caller,
        device: CredentialId,
    ) -> Result<Reply, ApiError> {
        self.pushing()?;
        let person = self.person_of(caller)?;
        if let Some(personal) = self.personal_if_any(caller).await? {
            let kept = self.devices(&personal, person).await?;
            if kept.iter().any(|kept| kept.subscribed.device == device) {
                self.forget_device(&personal, device, false).await?;
            }
        }
        self.offered(caller, None).await
    }

    /// What `push/get` says where the server pushes: the key to subscribe with and the caller's
    /// browsers, the latest first, `endpoint`'s current.
    async fn offered(&self, caller: &Caller, endpoint: Option<&str>) -> Result<Reply, ApiError> {
        let key = self.push_key().await?;
        let person = self.person_of(caller)?;
        let kept = match self.personal_if_any(caller).await? {
            Some(personal) => self.devices(&personal, person).await?,
            None => Vec::new(),
        };
        let mut devices: Vec<PushDevice> = kept
            .into_iter()
            .map(|device| PushDevice {
                id: device.subscribed.device,
                current: endpoint == Some(device.subscribed.endpoint.as_str()),
                name: device.subscribed.name,
                added: device.added,
            })
            .collect();
        devices.sort_by_key(|device| std::cmp::Reverse(device.added));
        Ok(Reply::Push {
            push: PushInfo::Offered {
                key: key.public(),
                devices,
            },
        })
    }

    /// `caller`'s personal project, when they have one: never made here, so reading what they
    /// keep leaves nothing behind.
    pub(crate) async fn personal_if_any(&self, caller: &Caller) -> Result<Option<Scope>, ApiError> {
        let (org, person) = (self.org_of(caller), self.person_of(caller)?);
        let owner = Principal::person(person);
        let projects = self
            .inner
            .projects
            .list(org, &owner, PROJECTS_LOOKED)
            .await?;
        Ok(projects
            .iter()
            .find(|project| project.kind == ProjectKind::Personal)
            .and_then(|project| Scope::of_owner(org, project, person)))
    }

    /// The browsers that take `person`'s notifications, kept in their personal project
    /// `personal`: those whose endpoints are still theirs, the rest forgotten as found, and any
    /// forgotten whose key still opens, as after a crash, erased now.
    pub(crate) async fn devices(
        &self,
        personal: &Scope,
        person: PrincipalId,
    ) -> Result<Vec<Device>, ApiError> {
        let mut kept: BTreeMap<CredentialId, Device> = BTreeMap::new();
        let mut forgotten = Vec::new();
        let log = self.inner.harness.log();
        catch_up(log, personal, Reserved::Push.id(), None, |page| {
            for raw in page {
                let Ok(event) = decode(raw) else {
                    continue;
                };
                match event.body {
                    Body::PushSubscribed(subscribed) => {
                        let added = event.header.at;
                        kept.insert(subscribed.device, Device { added, subscribed });
                    }
                    Body::PushUnsubscribed(PushUnsubscribed { device, .. }) => {
                        forgotten.extend(kept.remove(&device).map(|_| device));
                    }
                    _ => {}
                }
            }
        })
        .await?;
        for device in forgotten {
            self.erase_device(personal, device).await?;
        }
        let mut theirs = Vec::with_capacity(kept.len());
        for device in kept.into_values() {
            let claimant = self.claimant(&device.subscribed.endpoint).await?;
            if claimant.is_none_or(|claimant| claimant.person == person) {
                theirs.push(device);
            } else {
                // Subscribed since by someone else, whose browser it is now.
                self.forget_device(personal, device.subscribed.device, false)
                    .await?;
            }
        }
        Ok(theirs)
    }

    /// Forgets the browser `device`, erasing it: its person turned it off or it moved to
    /// someone else, or its push service said it is `gone` or kept failing.
    pub(crate) async fn forget_device(
        &self,
        personal: &Scope,
        device: CredentialId,
        gone: bool,
    ) -> Result<(), ApiError> {
        let unsubscribed = PushUnsubscribed { device, gone };
        self.keep_latest(personal, Reserved::Push.id(), &KEPT, &unsubscribed)
            .await?;
        self.erase_device(personal, device).await
    }

    /// Destroys the key `device`'s subscription is sealed under, so nothing reads it again.
    async fn erase_device(&self, personal: &Scope, device: CredentialId) -> Result<(), ApiError> {
        self.pushing()?
            .eraser
            .erase(*personal, Erasable::Account(device.as_uuid()))
            .await
            .map_err(|error| ApiError::internal(&error))
    }

    /// Keeps `endpoint` as `person`'s, as their `device`, unless it is already.
    async fn claim(
        &self,
        endpoint: &str,
        person: PrincipalId,
        device: CredentialId,
    ) -> Result<(), ApiError> {
        let claimed = PushClaimed { person, device };
        if self.claimant(endpoint).await?.as_ref() == Some(&claimed) {
            return Ok(());
        }
        let scope = Scope::of_organization(self.inner.own);
        self.keep_latest(&scope, claim_of(endpoint)?, &[PushClaimed::TYPE], &claimed)
            .await
    }

    /// Whose browser `endpoint` is now, if anyone has said.
    async fn claimant(&self, endpoint: &str) -> Result<Option<PushClaimed>, ApiError> {
        let scope = Scope::of_organization(self.inner.own);
        let log = self.inner.harness.log();
        let latest = log
            .latest_of(&scope, claim_of(endpoint)?, &[PushClaimed::TYPE])
            .await?;
        Ok(
            match latest.map(decode).transpose()?.map(|event| event.body) {
                Some(Body::PushClaimed(claimed)) => Some(claimed),
                _ => None,
            },
        )
    }

    /// The key every push on the store is signed with: the one the server's own organization's
    /// push ledger holds, else one made now and kept there, unless another server kept one
    /// first, which is then the one.
    pub(crate) async fn push_key(&self) -> Result<VapidKey, ApiError> {
        let scope = &Scope::of_organization(self.inner.own);
        let ledger = Reserved::Push.id();
        let log = self.inner.harness.log();
        let this = self;
        let made = append_retrying(log, scope, ledger, move || async move {
            if !log.read(scope, ledger, None, 1).await?.is_empty() {
                return Ok(None);
            }
            let key = VapidKey::make().map_err(|error| ApiError::internal(&error))?;
            let now = this.inner.harness.clock().now();
            let header = Header {
                id: EventId::new(now, this.inner.harness.entropy().bytes()?)?,
                thread: ledger,
                seq: Seq::FIRST,
                at: At::from(now),
            };
            let made = PushKeyMade { key: key.secret() };
            Ok::<_, ApiError>(Some((vec![encode(header, &made)?], key)))
        })
        .await?;
        if let Some((_, key)) = made {
            return Ok(key);
        }
        let first = log.read(scope, ledger, None, 1).await?.into_iter().next();
        match first.map(decode).transpose()?.map(|event| event.body) {
            Some(Body::PushKeyMade(made)) => {
                VapidKey::read(&made.key).map_err(|error| ApiError::internal(&error))
            }
            _ => Err(ApiError::internal(&"the push ledger begins with no key")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(n: u8) -> CredentialId {
        CredentialId::new(jiff::Timestamp::UNIX_EPOCH, [n; 10]).unwrap()
    }

    #[test]
    fn a_browser_failing_three_times_in_a_row_is_gone() {
        let failing = Failing::default();
        let (one, other) = (device(1), device(2));
        for _ in 1..FAILURES {
            assert!(failing.counted(one, Err(PushError::TimedOut)).is_err());
        }
        assert!(
            failing.counted(other, Err(PushError::TimedOut)).is_err(),
            "counted apart"
        );
        assert!(matches!(
            failing.counted(one, Err(PushError::TimedOut)),
            Ok(Sent::Gone)
        ));
        // A push taken starts the count again.
        assert!(failing.counted(other, Ok(Sent::Taken)).is_ok());
        for _ in 1..FAILURES {
            assert!(failing.counted(other, Err(PushError::TimedOut)).is_err());
        }
    }
}

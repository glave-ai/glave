//! The browsers that take a person's notifications, each kept in their personal project's push
//! ledger sealed under a key of its own and erased when it is forgotten: one for each endpoint,
//! only while the browser session it was subscribed in lasts, and only while the endpoint is
//! still theirs. Whose browser each endpoint is, in a stream named by its hash in the server's
//! own organization's scope, sealed under the browser's key too, so one browser is never two
//! people's and forgetting it erases both.

use std::collections::BTreeMap;

use glave_event::{
    At, Body, DeviceId, Payload, PrincipalId, PushClaimed, PushSubscribed, PushUnsubscribed,
    ThreadId, decode,
};
use glave_identity::signin::SESSION_LIFETIME;
use glave_ports::stream::{Reserved, catch_up};
use glave_ports::{BlobStore, Erasable, EventLog, ModelProvider, Scope};
use jiff::Timestamp;
use sha2::{Digest as _, Sha256};

use crate::protocol::ApiError;
use crate::server::Server;

/// The push ledger's events in a personal project, each numbered after the latest of them.
pub(super) const KEPT: [&str; 2] = [PushSubscribed::TYPE, PushUnsubscribed::TYPE];

/// A browser that takes the person's notifications, as its `push.subscribed` keeps it: when it
/// was first added, and as it was last subscribed.
pub(crate) struct Device {
    pub(crate) added: At,
    pub(crate) subscribed: PushSubscribed,
    /// When it was last subscribed, the same subscription kept again renewing it.
    written: At,
}

impl Device {
    /// When it lapses unless its browser subscribes it again first: as long after it was last
    /// subscribed as a glave browser session lasts after it was last used.
    pub(crate) fn lapses(&self) -> Timestamp {
        let written = self.written.timestamp();
        written
            .saturating_add(SESSION_LIFETIME)
            .unwrap_or(Timestamp::MAX)
    }

    /// Whether its browser should subscribe it again now: less than half of its time is left.
    pub(crate) fn renew(&self, now: Timestamp) -> bool {
        self.lapses().duration_since(now) < SESSION_LIFETIME / 2
    }
}

/// What a person's push ledger holds now, read, nothing written: their browsers, and what it
/// keeps that is theirs no more, which only work that may write forgets.
struct Held {
    theirs: Vec<Device>,
    /// Kept twice, moved to someone else, signed out of, or lapsed: to forget.
    stale: Vec<DeviceId>,
    /// Forgotten, but their keys still open, as after a crash: to erase.
    unerased: Vec<DeviceId>,
}

/// The stream that says whose browser `endpoint` is, named by its hash, never the endpoint.
fn claim_of(endpoint: &str) -> Result<ThreadId, ApiError> {
    let digest: [u8; 32] = Sha256::digest(endpoint.as_bytes()).into();
    let random = digest.first_chunk::<10>().copied().unwrap_or_default();
    Ok(ThreadId::new(Timestamp::UNIX_EPOCH, random)?)
}

/// The browsers `kept` holds, one for each endpoint, the latest subscribed; and those it holds
/// twice, as two tabs subscribing at once leave them.
fn by_endpoint(kept: BTreeMap<DeviceId, Device>) -> (Vec<Device>, Vec<DeviceId>) {
    let mut latest: BTreeMap<String, Device> = BTreeMap::new();
    let mut twice = Vec::new();
    for device in kept.into_values() {
        let endpoint = device.subscribed.endpoint.clone();
        match latest.remove(&endpoint) {
            Some(other) if other.added > device.added => {
                twice.push(device.subscribed.device);
                latest.insert(endpoint, other);
            }
            Some(other) => {
                twice.push(other.subscribed.device);
                latest.insert(endpoint, device);
            }
            None => {
                latest.insert(endpoint, device);
            }
        }
    }
    (latest.into_values().collect(), twice)
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The browsers that take `person`'s notifications, kept in their personal project
    /// `personal`: one for each endpoint, still theirs, in a session that has not ended, not
    /// lapsed. The rest are forgotten as found, and any forgotten whose key still opens, as
    /// after a crash, erased now: for a push, and for a change of the person's, never for a read.
    pub(crate) async fn devices(
        &self,
        personal: &Scope,
        person: PrincipalId,
    ) -> Result<Vec<Device>, ApiError> {
        let held = self.held(personal, person).await?;
        for device in held.unerased {
            self.erase_device(personal, device).await?;
        }
        for device in held.stale {
            self.forget_device(personal, device, false).await?;
        }
        Ok(held.theirs)
    }

    /// The browsers that take `person`'s notifications, as [`Server::devices`] finds them, read
    /// alone (`specs/invariants.md`, 5): what is theirs no more is left out, and left for the
    /// next push or change to forget.
    pub(crate) async fn devices_read(
        &self,
        personal: &Scope,
        person: PrincipalId,
    ) -> Result<Vec<Device>, ApiError> {
        Ok(self.held(personal, person).await?.theirs)
    }

    /// What `person`'s push ledger in `personal` holds now, nothing written.
    async fn held(&self, personal: &Scope, person: PrincipalId) -> Result<Held, ApiError> {
        let mut kept: BTreeMap<DeviceId, Device> = BTreeMap::new();
        let mut unerased = Vec::new();
        let log = self.inner.harness.log();
        catch_up(log, personal, Reserved::Push.id(), None, |page| {
            for raw in page {
                let Ok(event) = decode(raw) else {
                    continue;
                };
                match event.body {
                    Body::PushSubscribed(subscribed) => {
                        let written = event.header.at;
                        // Subscribed again, as renewing it does: added when it first was.
                        let added = kept.get(&subscribed.device).map_or(written, |it| it.added);
                        let device = subscribed.device;
                        let kept_now = Device {
                            added,
                            subscribed,
                            written,
                        };
                        kept.insert(device, kept_now);
                    }
                    Body::PushUnsubscribed(PushUnsubscribed { device, .. }) => {
                        unerased.extend(kept.remove(&device).map(|_| device));
                    }
                    _ => {}
                }
            }
        })
        .await?;
        let (kept, mut stale) = by_endpoint(kept);
        let mut theirs = Vec::with_capacity(kept.len());
        let now = self.inner.harness.clock().now();
        for device in kept {
            let claimant = self.claimant(&device.subscribed.endpoint).await?;
            let moved = claimant.is_some_and(|claimant| claimant.person != person);
            let ended = match &device.subscribed.session {
                Some(session) => self.session_ended(person, session).await?,
                None => false,
            };
            let lapsed = device.lapses() <= now;
            if moved || ended || lapsed {
                // Subscribed since by someone else, signed out of where it was subscribed, or
                // not subscribed again in time.
                stale.push(device.subscribed.device);
            } else {
                theirs.push(device);
            }
        }
        Ok(Held {
            theirs,
            stale,
            unerased,
        })
    }

    /// Forgets the browser `device`, erasing it: its person turned it off or signed out there,
    /// it moved to someone else, or its push service said it is `gone` or kept refusing it.
    pub(crate) async fn forget_device(
        &self,
        personal: &Scope,
        device: DeviceId,
        gone: bool,
    ) -> Result<(), ApiError> {
        let unsubscribed = PushUnsubscribed { device, gone };
        self.keep_latest(personal, Reserved::Push.id(), &KEPT, &unsubscribed)
            .await?;
        self.erase_device(personal, device).await
    }

    /// Destroys the key `device`'s subscription is sealed under in `personal`, and its claim's
    /// in the server's own organization, so nothing reads either again.
    async fn erase_device(&self, personal: &Scope, device: DeviceId) -> Result<(), ApiError> {
        let pushing = self.pushing()?;
        let erased = Erasable::Account(device.as_uuid());
        for scope in [*personal, Scope::of_organization(self.inner.own)] {
            pushing
                .eraser
                .erase(scope, erased)
                .await
                .map_err(|error| ApiError::internal(&error))?;
        }
        pushing.failing.forgot(device);
        Ok(())
    }

    /// Keeps `endpoint` as `person`'s, as their `device`, unless it is already.
    pub(super) async fn claim(
        &self,
        endpoint: &str,
        person: PrincipalId,
        device: DeviceId,
    ) -> Result<(), ApiError> {
        let claimed = PushClaimed { person, device };
        if self.claimant(endpoint).await?.as_ref() == Some(&claimed) {
            return Ok(());
        }
        let scope = Scope::of_organization(self.inner.own);
        self.keep_latest(&scope, claim_of(endpoint)?, &[PushClaimed::TYPE], &claimed)
            .await
    }

    /// Whose browser `endpoint` is now, if anyone's: none once its claim was erased.
    async fn claimant(&self, endpoint: &str) -> Result<Option<PushClaimed>, ApiError> {
        let scope = Scope::of_organization(self.inner.own);
        let log = self.inner.harness.log();
        let latest = log
            .latest_of(&scope, claim_of(endpoint)?, &[PushClaimed::TYPE])
            .await?;
        Ok(
            match latest
                .and_then(|raw| decode(raw).ok())
                .map(|event| event.body)
            {
                Some(Body::PushClaimed(claimed)) => Some(claimed),
                _ => None,
            },
        )
    }

    /// Whether `person`'s browser session `session` has ended, signed out or past its lifetime:
    /// in this server's browsers for the person on its computer, in their glave account's for
    /// anyone else. Not known to have ended where this server signs in no browsers.
    async fn session_ended(&self, person: PrincipalId, session: &str) -> Result<bool, ApiError> {
        let browsers = if self.inner.harness.person() == Some(person) {
            self.inner.browsers.clone()
        } else if let Some(accounts) = self.inner.accounts.as_deref() {
            Some(accounts.browsers(person).await?)
        } else {
            None
        };
        match browsers {
            Some(browsers) => Ok(browsers.ended(session).await?),
            None => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_event::Secret;

    fn kept(n: u8, endpoint: &str, at: i64) -> Device {
        let device = DeviceId::new(Timestamp::UNIX_EPOCH, [n; 10]).unwrap();
        let added = At::from(Timestamp::from_second(at).unwrap());
        Device {
            added,
            subscribed: PushSubscribed {
                device,
                endpoint: endpoint.to_owned(),
                p256dh: "key".into(),
                auth: Secret::new("auth"),
                name: "Firefox".into(),
                session: None,
            },
            written: added,
        }
    }

    /// A subscription lapses as long after it was last written as a browser session lasts, and
    /// is subscribed again once less than half of that is left.
    #[test]
    fn a_subscription_lapses_a_session_s_lifetime_after_it_was_last_written() {
        let device = kept(1, "a", 1_000);
        let written = device.written.timestamp();
        let lapses = written.saturating_add(SESSION_LIFETIME).unwrap();
        assert_eq!(device.lapses(), lapses);
        let half = written.saturating_add(SESSION_LIFETIME / 2).unwrap();
        let second = jiff::SignedDuration::from_secs(1);
        assert!(
            !device.renew(half.saturating_sub(second).unwrap()),
            "more than half left"
        );
        assert!(!device.renew(half), "half left");
        assert!(
            device.renew(half.saturating_add(second).unwrap()),
            "less than half left"
        );
    }

    #[test]
    fn one_endpoint_is_one_browser_the_latest_subscribed() {
        let all = [
            kept(1, "a", 10),
            kept(2, "a", 30),
            kept(3, "b", 20),
            kept(4, "a", 20),
        ];
        let ids: Vec<DeviceId> = all.iter().map(|kept| kept.subscribed.device).collect();
        let held = all
            .into_iter()
            .map(|kept| (kept.subscribed.device, kept))
            .collect();
        let (latest, twice) = by_endpoint(held);
        let mut kept: Vec<DeviceId> = latest.iter().map(|kept| kept.subscribed.device).collect();
        kept.sort();
        assert_eq!(kept, [ids[1], ids[2]]);
        let mut twice = twice;
        twice.sort();
        assert_eq!(twice, [ids[0], ids[3]]);
    }
}

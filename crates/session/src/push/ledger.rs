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
use glave_ports::stream::{Reserved, catch_up};
use glave_ports::{BlobStore, Erasable, EventLog, ModelProvider, Scope};
use sha2::{Digest as _, Sha256};

use crate::protocol::ApiError;
use crate::server::Server;

/// The push ledger's events in a personal project, each numbered after the latest of them.
pub(super) const KEPT: [&str; 2] = [PushSubscribed::TYPE, PushUnsubscribed::TYPE];

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
    /// `personal`: one for each endpoint, still theirs, in a session that has not ended. The
    /// rest are forgotten as found, and any forgotten whose key still opens, as after a crash,
    /// erased now.
    pub(crate) async fn devices(
        &self,
        personal: &Scope,
        person: PrincipalId,
    ) -> Result<Vec<Device>, ApiError> {
        let mut kept: BTreeMap<DeviceId, Device> = BTreeMap::new();
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
        let (kept, twice) = by_endpoint(kept);
        for device in twice {
            self.forget_device(personal, device, false).await?;
        }
        let mut theirs = Vec::with_capacity(kept.len());
        for device in kept {
            let claimant = self.claimant(&device.subscribed.endpoint).await?;
            let moved = claimant.is_some_and(|claimant| claimant.person != person);
            let ended = match &device.subscribed.session {
                Some(session) => self.session_ended(person, session).await?,
                None => false,
            };
            if moved || ended {
                // Subscribed since by someone else, or signed out of where it was subscribed.
                self.forget_device(personal, device.subscribed.device, false)
                    .await?;
            } else {
                theirs.push(device);
            }
        }
        Ok(theirs)
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
        let device = DeviceId::new(jiff::Timestamp::UNIX_EPOCH, [n; 10]).unwrap();
        Device {
            added: At::from(jiff::Timestamp::from_second(at).unwrap()),
            subscribed: PushSubscribed {
                device,
                endpoint: endpoint.to_owned(),
                p256dh: "key".into(),
                auth: Secret::new("auth"),
                name: "Firefox".into(),
                session: None,
            },
        }
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

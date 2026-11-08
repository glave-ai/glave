//! A person's push notifications (`specs/session.md`, "Push notifications"), as Settings reads
//! and changes them: the browsers that take theirs, each kept in their personal project's push
//! ledger until it is turned off or its push service says it is gone, at most [`MOST_DEVICES`];
//! which kinds they want, in their settings ledger; and the key their organization's server
//! signs pushes with, made the first time one is needed and kept sealed in the organization's
//! push ledger, where the first one recorded holds for every server on the store.

use std::collections::BTreeMap;

use glave_event::{
    At, Body, EventId, Header, NotificationsSet, Payload, PushKeyMade, PushSubscribed,
    PushUnsubscribed, Seq, decode, encode,
};
use glave_ports::stream::{Reserved, append_retrying, catch_up};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, OrgId, Scope};
use glave_push::{Pusher, VapidKey};

use crate::protocol::{ApiError, PushDevice, PushInfo, PushSubscription, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

/// The most browsers one person's notifications go to; one more forgets the oldest.
const MOST_DEVICES: usize = 16;

/// The longest name a browser is kept under, in characters.
const LONGEST_NAME: usize = 80;

/// The push ledger's events in a personal project, each numbered after the latest of them.
const KEPT: [&str; 2] = [PushSubscribed::TYPE, PushUnsubscribed::TYPE];

/// Which kinds a person wants until they choose: every kind.
const EVERY_KIND: NotificationsSet = NotificationsSet {
    waiting: true,
    finished: true,
};

/// A browser that takes the person's notifications: its subscription, and the event that kept
/// it.
pub(crate) struct Device {
    pub(crate) id: EventId,
    pub(crate) added: At,
    pub(crate) subscribed: PushSubscribed,
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// What sends this server's pushes; refused where it sends none.
    fn pusher(&self) -> Result<&Pusher, ApiError> {
        self.inner
            .push
            .as_deref()
            .ok_or_else(|| ApiError::new(code::NOT_AVAILABLE, Sentence::NoPush))
    }

    /// Answers `push/get`: never, with why, where the server sends none.
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

    /// Answers `push/subscribe`: the browser kept, unless kept already as it is now, and the
    /// oldest forgotten when the person has as many as are kept.
    pub(crate) async fn subscribe(
        &self,
        caller: &Caller,
        browser: PushSubscription,
    ) -> Result<Reply, ApiError> {
        let PushSubscription {
            endpoint,
            p256dh,
            auth,
            device,
        } = browser;
        let refused =
            |reason: String| ApiError::new(code::INVALID_PARAMS, Sentence::PushRefused { reason });
        self.pusher()?
            .subscription(&endpoint, &p256dh, auth.expose())
            .map_err(|refusal| refused(refusal.to_string()))?;
        let device = device.trim();
        if device.is_empty() || device.chars().count() > LONGEST_NAME {
            return Err(ApiError::new(code::INVALID_PARAMS, Sentence::DeviceUnnamed));
        }
        let subscribed = PushSubscribed {
            endpoint,
            p256dh,
            auth,
            device: device.to_owned(),
        };
        let endpoint = subscribed.endpoint.as_str();
        let (_, personal) = self.personal_project(caller).await?;
        let kept = self.devices(&personal).await?;
        if kept.get(endpoint).map(|kept| &kept.subscribed) != Some(&subscribed) {
            let mut others: Vec<&Device> = kept
                .values()
                .filter(|other| other.subscribed.endpoint != endpoint)
                .collect();
            others.sort_by_key(|other| other.added);
            let over = (others.len() + 1).saturating_sub(MOST_DEVICES);
            for oldest in others.into_iter().take(over) {
                self.forget_device(&personal, &oldest.subscribed.endpoint, false)
                    .await?;
            }
            let ledger = Reserved::Push.id();
            self.keep_latest(&personal, ledger, &KEPT, &subscribed)
                .await?;
        }
        self.offered(caller, Some(endpoint)).await
    }

    /// Answers `push/remove`.
    pub(crate) async fn remove_device(
        &self,
        caller: &Caller,
        device: EventId,
    ) -> Result<Reply, ApiError> {
        self.pusher()?;
        let (_, personal) = self.personal_project(caller).await?;
        let kept = self.devices(&personal).await?;
        if let Some(found) = kept.values().find(|kept| kept.id == device) {
            self.forget_device(&personal, &found.subscribed.endpoint, false)
                .await?;
        }
        self.offered(caller, None).await
    }

    /// Answers `push/set`.
    pub(crate) async fn choose_notifications(
        &self,
        caller: &Caller,
        waiting: bool,
        finished: bool,
    ) -> Result<Reply, ApiError> {
        self.pusher()?;
        let wanted = NotificationsSet { waiting, finished };
        self.keep_setting(caller, &wanted).await?;
        self.offered(caller, None).await
    }

    /// What `push/get` says where the server sends notifications: the key to subscribe with,
    /// what the caller wants, and their browsers, the latest first, `endpoint`'s current.
    async fn offered(&self, caller: &Caller, endpoint: Option<&str>) -> Result<Reply, ApiError> {
        let key = self.push_key(self.org_of(caller)).await?;
        let wanted = self.wanted(caller).await?;
        let (_, personal) = self.personal_project(caller).await?;
        let mut devices: Vec<PushDevice> = self
            .devices(&personal)
            .await?
            .into_values()
            .map(|device| PushDevice {
                id: device.id,
                current: endpoint == Some(device.subscribed.endpoint.as_str()),
                name: device.subscribed.device,
                added: device.added,
            })
            .collect();
        devices.sort_by_key(|device| std::cmp::Reverse(device.added));
        Ok(Reply::Push {
            push: PushInfo::Offered {
                key: key.public(),
                waiting: wanted.waiting,
                finished: wanted.finished,
                devices,
            },
        })
    }

    /// The kinds of notification `caller` wants.
    pub(crate) async fn wanted(&self, caller: &Caller) -> Result<NotificationsSet, ApiError> {
        Ok(match self.setting::<NotificationsSet>(caller).await? {
            Some(Body::NotificationsSet(wanted)) => wanted,
            _ => EVERY_KIND,
        })
    }

    /// The browsers that take the notifications of the person whose personal project is
    /// `personal`, by their endpoints.
    pub(crate) async fn devices(
        &self,
        personal: &Scope,
    ) -> Result<BTreeMap<String, Device>, ApiError> {
        let mut kept = BTreeMap::new();
        let log = self.inner.harness.log();
        catch_up(log, personal, Reserved::Push.id(), None, |page| {
            for raw in page {
                let Ok(event) = decode(raw) else {
                    continue;
                };
                match event.body {
                    Body::PushSubscribed(subscribed) => {
                        let device = Device {
                            id: event.header.id,
                            added: event.header.at,
                            subscribed,
                        };
                        kept.insert(device.subscribed.endpoint.clone(), device);
                    }
                    Body::PushUnsubscribed(unsubscribed) => {
                        kept.remove(&unsubscribed.endpoint);
                    }
                    _ => {}
                }
            }
        })
        .await?;
        Ok(kept)
    }

    /// Forgets the browser at `endpoint`: its person turned it off, or its push service said
    /// it is `gone`.
    pub(crate) async fn forget_device(
        &self,
        personal: &Scope,
        endpoint: &str,
        gone: bool,
    ) -> Result<(), ApiError> {
        let unsubscribed = PushUnsubscribed {
            endpoint: endpoint.to_owned(),
            gone,
        };
        self.keep_latest(personal, Reserved::Push.id(), &KEPT, &unsubscribed)
            .await
    }

    /// The key `org`'s pushes are signed with: the one its push ledger holds, else one made now
    /// and kept there, unless another server kept one first, which is then the one.
    pub(crate) async fn push_key(&self, org: OrgId) -> Result<VapidKey, ApiError> {
        let scope = &Scope::of_organization(org);
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

//! A person's push notifications (`specs/session.md`, "Push notifications"), as Settings reads
//! and changes them: the browsers that take theirs (`ledger.rs`), and the one key the store's
//! servers sign every push with, made the first time one is needed and kept sealed in the
//! server's own organization's push ledger, where the first one recorded holds.

mod failing;
mod ledger;

use std::sync::Arc;

use glave_event::{
    At, Body, DeviceId, EventId, Header, PushKeyMade, PushSubscribed, Seq, decode, encode,
};
use glave_ports::stream::{Reserved, append_retrying};
use glave_ports::{BlobStore, Caller, Eraser, EventLog, ModelProvider, Scope};
use glave_push::{Message, PushError, Pusher, Refused, Sent, VapidKey};

use crate::browsers::session_of;
use crate::protocol::{ApiError, PushDevice, PushInfo, PushSubscription, Reply, code};
use crate::server::Server;
use crate::words::Sentence;
use failing::Failing;
pub(crate) use ledger::Device;
use ledger::KEPT;

/// The most browsers one person's notifications go to; one more forgets the oldest.
const MOST_DEVICES: usize = 16;

/// The longest name a browser is kept under, in characters.
const LONGEST_NAME: usize = 80;

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
    /// gone or failing as its own fault ([`PushError::of_the_subscription`]) several times in a
    /// row.
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

/// What a person is told of a subscription no push can reach.
fn refused(refusal: &Refused) -> ApiError {
    let sentence = match refusal {
        Refused::Endpoint => Sentence::PushServiceRefused,
        Refused::Key | Refused::Secret => Sentence::PushKeysRefused,
    };
    ApiError::new(code::INVALID_PARAMS, sentence)
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

    /// Answers `push/subscribe`: the browser kept as the caller's, in the browser session they
    /// call from, unless kept already as it is now; taken from anyone whose it was; the oldest
    /// forgotten past the most kept.
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
            .map_err(|refusal| refused(&refusal))?;
        let name = name.trim();
        if name.is_empty() || name.chars().count() > LONGEST_NAME {
            let most = LONGEST_NAME;
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                Sentence::DeviceUnnamed { most },
            ));
        }
        let person = self.person_of(caller)?;
        let session = session_of(caller).map(str::to_owned);
        let (_, personal) = self.personal_project(caller).await?;
        let kept = self.devices(&personal, person).await?;
        let same = kept
            .iter()
            .find(|kept| kept.subscribed.endpoint == endpoint);
        if let Some(same) = same.filter(|same| {
            let kept = &same.subscribed;
            (&kept.p256dh, &kept.auth, kept.name.as_str(), &kept.session)
                == (&p256dh, &auth, name, &session)
        }) {
            self.claim(&endpoint, person, same.subscribed.device)
                .await?;
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
        let device = DeviceId::new(now, self.inner.harness.entropy().bytes()?)?;
        let subscribed = PushSubscribed {
            device,
            endpoint,
            p256dh,
            auth,
            name: name.to_owned(),
            session,
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
        device: DeviceId,
    ) -> Result<Reply, ApiError> {
        self.pushing()?;
        let person = self.person_of(caller)?;
        let (_, personal) = self.personal_project(caller).await?;
        let kept = self.devices(&personal, person).await?;
        if kept.iter().any(|kept| kept.subscribed.device == device) {
            self.forget_device(&personal, device, false).await?;
        }
        self.offered(caller, None).await
    }

    /// Forgets the caller's browsers whose sessions just ended, as `signin/end` and
    /// `signin/end_others` end them; any it misses are forgotten the next time they are read.
    pub(crate) async fn forget_signed_out(&self, caller: &Caller) {
        if self.inner.push.is_none() {
            return;
        }
        let forgot = async {
            let person = self.person_of(caller)?;
            let (_, personal) = self.personal_project(caller).await?;
            self.devices(&personal, person).await
        };
        if let Err(error) = forgot.await {
            tracing::warn!(%error, "the browsers of sessions just ended could not be forgotten");
        }
    }

    /// What `push/get` says where the server pushes: the key to subscribe with and the caller's
    /// browsers, the latest first, `endpoint`'s current.
    async fn offered(&self, caller: &Caller, endpoint: Option<&str>) -> Result<Reply, ApiError> {
        let key = self.push_key().await?;
        let person = self.person_of(caller)?;
        let (_, personal) = self.personal_project(caller).await?;
        let mut devices: Vec<PushDevice> = self
            .devices(&personal, person)
            .await?
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

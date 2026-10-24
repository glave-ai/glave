//! The caller's settings ledger (`specs/events.md`, "The settings ledger"), in their personal
//! project: the latest of one kind of their choices, and keeping a new one after the ledger's
//! last event, whatever that chose. The language they read glave in and how their threads are
//! grouped are kept here, beside whether their computer stays awake.

use glave_event::{At, Body, EventId, Header, Payload, Seq, decode, encode};
use glave_ports::stream::{Reserved, SETTINGS, append_retrying};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};

use crate::protocol::ApiError;
use crate::server::Server;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The caller's latest choice recorded as `kind`; none when they never chose, or when this
    /// glave cannot read the one they made.
    pub(crate) async fn setting(
        &self,
        caller: &Caller,
        kind: &str,
    ) -> Result<Option<Body>, ApiError> {
        let (_, scope) = self.personal_project(caller).await?;
        let latest = self
            .inner
            .harness
            .log()
            .latest_of(&scope, Reserved::Settings.id(), &[kind])
            .await?;
        Ok(latest
            .and_then(|raw| decode(raw).ok())
            .map(|event| event.body))
    }

    /// Keeps `setting` as the caller's latest choice of its kind: appended after the ledger's
    /// last event, read again and tried again when another writer appended first.
    pub(crate) async fn keep_setting<T: Payload + Sync>(
        &self,
        caller: &Caller,
        setting: &T,
    ) -> Result<(), ApiError> {
        let (_, scope) = self.personal_project(caller).await?;
        let (log, ledger) = (self.inner.harness.log(), Reserved::Settings.id());
        let (this, scope) = (self, &scope);
        append_retrying(log, scope, ledger, move || async move {
            let last = log.latest_of(scope, ledger, &SETTINGS).await?;
            let seq = last
                .map_or(Some(Seq::FIRST), |raw| raw.seq.next())
                .ok_or_else(|| ApiError::internal(&"the settings ledger is full"))?;
            let now = this.inner.harness.clock().now();
            let header = Header {
                id: EventId::new(now, this.inner.harness.entropy().bytes()?)?,
                thread: ledger,
                seq,
                at: At::from(now),
            };
            Ok::<_, ApiError>(Some((vec![encode(header, setting)?], ())))
        })
        .await?;
        Ok(())
    }
}

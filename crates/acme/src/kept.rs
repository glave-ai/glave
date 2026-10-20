//! The certificates ledger (`glave_ports::stream::Reserved::Certificates`): the accounts glave
//! registered with ACME directories and the certificates it was issued, kept in the store and
//! sealed there with everything else. The latest of each counts.

use glave_event::{
    AcmeRegistered, At, Body, CertificateIssued, EventId, Header, RawEvent, Seq, decode, encode,
};
use glave_ports::stream::{Reserved, StreamError, append_retrying, catch_up};
use glave_ports::{Clock, Entropy, EventLog, Scope};

use crate::https::HttpsError;

/// The ledger in `scope` of `log`.
pub(crate) struct Ledger<L> {
    pub(crate) log: L,
    pub(crate) scope: Scope,
    pub(crate) clock: Box<dyn Clock>,
    pub(crate) entropy: Box<dyn Entropy>,
}

/// What the ledger holds for one name at one directory.
#[derive(Default)]
pub(crate) struct Kept {
    pub(crate) account: Option<AcmeRegistered>,
    pub(crate) certificate: Option<CertificateIssued>,
}

impl<L: EventLog> Ledger<L> {
    /// The latest account at `directory` and the latest certificate for `name`.
    pub(crate) async fn read(&self, name: &str, directory: &str) -> Result<Kept, HttpsError> {
        let mut kept = Kept::default();
        let thread = Reserved::Certificates.id();
        catch_up(&self.log, &self.scope, thread, None, |page| {
            for raw in page {
                match decode(raw).map(|event| event.body) {
                    Ok(Body::AcmeRegistered(account)) if account.directory == directory => {
                        kept.account = Some(account);
                    }
                    Ok(Body::CertificateIssued(issued)) if issued.name == name => {
                        kept.certificate = Some(issued);
                    }
                    _ => {}
                }
            }
        })
        .await?;
        Ok(kept)
    }

    /// Appends `account`.
    pub(crate) async fn registered(&self, account: &AcmeRegistered) -> Result<(), HttpsError> {
        self.append(|header| encode(header, account)).await
    }

    /// Appends `issued`.
    pub(crate) async fn issued(&self, issued: &CertificateIssued) -> Result<(), HttpsError> {
        self.append(|header| encode(header, issued)).await
    }

    /// Appends the event `make` makes of the header after the ledger's last event, reading it
    /// again when another glave appended first.
    async fn append<E: std::fmt::Display>(
        &self,
        make: impl Fn(Header) -> Result<RawEvent, E>,
    ) -> Result<(), HttpsError> {
        let thread = Reserved::Certificates.id();
        let make = &make;
        append_retrying(&self.log, &self.scope, thread, || async move {
            let mut last = None;
            catch_up(&self.log, &self.scope, thread, None, |page| {
                last = page.last().map(|event| event.seq).or(last);
            })
            .await?;
            let now = self.clock.now();
            let next = last.map_or(1, |seq| seq.get().saturating_add(1));
            let header = Header {
                id: EventId::new(now, self.entropy.bytes()?).map_err(keep)?,
                thread,
                seq: Seq::new(next).map_err(keep)?,
                at: At::from(now),
            };
            let event = make(header).map_err(keep)?;
            Ok::<_, HttpsError>(Some((vec![event], ())))
        })
        .await?;
        Ok(())
    }
}

/// Why the ledger could not be written, as an error.
fn keep(error: impl std::fmt::Display) -> HttpsError {
    HttpsError::Keep(error.to_string())
}

impl From<StreamError> for HttpsError {
    fn from(error: StreamError) -> Self {
        Self::Keep(error.to_string())
    }
}

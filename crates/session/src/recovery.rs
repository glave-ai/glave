//! Getting back into a glave account (`specs/model.md`, "Identity"), on a server that keeps
//! glave accounts at a public origin and sends email: someone who lost their passkeys gives
//! their address and is sent a one-time link, good for 15 minutes, which makes a new passkey
//! for the account that address has; an address with none is sent a note saying so, through
//! the same steps (`signups`). A passkey so made is marked, its account's address is told of
//! it, and so are the person's other devices, which can remove it.

use std::sync::Arc;

use glave_identity::signin::{Browsers, Checked};
use glave_ports::{BlobStore, Caller, EventLog, Mail, ModelProvider};

use crate::organizations::with_account;
use crate::protocol::{ApiError, Reply};
use crate::server::Server;
use crate::signups::Signing;
use crate::words::{Language, Sentence, say};

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Whether someone may get back into their account by email here: the server keeps glave
    /// accounts at a public origin and sends email.
    pub(crate) fn recovers(&self) -> bool {
        self.inner.accounts.is_some() && self.inner.mail.is_some() && self.inner.origin.is_some()
    }

    /// Finishes `passkey/create` for a link to get back in, spent already: keeps the passkey
    /// `checked`, marked as made by email, signs the browser in, and tells the address.
    pub(crate) async fn recovered(
        &self,
        signing: &Signing,
        browsers: &Arc<Browsers>,
        checked: Checked,
    ) -> Result<Reply, ApiError> {
        let session = browsers.keep_emailed(checked).await?;
        let (mailer, origin, to) = self.sender(&signing.email)?;
        let said = say(Language::English, &Sentence::PasskeyEmailed);
        let mail = Mail {
            to,
            subject: say(Language::English, &Sentence::PasskeyEmailedSubject),
            text: format!("{said}\n\n{origin}\n"),
        };
        // The passkey is made: a message that cannot go now is no reason to undo it, and the
        // person's other devices are told all the same.
        if self.send(&*mailer, &mail).await.is_err() {
            tracing::warn!("could not tell an account's address of a passkey made by email");
        }
        Ok(self.signed_in(session, browsers))
    }

    /// Whether `caller`, a glave account, can get back in by email without a passkey: the
    /// server sends links to get back in, and their account has an address it would send one
    /// to. Then their last passkey may go.
    pub(crate) async fn gets_back_in(&self, caller: &Caller) -> Result<bool, ApiError> {
        Ok(self.recovers() && self.account_email(caller).await?.is_some())
    }

    /// The address `caller`'s glave account gets back in with: the one it signed up or was
    /// invited with, as the people ledger of one of its organizations has it, and only while
    /// the directory says it is this account's, the one a link to get back in opens. None for
    /// anyone else.
    pub(crate) async fn account_email(&self, caller: &Caller) -> Result<Option<String>, ApiError> {
        let (Some(person), Some(accounts)) = (caller.person, self.inner.accounts.as_deref()) else {
            return Ok(None);
        };
        if !with_account(caller) {
            return Ok(None);
        }
        for org in accounts.organizations(person).await? {
            self.catch_up_people(org).await?;
            let email = self.people(org, |people| {
                people.info(person).and_then(|info| info.email)
            });
            if let Some(email) = email
                && accounts.with_email(&email).await? == Some(person)
            {
                return Ok(Some(email));
            }
        }
        Ok(None)
    }
}

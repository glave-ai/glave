//! `server/discover`: what the server is, what exists here, and what the caller may do. No one,
//! not signed in, is told only what the server is and how to sign in, and nothing of anyone's
//! is read for them: not the person's settings, which `reread` reads, nor what they may
//! choose, nor the server's sandbox (`specs/session.md`, "Who is calling").

use glave_event::Permissions;
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};

use crate::protocol::{ApiError, Exists, PROTOCOL, Reply, ServerInfo, Told};
use crate::server::Server;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `server/discover` for `caller`.
    pub(crate) async fn discover(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let no_one = caller.is_no_one();
        if !no_one {
            self.reread().await;
        }
        let organization = self.organization_shown(caller).await?;
        let invites = self.may_invite(caller).await?;
        let personal = !no_one && self.in_personal(caller).await?;
        let role = if organization && caller.auth.is_some() {
            Some(self.role_of(caller).await?)
        } else {
            None
        };
        // What a thread of theirs may run with: no one has a thread, so glave's own default
        // and nothing to choose, which says nothing of the person's choices or of this computer.
        let (permissions, modes, unsandboxed) = if no_one {
            (Permissions::default(), Vec::new(), None)
        } else {
            let person = caller.person;
            let permissions = self.inner.harness.defaults().permissions;
            (
                self.within_sandbox(person, permissions),
                self.modes(person),
                self.unsandboxed(person).map(Told::from),
            )
        };
        Ok(Reply::Server(ServerInfo {
            name: "glave".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            protocols: vec![PROTOCOL.to_owned()],
            permissions,
            modes,
            unsandboxed,
            id: self.server_id(),
            origin: self.inner.origin.as_ref().map(ToString::to_string),
            here: self.passkeys_here(),
            signin: self.signin_ways(),
            invite_only: self.invite_only(),
            exists: Exists {
                organization,
                credits: self.inner.credits.is_some(),
            },
            role,
            email: self.inner.mail.is_some() && self.inner.origin.is_some(),
            invites,
            personal,
            organizations: self.account_of(caller).is_some(),
            model_accounts: self.providers_for(caller).is_ok(),
            push: self.inner.push.is_some(),
            connects: self.connects(caller),
        }))
    }
}

//! Passkeys indexed as a server starts (#1523): those made before the index sign in from its
//! first request, and a person whose passkeys can't be indexed then is skipped, the start
//! going on.

use std::sync::{Arc, Mutex, PoisonError};

use glave_event::{OrgRole, PrincipalId};
use glave_identity::testing::Authenticator;
use glave_ports::{BoxFuture, Directory, DirectoryError, IdentityKey, OrgId};
use glave_session::protocol::Request;
use glave_session::signin::LocalToken;

use super::{Reached, invite, invited, join, serve_keeping, serve_through, sign_in};
use crate::support::web::{ORIGIN, Web, asked};
use crate::support::*;

/// A passkey made before passkeys were indexed (#1523) signs in on a server just started, with
/// no session of its person's to open their account first: the server indexes everyone's in
/// its organizations as it starts.
#[test]
fn a_passkey_made_before_the_index_signs_in_on_a_server_just_started() {
    block_on!(async {
        let dir = folder("accounts-indexed-at-start");
        let token = LocalToken::new([24; 32]);
        let (served, directory, _) = serve_keeping(&dir, true, &token).await;
        let mut tablet = Authenticator::new(ORIGIN).unwrap();
        join(
            &served,
            invite(&served, &token, OrgRole::Member).await,
            &mut tablet,
        )
        .await;
        let mut here = Web::new(&served, Some(token.reveal().to_owned()));
        let listed = Request::PeopleList {
            after: None,
            limit: None,
        };
        let people = asked(&mut here, &listed).await;
        let ada: PrincipalId = serde_json::from_value(invited(&people)["id"].clone()).unwrap();
        served.close().await;
        // As a store from before the index holds it: her passkey indexed nowhere, though she
        // is in the organization.
        let org = scope().org();
        directory.leave(ada, org).await.unwrap();
        directory.unlink_email_if_alone(ada).await.unwrap();
        directory.join(ada, org).await.unwrap();

        let (served, _, _) = serve_keeping(&dir, true, &token).await;
        let session = sign_in(&served, &mut tablet).await;
        assert!(session.is_string(), "{session}");
        served.close().await;
    });
}

/// A directory that fails to index `person`'s passkeys, as a store that can't be reached for
/// them, and is otherwise `inner`.
struct FailingFor {
    inner: Arc<dyn Directory>,
    person: Mutex<Option<PrincipalId>>,
}

impl FailingFor {
    fn fails(&self, person: PrincipalId) -> bool {
        *self.person.lock().unwrap_or_else(PoisonError::into_inner) == Some(person)
    }
}

#[deny(clippy::missing_trait_methods)]
impl Directory for FailingFor {
    fn link<'a>(
        &'a self,
        identity: &'a IdentityKey,
        person: PrincipalId,
    ) -> BoxFuture<'a, Result<PrincipalId, DirectoryError>> {
        if self.fails(person) {
            return Box::pin(async { Err(DirectoryError::Storage("did not answer".into())) });
        }
        self.inner.link(identity, person)
    }

    fn unlink_email_if_alone(
        &self,
        person: PrincipalId,
    ) -> BoxFuture<'_, Result<(), DirectoryError>> {
        self.inner.unlink_email_if_alone(person)
    }

    fn find<'a>(
        &'a self,
        identity: &'a IdentityKey,
    ) -> BoxFuture<'a, Result<Option<PrincipalId>, DirectoryError>> {
        self.inner.find(identity)
    }

    fn join(&self, person: PrincipalId, org: OrgId) -> BoxFuture<'_, Result<(), DirectoryError>> {
        self.inner.join(person, org)
    }

    fn leave(&self, person: PrincipalId, org: OrgId) -> BoxFuture<'_, Result<(), DirectoryError>> {
        self.inner.leave(person, org)
    }

    fn organizations(
        &self,
        person: PrincipalId,
    ) -> BoxFuture<'_, Result<Vec<OrgId>, DirectoryError>> {
        self.inner.organizations(person)
    }
}

/// The person whose browser session `session` is: an account's token names it.
fn whose(session: &serde_json::Value) -> PrincipalId {
    let token = session.as_str().unwrap();
    let named = token.strip_prefix("glave_session_").unwrap();
    let (id, _) = named.split_once('_').unwrap();
    serde_json::from_value(serde_json::json!(uuid::Uuid::parse_str(id).unwrap())).unwrap()
}

/// Drops `people`'s passkeys from the index, as a store from before the index holds them,
/// leaving them in the organization.
async fn unindexed(directory: &Arc<dyn Directory>, people: &[PrincipalId]) {
    let org = scope().org();
    for id in people {
        directory.leave(*id, org).await.unwrap();
        directory.unlink_email_if_alone(*id).await.unwrap();
        directory.join(*id, org).await.unwrap();
    }
}

/// A person whose passkeys can't be indexed as the server starts is skipped, said in the log,
/// and the start goes on: everyone else's are indexed, and sign in.
#[test]
fn a_person_whose_passkeys_cant_be_indexed_at_the_start_is_skipped() {
    block_on!(async {
        let dir = folder("accounts-index-skips");
        let token = LocalToken::new([25; 32]);
        let (served, directory, _) = serve_keeping(&dir, true, &token).await;
        let mut first = Authenticator::new(ORIGIN).unwrap();
        let code = invite(&served, &token, OrgRole::Member).await;
        join(&served, code, &mut first).await;
        let mut second = Authenticator::new(ORIGIN).unwrap();
        let code = invite(&served, &token, OrgRole::Member).await;
        join(&served, code, &mut second).await;
        let failing = whose(&sign_in(&served, &mut first).await);
        let other = whose(&sign_in(&served, &mut second).await);
        unindexed(&directory, &[failing, other]).await;
        served.close().await;

        let (served, _, _) = serve_through(&dir, Reached::Cloud, &token, |inner| {
            Arc::new(FailingFor {
                inner,
                person: Mutex::new(Some(failing)),
            })
        })
        .await;
        assert!(sign_in(&served, &mut first).await.is_null(), "skipped");
        assert!(sign_in(&served, &mut second).await.is_string(), "indexed");
        served.close().await;
    });
}

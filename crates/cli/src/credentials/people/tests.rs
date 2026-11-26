//! Each person's git hosts on one store: one person's kept for them alone, in each
//! organization apart, never offered to or handed to another's commands.

use std::sync::Arc;

use glave_event::{CredentialConnected, Forge, Principal, PrincipalId, Secret};
use glave_ports::computer::CredentialAsk;
use glave_ports::testing::fake_secret;
use glave_ports::{Clock as _, OrgId, SystemClock};

use super::*;
use crate::accounts::random;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn someone() -> PrincipalId {
    PrincipalId::new(SystemClock.now(), random().unwrap()).unwrap()
}

/// `person`'s sign-in to GitHub, with `token`.
fn github(person: PrincipalId, token: &Secret) -> CredentialConnected {
    CredentialConnected {
        credential: super::super::new_id().unwrap(),
        forge: Forge::Github,
        url: "https://github.com".to_owned(),
        username: "x-access-token".into(),
        token: token.clone(),
        login: None,
        scopes: None,
        renewal: None,
        by: Principal::person(person),
    }
}

fn asked() -> CredentialAsk {
    CredentialAsk {
        protocol: "https".into(),
        host: "github.com".into(),
        path: None,
    }
}

#[test]
fn each_person_s_commands_get_their_own_git_hosts_and_never_another_s() {
    block_on(async {
        let store = Data::in_memory().unwrap();
        let own_org = store.org().await.unwrap();
        let (here, ada, grace) = (someone(), someone(), someone());
        let own_ledger = crate::accounts::open(&store, here).await.unwrap();
        let own = Hosts::open(own_ledger, None).await.unwrap();
        let people = PeoplesHosts::new(store.clone(), None, (own_org, here), Arc::clone(&own));
        let team = OrgId::from_uuid(uuid::Uuid::from_u128(7));

        let token = Secret::new(fake_secret("ghu_"));
        let ledger = crate::accounts::open_in(&store, team, ada).await.unwrap();
        ledger.keep_credential(&github(ada, &token)).await.unwrap();

        let theirs = people.credentials(team, ada);
        assert_eq!(
            theirs.hosts().await.len(),
            1,
            "ada's commands are offered hers"
        );
        let given = theirs.request(&asked()).await.unwrap();
        assert!(
            given.password.expose() == token.expose(),
            "ada's command is given her own token"
        );

        let others = [
            ("grace in the team", team, grace),
            ("ada in the server's own organization", own_org, ada),
            ("the person on this computer", own_org, here),
        ];
        for (who, org, person) in others {
            let other = people.credentials(org, person);
            assert!(other.hosts().await.is_empty(), "{who} is offered none");
            assert!(
                other.request(&asked()).await.is_none(),
                "{who} is given none"
            );
            let listed = people.of(org, person).await.unwrap().list().await.unwrap();
            assert!(listed.is_empty(), "{who} lists none");
        }
        let nobody = someone();
        let theirs = people.credentials(team, nobody);
        assert!(theirs.hosts().await.is_empty() && theirs.request(&asked()).await.is_none());
        let made = crate::projects::find_personal_in(&store, team, nobody)
            .await
            .unwrap();
        assert!(made.is_none(), "a command's ask makes no personal project");

        let own_listed = people.of(own_org, here).await.unwrap();
        assert!(
            Arc::ptr_eq(
                &(own_listed as Arc<dyn GitHosts>),
                &(own as Arc<dyn GitHosts>)
            ),
            "the person on this computer's are the ones its commands ask"
        );
    });
}

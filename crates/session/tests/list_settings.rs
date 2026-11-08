//! What a list's watch (`thread/list/watch`) says of settings: a person's change of a setting
//! is heard in the organization it was made in, their name and passkeys in all of them, an
//! organization's brand by everyone in it, and a client back from away is told every setting
//! may have changed.

use std::sync::Arc;

use glave_event::{Accent, OrgRole, Secret};
use glave_ports::Caller;
use glave_ports::testing::{HUNG, Outbox};
use glave_session::Answering;
use glave_session::protocol::{Grouping, Notification, Request, Setting};

use crate::list_watching::{bea, hear, long_ago};
use crate::signups::{serve, signed_up};
use crate::support::following::*;
use crate::support::web::{Served, Web, asked, elsewhere};
use crate::support::*;

#[test]
fn a_person_hears_their_own_settings_change_and_nobody_else_s() {
    block_on!(async {
        for kind in KINDS {
            let name = format!("list-settings-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let server = server(&dir, Vec::new(), Answering::Clients).await;
            let mut over = Over::new(kind, &server, &name).await;
            let bea = bea();
            let mut watcher = over.watcher().await;
            // From long ago, so the thread is heard of whether the watch begins before it or
            // after it; heard, the watch follows from there.
            watcher
                .watch_list(&Caller::default(), None, Some(long_ago()), false)
                .await;
            let first = start(&server, false).await;
            hear(&mut watcher, first, &mut Vec::new(), |_| true).await;
            let grouped = || Request::GroupingSet {
                grouping: Grouping::Status,
            };
            server.request_as(&bea, grouped()).await.unwrap();
            server.request(grouped()).await.unwrap();
            server.request(Request::GroupingGet {}).await.unwrap();
            let chosen = Request::LanguageSet {
                language: Some("en-XA".into()),
            };
            server.request(chosen).await.unwrap();
            let second = start(&server, false).await;
            let mut settings = Vec::new();
            loop {
                match watcher.next().await {
                    Said::Note(Notification::SettingsChanged { setting }) => settings.push(setting),
                    Said::Note(Notification::Changed { summary, .. }) if summary.id == second => {
                        break;
                    }
                    Said::Note(_) => {}
                    Said::Answer(answer) => panic!("the watch ended: {answer}"),
                }
            }
            assert_eq!(
                settings,
                [Setting::Grouping, Setting::Language],
                "{kind:?}: their own changes, once each; not another person's, nor a read"
            );
            over.close().await;
        }
    });
}

#[test]
fn a_client_back_from_away_hears_every_setting_may_have_changed() {
    block_on!(async {
        for kind in KINDS {
            let name = format!("list-away-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let server = server(&dir, Vec::new(), Answering::Clients).await;
            let mut over = Over::new(kind, &server, &name).await;
            let mut watcher = over.watcher().await;
            watcher
                .watch_list(&Caller::default(), None, None, true)
                .await;
            let every = [
                Setting::Providers,
                Setting::Credentials,
                Setting::Connections,
                Setting::Secrets,
                Setting::Search,
                Setting::Grouping,
                Setting::Filters,
                Setting::Appearance,
                Setting::Brand,
                Setting::Awake,
                Setting::Memory,
                Setting::Passkeys,
                Setting::Language,
                Setting::Push,
            ];
            assert_eq!(
                settings_until(&mut watcher, Setting::Push).await,
                every,
                "{kind:?}: their own settings and their organization's brand, once each"
            );
            // Then it follows as any watch does.
            server.request(language_chosen()).await.unwrap();
            assert_eq!(
                settings_until(&mut watcher, Setting::Language).await,
                [Setting::Language],
                "{kind:?}"
            );
            over.close().await;
        }
    });
}

/// Ada, signed up, and the session and ids of her two organizations: the first her own, the
/// second one she joined by Carol's invite.
async fn ada_in_two_organizations(served: &Served, outbox: &Outbox) -> (String, String, String) {
    let mut carol = signed_up(served, outbox, "carol@example.com").await;
    ada_invited_by(&mut carol, served, outbox).await
}

/// [`ada_in_two_organizations`], the second the organization of `carol`, its admin.
async fn ada_invited_by(
    carol: &mut Web,
    served: &Served,
    outbox: &Outbox,
) -> (String, String, String) {
    let mut ada = signed_up(served, outbox, "ada@example.com").await;
    let invite = Request::InviteCreate {
        role: OrgRole::Member,
        email: None,
    };
    let invited = asked(carol, &invite).await;
    let accept = Request::InviteAccept {
        code: Secret::new(invited["code"].as_str().unwrap()),
    };
    changed(&mut ada, &accept).await;
    let listed = asked(&mut ada, &Request::OrganizationList {}).await;
    let id = |at: usize| {
        listed["organizations"][at]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    (ada.token.clone().unwrap(), id(0), id(1))
}

/// Asks `request` of `web`, which must answer it: refused, a watch waiting to hear of it would
/// wait on.
async fn changed(web: &mut Web, request: &Request) {
    let answer = asked(web, request).await;
    assert!(answer.is_object(), "{request:?}: {answer}");
}

/// Watches the threads list of whoever `web` is, and the organization it names.
async fn watching(web: Web) -> Watcher {
    let mut watcher = Watcher::Http(web, None);
    watcher
        .watch_list(&Caller::default(), None, None, false)
        .await;
    watcher
}

/// Reads `watcher` until it names `last`: each setting it named, `last` too. Those told together
/// come in the order of [`Setting`], so one told before `last` that sorts after it would go
/// unread: `last` is the latest of what a test tells.
async fn settings_until(watcher: &mut Watcher, last: Setting) -> Vec<Setting> {
    let heard = tokio::time::timeout(HUNG, async {
        let mut settings = Vec::new();
        loop {
            match watcher.next().await {
                Said::Note(Notification::SettingsChanged { setting }) => {
                    settings.push(setting);
                    if setting == last {
                        return settings;
                    }
                }
                Said::Note(other) => panic!("only settings changed, not {other:?}"),
                Said::Answer(answer) => panic!("the watch ended: {answer}"),
            }
        }
    });
    heard.await.unwrap()
}

/// A client of `session` that names `org` as the one it works in.
fn working_in(served: &Served, session: &str, org: &str) -> Web {
    elsewhere(served, Some(session.to_owned())).working_in(org)
}

/// The change of a setting a test tells last: see [`settings_until`].
fn language_chosen() -> Request {
    Request::LanguageSet {
        language: Some("en-XA".into()),
    }
}

#[test]
fn a_setting_changed_in_one_organization_is_heard_only_in_that_one() {
    block_on!(async {
        let dir = folder("list-setting-organizations");
        let outbox = Arc::new(Outbox::default());
        let served = serve(&dir, true, &outbox).await;
        let (session, own, joined) = ada_in_two_organizations(&served, &outbox).await;
        let grouped = Request::GroupingSet {
            grouping: Grouping::Status,
        };
        let naming_none = || elsewhere(&served, Some(session.clone()));
        let client = |org: &str| working_in(&served, &session, org);
        for (watch, here, there) in [
            (client(&own), &own, &joined),
            (client(&joined), &joined, &own),
            // Naming none, she works in the first she joined.
            (naming_none(), &own, &joined),
        ] {
            let mut watcher = watching(watch).await;
            changed(&mut client(there), &grouped).await;
            changed(&mut client(here), &language_chosen()).await;
            assert_eq!(
                settings_until(&mut watcher, Setting::Language).await,
                [Setting::Language],
                "watching in {here}, the change in {there} is not heard"
            );
        }
        served.close().await;
    });
}

#[test]
fn a_name_changed_in_one_organization_is_heard_in_all_of_them() {
    block_on!(async {
        let dir = folder("list-name-organizations");
        let outbox = Arc::new(Outbox::default());
        let served = serve(&dir, true, &outbox).await;
        let (session, own, joined) = ada_in_two_organizations(&served, &outbox).await;
        let client = |org: &str| working_in(&served, &session, org);
        let mut watcher = watching(client(&own)).await;
        let named = Request::PersonName { name: "Ada".into() };
        changed(&mut client(&joined), &named).await;
        changed(&mut client(&own), &language_chosen()).await;
        assert_eq!(
            settings_until(&mut watcher, Setting::Language).await,
            [Setting::Passkeys, Setting::Language],
            "her name and passkeys are hers wherever she works"
        );
        served.close().await;
    });
}

#[test]
fn a_brand_changed_is_heard_by_everyone_in_its_organization_and_nobody_else() {
    block_on!(async {
        let dir = folder("list-brand-organization");
        let outbox = Arc::new(Outbox::default());
        let served = serve(&dir, true, &outbox).await;
        let mut carol = signed_up(&served, &outbox, "carol@example.com").await;
        let (session, own, carols) = ada_invited_by(&mut carol, &served, &outbox).await;
        let dave = signed_up(&served, &outbox, "dave@example.com").await;
        let of = |person: &Web| elsewhere(&served, person.token.clone());
        let ada = |org: &str| working_in(&served, &session, org);
        let mut admin = watching(of(&carol)).await;
        let mut member = watching(ada(&carols)).await;
        let mut elsewhere_too = watching(ada(&own)).await;
        let mut outsider = watching(of(&dave)).await;

        let branded = Request::BrandSet {
            name: Some("Carol's".into()),
            logo: None,
            accent: Accent::Plain,
            locked: false,
        };
        changed(&mut carol, &branded).await;
        for (mut client, watcher, told, who) in [
            (of(&carol), &mut admin, true, "another tab of the admin"),
            (ada(&carols), &mut member, true, "a member"),
            (ada(&own), &mut elsewhere_too, false, "the member elsewhere"),
            (of(&dave), &mut outsider, false, "someone in another"),
        ] {
            changed(&mut client, &language_chosen()).await;
            let expected: &[Setting] = if told {
                &[Setting::Brand, Setting::Language]
            } else {
                &[Setting::Language]
            };
            assert_eq!(
                settings_until(watcher, Setting::Language).await,
                expected,
                "{who}"
            );
        }
        // What the member reads again is what the admin kept.
        let read = asked(&mut ada(&carols), &Request::AppearanceGet {}).await;
        assert_eq!(read["organization"]["name"], "Carol's");

        // A person's own accent stays theirs, in the same organization.
        let accent = Request::AppearanceSet {
            accent: Some(Accent::Plain),
        };
        changed(&mut ada(&carols), &accent).await;
        changed(&mut of(&carol), &language_chosen()).await;
        assert_eq!(
            settings_until(&mut admin, Setting::Language).await,
            [Setting::Language],
            "a member's accent is not told to anyone else"
        );
        served.close().await;
    });
}

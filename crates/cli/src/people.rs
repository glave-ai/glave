//! `glave people lookup`: whom an anonymous id a model provider reports is (`specs/gateway.md`,
//! "Whose call it is"), for whoever runs the server. Each id is made under a secret only the
//! server's root derives, so the lookup reads the store and the root itself, on the server's
//! computer, and makes the id of every person in every organization there until one matches.

use std::collections::BTreeSet;
use std::process::ExitCode;

use anyhow::{Context as _, bail};
use glave_event::{Body, PrincipalId};
use glave_ports::stream::{Reserved, catch_up, in_order};
use glave_ports::{OrgId, Scope};

use crate::data::Data;
use crate::session::run_with;

pub(crate) const USAGE: &str = "\
glave people: the people on this glave, for whoever runs it

Usage:
  glave people lookup <id>      whom an id a model provider reports is, such as g_3f2a…:
                                the organization and the person, read from this glave's
                                own store on its computer";

/// The id the words after `people` ask about.
fn parse(words: &[String]) -> anyhow::Result<&str> {
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["lookup", id] => Ok(id),
        ["lookup"] => bail!("which id?\n\n{USAGE}"),
        [] => bail!("{USAGE}"),
        ["lookup", _, extra, ..] | [extra, ..] => bail!("unexpected {extra:?}\n\n{USAGE}"),
    }
}

/// `glave people ...`.
pub(crate) fn main(words: &[String]) -> ExitCode {
    run_with(words, USAGE, |words| async move {
        let id = parse(&words)?;
        let store = Data::here().await?;
        let end_users = crate::keys::end_users(&store).await?;
        let people = everyone(&store).await?;
        let Some((org, person)) = end_users.find(id, people) else {
            bail!("no one on this glave has the id {id}");
        };
        println!("organization {org}\nperson {person}");
        Ok(())
    })
}

/// Every person in every organization on `store`, removed ones included, since a provider
/// may report an id after its person has left: those its organizations' people ledgers name,
/// and the person on this computer in its own, whom no ledger records.
async fn everyone(store: &Data) -> anyhow::Result<BTreeSet<(OrgId, PrincipalId)>> {
    let mut people = BTreeSet::new();
    let own = store.org().await?;
    let local = crate::identity::load_or_make(store).await?.person;
    people.insert((own, local));
    let log = store.log();
    for org in store.organizations().await? {
        let scope = Scope::of_organization(org);
        let mut next = 0;
        catch_up(&log, &scope, Reserved::People.id(), None, |page| {
            for raw in page {
                if let Some((_, body)) = in_order(&mut next, raw)
                    && let Some(person) = named(&body)
                {
                    people.insert((org, person));
                }
            }
        })
        .await
        .with_context(|| format!("could not read the people of organization {org}"))?;
    }
    Ok(people)
}

/// The person a people ledger's event is about, if it is about one.
fn named(body: &Body) -> Option<PrincipalId> {
    match body {
        Body::PersonJoined(joined) => Some(joined.person),
        Body::PersonRemoved(gone) => Some(gone.person),
        Body::RoleSet(set) => Some(set.person),
        Body::InviteAccepted(accepted) => Some(accepted.person),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use glave_event::{At, EventId, Header, PersonJoined, Seq, encode};
    use glave_ports::{Clock as _, EventLog as _, SystemClock};

    use super::*;

    #[test]
    fn an_id_a_provider_reports_leads_back_to_its_person_and_organization() {
        let root = std::sync::Arc::new(glave_keys::InMemoryRoot::new().unwrap());
        let keys = glave_keys::KeySettings::new(root);
        let store = Data::Sqlite(glave_store_sqlite::Store::open_in_memory(keys).unwrap());
        glave_ports::block_on!(async {
            let now = SystemClock.now();
            let joined = PrincipalId::new(now, [2; 10]).unwrap();
            let org = OrgId::from_uuid(PrincipalId::new(now, [3; 10]).unwrap().as_uuid());
            let header = Header {
                id: EventId::new(now, [4; 10]).unwrap(),
                thread: Reserved::People.id(),
                seq: Seq::new(1).unwrap(),
                at: At::from(now),
            };
            let payload = PersonJoined {
                person: joined,
                issuer: None,
                subject: None,
                profile: glave_event::Profile::default(),
            };
            let event = encode(header, &payload).unwrap();
            let log = store.log();
            log.append(&Scope::of_organization(org), &[event])
                .await
                .unwrap();
            let ids = crate::keys::end_users(&store).await.unwrap();
            let local = crate::identity::load_or_make(&store).await.unwrap().person;
            let own = store.org().await.unwrap();
            for (org, person) in [(org, joined), (own, local)] {
                let id = ids.id(org, person);
                let found = ids.find(id.as_str(), everyone(&store).await.unwrap());
                assert_eq!(found, Some((org, person)));
            }
            let stranger = ids.id(own, joined);
            assert_eq!(
                ids.find(stranger.as_str(), everyone(&store).await.unwrap()),
                None
            );
        });
    }

    fn words(text: &[&str]) -> Vec<String> {
        text.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn people_lookup_takes_one_id() {
        let asked = words(&["lookup", "g_00"]);
        assert_eq!(parse(&asked).unwrap(), "g_00");
        for wrong in [&[][..], &["lookup"], &["lookup", "a", "b"], &["find", "a"]] {
            assert!(parse(&words(wrong)).is_err(), "{wrong:?}");
        }
    }
}

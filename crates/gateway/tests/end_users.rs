//! Whose call it is: each call that works for a person tells its provider who, by an
//! anonymous id of that person in their organization; a call for no one tells nothing.

use glave_event::PrincipalId;
use glave_gateway::{Gateway, Routes};
use glave_ports::{EndUsers, ModelProvider, OrgId, Payer, Purpose};

use crate::support::{Fake, block_on, request};

fn person(n: u8) -> PrincipalId {
    PrincipalId::new(jiff::Timestamp::UNIX_EPOCH, [n; 10]).unwrap()
}

fn org() -> OrgId {
    OrgId::from_uuid(person(100).as_uuid())
}

#[test]
fn a_call_for_a_person_names_them_by_their_id_and_one_for_no_one_names_no_one() {
    let provider = Fake::new("openrouter");
    let routes = Routes::new(vec![provider.target("big")]).unwrap();
    let ids = EndUsers::new(&[7; 32]);
    let gateway = Gateway::new(routes).with_end_users(Some(ids.clone()));
    let org = org();
    let calls = [
        Some(Payer {
            org,
            person: Some(person(1)),
        }),
        Some(Payer {
            org,
            person: Some(person(2)),
        }),
        Some(Payer { org, person: None }),
        None,
    ];
    for payer in calls {
        let mut asked = request(Purpose::Main);
        asked.payer = payer;
        block_on!(gateway.complete(&asked, &mut |_| {})).unwrap();
    }
    let seen = provider.seen();
    let sent: Vec<_> = seen.iter().map(|r| r.end_user.clone()).collect();
    assert_eq!(
        sent,
        [
            Some(ids.id(org, person(1))),
            Some(ids.id(org, person(2))),
            None,
            None
        ]
    );
    for request in &seen {
        assert_eq!(
            (&request.system, &request.messages, &request.tools),
            (&seen[0].system, &seen[0].messages, &seen[0].tools),
            "the prompt is the same whoever the call is for"
        );
    }
}

#[test]
fn a_server_with_no_secret_names_no_one() {
    let provider = Fake::new("openrouter");
    let gateway = Gateway::new(Routes::new(vec![provider.target("big")]).unwrap());
    let mut asked = request(Purpose::Main);
    asked.payer = Some(Payer {
        org: org(),
        person: Some(person(1)),
    });
    block_on!(gateway.complete(&asked, &mut |_| {})).unwrap();
    assert_eq!(provider.seen()[0].end_user, None);
}

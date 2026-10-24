//! How a person's threads are grouped (`specs/session.md`, "Watching a list"): by project until
//! they choose, their choice kept in their settings ledger, so every server on one store, and
//! so every browser of theirs, reads the same, and no one else's is changed by it.

use glave_event::Grouping;
use glave_ports::Caller;
use glave_session::protocol::{Reply, Request};
use glave_session::signin::person;
use serde_json::json;

use crate::support::*;

#[test]
fn its_answer_reads_back_as_itself_over_the_wire() {
    let reply: Reply = serde_json::from_value(json!({"grouping": "status"})).unwrap();
    assert!(matches!(
        reply,
        Reply::Grouping {
            grouping: Grouping::Status
        }
    ));
}

/// How `server` groups `caller`'s threads.
async fn grouping(server: &Tested<Shared>, caller: &Caller) -> Grouping {
    match server
        .request_as(caller, Request::GroupingGet {})
        .await
        .unwrap()
    {
        Reply::Grouping { grouping } => grouping,
        other => panic!("how the threads are grouped, not {other:?}"),
    }
}

/// Someone signed in from elsewhere, a person of their own.
fn grace() -> Caller {
    Caller {
        person: Some(person("https://id.example.com", "grace").unwrap()),
        ..Caller::default()
    }
}

#[test]
fn by_project_until_the_person_chooses_then_their_choice_on_every_server_and_theirs_alone() {
    let (one, other) = (folder("grouping-one"), folder("grouping-other"));
    block_on!(async {
        let log = Shared::default();
        let (first, _) = Setup::new(Vec::new()).build_on(&one, log.clone()).await;
        let (second, _) = Setup::new(Vec::new()).build_on(&other, log).await;
        let (ada, grace) = (Caller::default(), grace());
        assert_eq!(grouping(&first, &ada).await, Grouping::Project);

        let set = first
            .request_as(
                &ada,
                Request::GroupingSet {
                    grouping: Grouping::Status,
                },
            )
            .await;
        assert!(matches!(
            set,
            Ok(Reply::Grouping {
                grouping: Grouping::Status
            })
        ));
        assert_eq!(grouping(&second, &ada).await, Grouping::Status);
        assert_eq!(grouping(&second, &grace).await, Grouping::Project);

        // Kept beside their language, each choice after the other's, each read on its own.
        first
            .request_as(
                &ada,
                Request::LanguageSet {
                    language: Some("en-XA".into()),
                },
            )
            .await
            .unwrap();
        second
            .request_as(
                &ada,
                Request::GroupingSet {
                    grouping: Grouping::Project,
                },
            )
            .await
            .unwrap();
        assert_eq!(grouping(&first, &ada).await, Grouping::Project);
        let language = first.request_as(&ada, Request::LanguageGet {}).await;
        assert!(matches!(
            language,
            Ok(Reply::Language { language: Some(tag) }) if tag == "en-XA"
        ));
    });
}

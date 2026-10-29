//! Whether glave remembers for the person (`specs/memory.md`, "Defaults and policy"): on until
//! they turn it off, their choice kept in their settings ledger and run by from their threads'
//! next turn; what they saved still theirs to see; and never theirs to choose where memory does
//! not run or they are not the person on this computer.

use glave_event::{Auth, AuthMethod};
use glave_ports::Caller;
use glave_ports::memories::REMEMBER;
use glave_session::Answering;
use glave_session::protocol::{RememberingInfo, Reply, Request, code};
use glave_session::signin::person;

use crate::memory::served_seeing;
use crate::support::*;

async fn remembering<L: glave_ports::EventLog + 'static>(
    server: &Tested<L>,
    caller: &Caller,
) -> RememberingInfo {
    match server
        .request_as(caller, Request::MemoryGet {})
        .await
        .unwrap()
    {
        Reply::Remembering { remembering } => remembering,
        other => panic!("whether glave remembers, not {other:?}"),
    }
}

/// Whether the model was offered memory's tools in the `n`th request it received.
fn offered(seen: &Seen, n: usize) -> bool {
    json(seen).get(n).unwrap().contains(REMEMBER)
}

/// Someone signed in from elsewhere, not the person on this computer.
fn elsewhere() -> Caller {
    Caller {
        person: Some(person("https://id.example.com", "grace").unwrap()),
        auth: Some(Auth {
            method: AuthMethod::Oidc,
            issuer: Some("https://id.example.com".into()),
            subject: Some("grace".into()),
            session: None,
        }),
        trace: None,
    }
}

#[test]
fn on_until_the_person_turns_it_off_then_no_thread_reads_it_and_what_they_saved_stays() {
    block_on!(async {
        let answers = (0..3).map(|_| answer(Some("Hi."), None)).collect();
        let (server, _store, _dir, seen) = served_seeing("remembering", answers).await;
        let ada = Caller::default();
        assert_eq!(
            remembering(&server, &ada).await,
            RememberingInfo::Chosen { on: true }
        );
        let chat = start(&server, false).await;
        turn(&server, chat, "hello").await.unwrap();
        assert!(offered(&seen, 0), "memory's tools while it is on");
        let save = Request::MemorySave {
            text: "Ada prefers short answers".into(),
            scope: glave_event::MemoryScope::Project,
            project: None,
        };
        server.request(save).await.unwrap();

        let off = server.request(Request::MemorySet { on: false }).await;
        assert!(matches!(
            off,
            Ok(Reply::Remembering {
                remembering: RememberingInfo::Chosen { on: false }
            })
        ));
        assert_eq!(
            remembering(&server, &ada).await,
            RememberingInfo::Chosen { on: false }
        );
        turn(&server, chat, "again").await.unwrap();
        assert!(!offered(&seen, 1), "no memory tool once it is off");
        let list = Request::MemoryList {
            scope: None,
            project: None,
        };
        match server.request(list).await.unwrap() {
            Reply::Memories { memories } => assert_eq!(memories.len(), 1, "theirs to see"),
            other => panic!("{other:?}"),
        }

        server
            .request(Request::MemorySet { on: true })
            .await
            .unwrap();
        turn(&server, chat, "and again").await.unwrap();
        assert!(offered(&seen, 2), "on again from the next turn");
    });
}

#[test]
fn not_theirs_to_choose_from_elsewhere_or_where_memory_does_not_run() {
    block_on!(async {
        let (shared, _store, _dir, _) = served_seeing("remembering-elsewhere", Vec::new()).await;
        let grace = elsewhere();
        assert!(matches!(
            remembering(&shared, &grace).await,
            RememberingInfo::Never { .. }
        ));
        let refused = shared
            .request_as(&grace, Request::MemorySet { on: false })
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);

        let dir = folder("remembering-none");
        // As a policy that turns memory off leaves a server.
        let without = server(&dir, Vec::new(), Answering::NoOne).await;
        let RememberingInfo::Never { reason } = remembering(&without, &Caller::default()).await
        else {
            panic!("never, where memory does not run");
        };
        assert_eq!(reason.to_string(), "memory is off for this server");
        let refused = without
            .request(Request::MemorySet { on: false })
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::NOT_AVAILABLE);
    });
}

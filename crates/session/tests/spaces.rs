//! Spaces over the session API: a server serves many spaces, a thread is for whoever started
//! it, a caller who does not work in a thread's space can neither read nor answer it, a one-off
//! chat lands in the caller's personal space with a scratch folder of its own, and a local turn
//! sends the model exactly what it always did.

use glave_event::{CallId, Principal, ThreadId};
use glave_ports::SpaceId;
use glave_session::protocol::{Answer, Meta, Reply, Request, code};
use glave_session::signin::person;
use glave_session::{Answering, Caller};
use serde_json::{Value, json};

use crate::support::issuer::{Issuer, Signer};
use crate::support::web::{Web, serve_http};
use crate::support::*;

/// The requests a local turn sends, with the test's folder as `<folder>`: pinned from
/// before spaces were records, so serving many spaces changes nothing the model sees.
const LOCAL_TURN: &str = include_str!("fixtures/local_turn_requests.json");

#[test]
fn an_ordinary_local_turn_sends_the_model_exactly_what_it_did_before_spaces() {
    let dir = folder("spaces-local-turn");
    std::fs::write(dir.join("notes.txt"), "a line\n").unwrap();
    block_on!(async {
        let answers = vec![
            answer(None, Some(("read", json!({"path": "notes.txt"})))),
            answer(Some("It says a line."), None),
        ];
        let (server, seen) = Setup::new(answers).build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "go").await.unwrap();
        let folder = dir.to_string_lossy().into_owned();
        let sent: Vec<String> = seen
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.replace(&folder, "<folder>"))
            .collect();
        assert_eq!(format!("{}\n", sent.join("\n")), LOCAL_TURN);
    });
}

/// The error code of an answer, if it is an error.
fn refused(answer: &Value) -> Option<i64> {
    answer["error"]["code"].as_i64()
}

fn listing(space: Option<SpaceId>) -> Request {
    Request::ThreadList {
        space,
        limit: None,
        show: None,
        cursor: None,
    }
}

fn events_of(thread: ThreadId) -> Request {
    Request::ThreadEvents {
        thread,
        after: None,
        limit: None,
    }
}

/// Every request that reads, answers or changes `thread`, or the server's own space.
fn on_a_thread(thread: ThreadId) -> Vec<Request> {
    let call = CallId::new("call_1").unwrap();
    vec![
        Request::ThreadGet { thread },
        events_of(thread),
        Request::TurnStart {
            thread,
            prompt: "hello".into(),
            background: false,
            budget: None,
            meta: Meta::default(),
            model: None,
            approval: None,
        },
        Request::ApprovalDecide {
            thread,
            approval: call.clone(),
            decision: Answer::AllowOnce,
        },
        Request::QuestionAnswer {
            thread,
            question: call,
            answers: vec!["yes".into()],
        },
        Request::GoalGet { thread },
        Request::ThreadArchive { thread },
        Request::ThreadDelete { thread },
        Request::TurnCancel { thread },
        Request::TasksStop { thread },
    ]
}

/// Runs a turn over HTTP and answers how it ended.
async fn turn_over(web: &mut Web, thread: ThreadId, prompt: &str) -> Value {
    let (message, asked) = web.message(&Request::TurnStart {
        thread,
        prompt: prompt.into(),
        background: false,
        budget: None,
        meta: Meta::default(),
        model: None,
        approval: None,
    });
    web.post(&message)
        .await
        .messages
        .into_iter()
        .find(|message| message["id"] == asked)
        .unwrap()
}

#[test]
fn one_person_can_neither_read_nor_answer_anothers_threads() {
    let dir = folder("spaces-strangers");
    block_on!(async {
        let issuer = Issuer::start().await;
        let signer = Signer::es256("key-1");
        issuer.publish(&signer);
        let served = serve_http(server(&dir, Vec::new(), Answering::Clients).await, &issuer).await;
        // ada works in the server's own space; grace does not.
        let mut ada = Web::new(&served, Some(signer.sign(&issuer.claims("ada"))));
        let mut grace = Web::new(&served, Some(signer.sign(&issuer.claims("grace"))));
        let start = Request::ThreadStart {
            space: None,
            worktree: None,
            background: false,
            computer: None,
            model: None,
            approval: None,
        };
        let theirs = ada.call(&start).await["result"]["thread"]["id"].clone();
        let thread = serde_json::from_value(theirs.clone()).unwrap();
        for attempt in &on_a_thread(thread) {
            let answer = grace.call(attempt).await;
            assert_eq!(
                refused(&answer),
                Some(code::NOT_ALLOWED),
                "{}: {answer}",
                attempt.method()
            );
        }
        let home = SpaceId::from_uuid(uuid::Uuid::from_u128(2));
        let listed = grace.call(&listing(Some(home))).await;
        assert_eq!(refused(&listed), Some(code::NOT_ALLOWED), "{listed}");
        // Her own listing is of her own space, without ada's thread.
        let hers = grace.call(&listing(None)).await;
        assert_eq!(hers["result"]["threads"], json!([]), "{hers}");
        // So are her tasks and schedules: none of ada's, which she cannot delete.
        let schedule = ada
            .call(&Request::ScheduleCreate {
                space: None,
                when: glave_event::When::Cron("0 9 * * *".into()),
                prompt: "triage".into(),
                goal: false,
                budget: 1_000,
            })
            .await["result"]["schedule"]["schedule"]
            .clone();
        let listed = ada.call(&Request::ScheduleList {}).await;
        assert_eq!(listed["result"]["schedules"][0]["schedule"], schedule);
        let hers = grace.call(&Request::ScheduleList {}).await;
        assert_eq!(hers["result"]["schedules"], json!([]), "{hers}");
        let hers = grace.call(&Request::TasksList { limit: None }).await;
        assert_eq!(hers["result"]["tasks"], json!([]), "{hers}");
        let deleted = grace
            .call(&Request::ScheduleDelete {
                schedule: serde_json::from_value(schedule).unwrap(),
            })
            .await;
        assert_eq!(refused(&deleted), Some(code::NO_SCHEDULE), "{deleted}");
        // ada still reads her thread, recorded as hers.
        let read = ada.call(&events_of(thread)).await;
        let ada_person = person(&issuer.url, "ada").unwrap();
        assert_eq!(
            read["result"]["events"][0]["data"]["for"],
            json!({"kind": "person", "id": ada_person})
        );
        served.close().await;
    });
}

#[test]
fn a_one_off_chat_lands_in_the_callers_personal_space_with_a_scratch_folder() {
    let dir = folder("spaces-personal");
    block_on!(async {
        let issuer = Issuer::start().await;
        let signer = Signer::es256("key-1");
        issuer.publish(&signer);
        let answers = vec![
            answer(
                None,
                Some(("write", json!({"path": "note.md", "content": "hi"}))),
            ),
            answer(Some("Saved."), None),
        ];
        let mut setup = Setup::new(answers);
        setup.answering = Answering::NoOne;
        let (tested, _) = setup.build(&dir).await;
        let served = serve_http(tested, &issuer).await;
        let mut grace = Web::new(&served, Some(signer.sign(&issuer.claims("grace"))));
        let mut ada = Web::new(&served, Some(signer.sign(&issuer.claims("ada"))));
        let spaces = grace.call(&Request::SpaceList { limit: None }).await;
        assert_eq!(
            spaces["result"]["spaces"],
            json!([]),
            "none until she starts"
        );
        let started = grace
            .call(&Request::ThreadStart {
                space: None,
                worktree: None,
                background: false,
                computer: None,
                model: None,
                approval: None,
            })
            .await;
        let id = started["result"]["thread"]["id"].clone();
        let thread = serde_json::from_value(id.clone()).unwrap();
        let spaces = grace.call(&Request::SpaceList { limit: None }).await;
        let listed = spaces["result"]["spaces"].as_array().unwrap().clone();
        assert_eq!(listed.len(), 1, "{spaces}");
        assert_eq!(listed[0]["kind"], "personal");
        assert_eq!(listed[0]["name"], glave_session::PERSONAL_NAME);
        let grace_person = person(&issuer.url, "grace").unwrap();
        assert_eq!(
            listed[0]["owner"],
            json!({"kind": "person", "id": grace_person})
        );
        let ran = turn_over(&mut grace, thread, "save a note").await;
        assert_eq!(ran["result"]["turn"]["outcome"], "completed", "{ran}");
        let events = grace.call(&events_of(thread)).await;
        let bound = &events["result"]["events"][1]["data"];
        assert_eq!(bound["opened"]["kind"], "scratch", "{bound}");
        let root = std::path::PathBuf::from(bound["root"].as_str().unwrap());
        assert!(
            root.starts_with(dir.with_extension("scratch").canonicalize().unwrap()),
            "{root:?}"
        );
        assert!(
            root.join("note.md").exists(),
            "written in the scratch folder"
        );
        assert!(
            !dir.join("note.md").exists(),
            "never in the server's folder"
        );
        // ada works in the server's own space, not in grace's.
        let peek = ada.call(&Request::ThreadGet { thread }).await;
        assert_eq!(refused(&peek), Some(code::NOT_ALLOWED), "{peek}");
        served.close().await;
    });
}

/// A person other than the local one, calling on this computer.
fn someone(name: &str) -> Caller {
    Caller {
        person: Some(person("https://id.example.com", name).unwrap()),
        auth: None,
        trace: None,
    }
}

#[test]
fn spaces_are_made_shared_and_renamed_only_by_their_owner() {
    let dir = folder("spaces-made");
    block_on!(async {
        let server = server(&dir, Vec::new(), Answering::Clients).await;
        let (grace, linus) = (someone("grace"), someone("linus"));
        let made = server
            .request_as(
                &grace,
                Request::SpaceCreate {
                    name: "Garden".into(),
                    members: vec![Principal::person(linus.person.unwrap())],
                    folder: None,
                },
            )
            .await
            .unwrap();
        let Reply::Space { space, .. } = made else {
            panic!("a space, not {made:?}");
        };
        let Reply::Spaces { spaces } = server
            .request_as(&linus, Request::SpaceList { limit: None })
            .await
            .unwrap()
        else {
            panic!("spaces");
        };
        assert_eq!(spaces, std::slice::from_ref(&space), "a member sees it");
        // linus may start a thread there, which is his.
        let started = server
            .request_as(
                &linus,
                Request::ThreadStart {
                    space: Some(space.id),
                    worktree: None,
                    background: false,
                    computer: None,
                    model: None,
                    approval: None,
                },
            )
            .await
            .unwrap();
        assert!(matches!(started, Reply::Thread { .. }));
        let renamed = server
            .request_as(
                &linus,
                Request::SpaceRename {
                    space: space.id,
                    name: "Mine now".into(),
                },
            )
            .await;
        assert_eq!(renamed.unwrap_err().code, code::NOT_ALLOWED);
        let renamed = server
            .request_as(
                &grace,
                Request::SpaceRename {
                    space: space.id,
                    name: "Allotment".into(),
                },
            )
            .await
            .unwrap();
        let Reply::Space { space: renamed, .. } = renamed else {
            panic!("a space");
        };
        assert_eq!(renamed.name, "Allotment");
        // The local person works in neither grace's space nor her memory's.
        let local = server.request(Request::SpaceGet { space: space.id }).await;
        assert_eq!(local.unwrap_err().code, code::NOT_ALLOWED);
        let memory = server
            .request_as(&grace, Request::MemoryList { scope: None })
            .await;
        assert_eq!(memory.unwrap_err().code, code::NOT_ALLOWED);
    });
}

//! Projects over the session API: a server serves many projects, a thread is for whoever started
//! it, a caller who does not work in a thread's project can neither read nor answer it, a one-off
//! chat lands in the caller's personal project with a scratch folder of its own, as does every
//! request that names no project whatever folder the server started in, and a local turn sends
//! the model exactly what it always did.

use glave_event::{CallId, Opened, Principal, ThreadId};
use glave_ports::{Caller, ProjectId, ProjectKind};
use glave_session::Answering;
use glave_session::protocol::{Answer, Meta, Reply, Request, code};
use glave_session::signin::person;
use serde_json::{Value, json};

use crate::support::issuer::{Issuer, Signer};
use crate::support::web::{Web, serve_http};
use crate::support::*;

/// The requests a local turn sends, with the test's folder as `<folder>`: pinned from
/// before projects were records, so serving many projects changes nothing the model sees.
const LOCAL_TURN: &str = include_str!("fixtures/local_turn_requests.json");

#[test]
fn an_ordinary_local_turn_sends_the_model_exactly_what_it_did_before_projects() {
    let dir = folder("projects-local-turn");
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
        let sent: Vec<String> = json(&seen)
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

fn listing(project: Option<ProjectId>) -> Request {
    Request::ThreadList {
        project,
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

/// Every request that reads, answers or changes `thread`, or the server's own project.
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
    let dir = folder("projects-strangers");
    block_on!(async {
        let issuer = Issuer::start().await;
        let signer = Signer::es256("key-1");
        issuer.publish(&signer);
        let served = serve_http(server(&dir, Vec::new(), Answering::Clients).await, &issuer).await;
        // ada works in the server's own project; grace does not.
        let mut ada = Web::new(&served, Some(signer.sign(&issuer.claims("ada"))));
        let mut grace = Web::new(&served, Some(signer.sign(&issuer.claims("grace"))));
        let start = Request::ThreadStart {
            project: None,
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
                Some(code::NO_THREAD),
                "{}: {answer}",
                attempt.method()
            );
        }
        let home = ProjectId::from_uuid(uuid::Uuid::from_u128(2));
        let listed = grace.call(&listing(Some(home))).await;
        assert_eq!(refused(&listed), Some(code::NO_PROJECT), "{listed}");
        // Her own listing is of her own project, without ada's thread.
        let hers = grace.call(&listing(None)).await;
        assert_eq!(hers["result"]["threads"], json!([]), "{hers}");
        // So are her tasks and schedules: none of ada's, which she cannot delete.
        let schedule = ada
            .call(&Request::ScheduleCreate {
                project: None,
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
fn a_one_off_chat_lands_in_the_callers_personal_project_with_a_scratch_folder() {
    let dir = folder("projects-personal");
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
        let projects = grace.call(&Request::ProjectList { limit: None }).await;
        assert_eq!(
            projects["result"]["projects"],
            json!([]),
            "none until she starts"
        );
        let started = grace
            .call(&Request::ThreadStart {
                project: None,
                worktree: None,
                background: false,
                computer: None,
                model: None,
                approval: None,
            })
            .await;
        let id = started["result"]["thread"]["id"].clone();
        let thread = serde_json::from_value(id.clone()).unwrap();
        let projects = grace.call(&Request::ProjectList { limit: None }).await;
        let listed = projects["result"]["projects"].as_array().unwrap().clone();
        assert_eq!(listed.len(), 1, "{projects}");
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
        // ada works in the server's own project, not in grace's.
        let peek = ada.call(&Request::ThreadGet { thread }).await;
        assert_eq!(refused(&peek), Some(code::NO_THREAD), "{peek}");
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

fn rename(project: ProjectId, name: &str) -> Request {
    Request::ProjectRename {
        project,
        name: name.into(),
    }
}

#[test]
fn projects_are_made_shared_and_renamed_only_by_their_owner() {
    let dir = folder("projects-made");
    block_on!(async {
        let server = server(&dir, Vec::new(), Answering::Clients).await;
        let (grace, linus) = (someone("grace"), someone("linus"));
        let made = server
            .request_as(
                &grace,
                Request::ProjectCreate {
                    name: "Garden".into(),
                    members: vec![Principal::person(linus.person.unwrap())],
                    folder: None,
                },
            )
            .await
            .unwrap();
        let Reply::Project { project, .. } = made else {
            panic!("a project, not {made:?}");
        };
        let Reply::Projects { projects, .. } = server
            .request_as(&linus, Request::ProjectList { limit: None })
            .await
            .unwrap()
        else {
            panic!("projects");
        };
        assert_eq!(projects, std::slice::from_ref(&project), "a member sees it");
        // linus may start a thread there, which is his.
        let started = server
            .request_as(
                &linus,
                Request::ThreadStart {
                    project: Some(project.id),
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
            .request_as(&linus, rename(project.id, "Mine now"))
            .await;
        assert_eq!(renamed.unwrap_err().code, code::NOT_ALLOWED);
        let renamed = server
            .request_as(&grace, rename(project.id, "Allotment"))
            .await
            .unwrap();
        let Reply::Project {
            project: renamed, ..
        } = renamed
        else {
            panic!("a project");
        };
        assert_eq!(renamed.name, "Allotment");
        // The local person works in neither grace's project nor her memory's.
        let local = server
            .request(Request::ProjectGet {
                project: project.id,
            })
            .await;
        assert_eq!(local.unwrap_err().code, code::NO_PROJECT);
        let memory = server
            .request_as(
                &grace,
                Request::MemoryList {
                    scope: None,
                    project: None,
                },
            )
            .await;
        assert_eq!(memory.unwrap_err().code, code::NO_PROJECT);
    });
}

#[test]
fn a_request_that_names_no_project_acts_in_the_person_s_chats_whatever_folder_the_server_started_in()
 {
    let dir = folder("projects-chats");
    block_on!(async {
        let server = server(&dir, vec![answer(Some("Hi."), None)], Answering::NoOne).await;
        let started = server
            .request(Request::ThreadStart {
                project: None,
                worktree: None,
                background: false,
                computer: None,
                model: None,
                approval: None,
            })
            .await
            .unwrap();
        let Reply::Thread { thread } = started else {
            panic!("a thread");
        };
        assert_ne!(
            thread.project,
            scope().project(),
            "not the folder's project"
        );
        let got = server.request(Request::ProjectGet {
            project: thread.project,
        });
        let Reply::Project { project, folder } = got.await.unwrap() else {
            panic!("a project");
        };
        assert_eq!((project.kind, folder), (ProjectKind::Personal, None));
        let (place, _) = opened(&server, thread.id).await;
        assert!(matches!(place, Opened::Scratch { .. }), "{place:?}");
        let ids = |reply: Reply| match reply {
            Reply::Threads { threads, .. } => threads.into_iter().map(|t| t.id).collect::<Vec<_>>(),
            other => panic!("threads, not {other:?}"),
        };
        let chats = server.request(listing(None)).await.unwrap();
        assert_eq!(ids(chats), [thread.id]);
        let project = server
            .request(listing(Some(scope().project())))
            .await
            .unwrap();
        assert_eq!(ids(project), Vec::new());
    });
}

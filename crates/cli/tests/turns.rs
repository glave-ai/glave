//! Turns through the person's server, the real binary in a fresh home against a scripted model
//! (`specs/one-server.md`, "The CLI as a client"): from a terminal, the server starts and
//! outlives it, a new folder is asked about once, an approval and a project's trust are
//! answered in place, `-c` continues the folder's thread, Ctrl-C stops the turn, and a client
//! killed mid-turn leaves the turn to finish; the home folder's turns are chats; and `glave`
//! alone opens the folder's project, its newest thread with `-c`, or the home folder's Chats.
//! With no one at the terminal and no server running, the turn runs in the command's own
//! process, answered by no one, and leaves no server behind.

use std::time::Duration;

use glave_event::{Body, decode};
use glave_provider_openai::fake::{FakeServer, Reply};
use glave_session::client::Remote;
use glave_session::protocol::{Answer, Reply as Answered, Request, Status};
use serde_json::json;

use crate::support::*;

/// How long a scripted model waits for glave's requests before a test gives up.
const PATIENCE: Duration = Duration::from_secs(60);

/// `place` where the person's settings answer with the model `m`, ask before commands run,
/// and title no threads (a title call would take one of the model's replies).
fn asking(name: &str) -> Place {
    let place = place(name);
    choose(&place, "model", "\"m\"");
    choose(&place, "approval", "\"ask\"");
    choose(&place, "titles", "false");
    place
}

/// A model that runs `touch ran.txt` once, then says `then`, then each of `after`.
fn touching(then: &str, after: &[&str]) -> FakeServer {
    let touch = json!({"command": "touch ran.txt"});
    let replies = [
        Reply::tool_call("call_1", "bash", &touch),
        Reply::text(then),
    ];
    let more = after.iter().map(|text| Reply::text(text));
    FakeServer::start(replies.into_iter().chain(more).collect()).unwrap()
}

/// `shown`, with what the person's server in `place` wrote to its log, for a failure to say why.
fn with_log(place: &Place, shown: &str) -> String {
    let log = std::fs::read_to_string(place.home.join("run/server.log")).unwrap_or_default();
    format!("{shown}\n--- the server's log ---\n{log}")
}

/// The person's server in `place`, as a program on this computer reaches it.
fn api(place: &Place) -> Remote {
    let url = std::fs::read_to_string(place.home.join("serve.url")).unwrap();
    let token = std::fs::read_to_string(place.home.join("serve.token")).unwrap();
    Remote::new(url.trim(), token.trim().to_owned()).unwrap()
}

/// The newest thread of the project `place`'s folder leads to.
async fn newest_thread(remote: &Remote, place: &Place) -> glave_event::ThreadId {
    let path = std::fs::canonicalize(&place.project).unwrap();
    let found = Request::ProjectFind {
        path: path.to_string_lossy().into_owned(),
    };
    let Answered::Projects { projects, .. } = remote.request(&found).await.unwrap() else {
        panic!("projects")
    };
    let listed = Request::ThreadList {
        project: Some(projects[0].id),
        limit: Some(1),
        show: None,
        cursor: None,
    };
    let Answered::Threads { threads, .. } = remote.request(&listed).await.unwrap() else {
        panic!("threads")
    };
    threads[0].id
}

/// `thread`'s status, once it is `wanted`; read again until it is.
async fn once(remote: &Remote, thread: glave_event::ThreadId, wanted: Status) {
    for _ in 0..10_000 {
        let Answered::Thread { thread: summary } = remote
            .request(&Request::ThreadGet { thread })
            .await
            .unwrap()
        else {
            panic!("a thread")
        };
        if summary.status == wanted {
            return;
        }
    }
    panic!("{thread} never became {wanted:?}");
}

#[test]
fn a_turn_goes_through_the_person_s_server_which_asks_once_for_a_new_folder() {
    // A short name: the socket's path must stay short enough for a socket.
    let place = asking("tn");
    let _stopped = Stopped(&place);
    let model = touching("Done.", &["Again."]);
    let mut declined = terminal(&place, &model, &["touch it"]);
    typed(&mut declined, "n\n");
    let said = declined.wait_with_output().unwrap();
    let shown = with_log(&place, &text(&said));
    assert!(said.status.success(), "{shown}");
    assert!(shown.contains("a glave project? [y/N]"), "{shown}");
    assert!(shown.contains("nothing was made"), "{shown}");
    assert!(model.received().is_empty(), "nothing ran: {shown}");

    let mut made = terminal(&place, &model, &["touch it"]);
    typed(&mut made, "y\ny\n");
    let said = made.wait_with_output().unwrap();
    let shown = text(&said);
    assert!(said.status.success(), "{shown}");
    assert!(
        shown.contains("a glave project? [y/N]"),
        "asked again: {shown}"
    );
    assert!(shown.contains("Done."), "{shown}");
    assert!(place.project.join("ran.txt").exists(), "allowed: {shown}");

    let continued = terminal(&place, &model, &["-c", "and again"]);
    let said = continued.wait_with_output().unwrap();
    let shown = text(&said);
    assert!(said.status.success(), "{shown}");
    assert!(!shown.contains("a glave project?"), "asked once: {shown}");
    assert!(shown.contains("Again."), "{shown}");
    let last = model.received().last().unwrap().body.clone();
    let asked = said_by(&last, "user");
    assert!(
        asked.iter().any(|said| said.contains("touch it")),
        "{asked:?}"
    );
    // Every terminal is gone; the server they started is not.
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_glave"))
        .args(["server", "status"])
        .env("GLAVE_HOME", &place.home)
        .output()
        .unwrap();
    assert_eq!(status.status.code(), Some(0), "{}", text(&status));
}

/// What `role` said in a request the model received.
fn said_by(body: &serde_json::Value, role: &str) -> Vec<String> {
    said(body, role)
}

#[test]
fn ctrl_c_stops_the_turn_and_the_server_goes_on() {
    let place = asking("tc");
    let _stopped = Stopped(&place);
    let model = FakeServer::start(vec![Reply::Stall]).unwrap();
    let mut turn = terminal(&place, &model, &["wait for it"]);
    typed(&mut turn, "y\n");
    assert!(model.wait_for_requests(1, PATIENCE), "the model was asked");
    typed(&mut turn, "\x03");
    let said = turn.wait_with_output().unwrap();
    assert_eq!(said.status.code(), Some(130), "{}", text(&said));
    glave_ports::block_on!(async {
        let remote = api(&place);
        let thread = newest_thread(&remote, &place).await;
        once(&remote, thread, Status::Idle).await;
    });
}

#[test]
fn a_client_killed_mid_turn_leaves_the_turn_to_finish_answered_by_any_client() {
    let place = asking("tk");
    let _stopped = Stopped(&place);
    let model = touching("Finished.", &[]);
    let mut turn = terminal(&place, &model, &["touch it"]);
    typed(&mut turn, "y\n");
    assert!(model.wait_for_requests(1, PATIENCE), "the model was asked");
    turn.kill().unwrap();
    turn.wait().unwrap();
    glave_ports::block_on!(async {
        let remote = api(&place);
        let thread = newest_thread(&remote, &place).await;
        once(&remote, thread, Status::Waiting).await;
        let page = Request::ThreadEvents {
            thread,
            after: None,
            limit: None,
        };
        let answered = remote.request(&page).await.unwrap();
        let Answered::Events { events, .. } = answered else {
            panic!("events, not {answered:?}")
        };
        let approval = events
            .into_iter()
            .find_map(|raw| match decode(raw).unwrap().body {
                Body::ApprovalRequested(asked) => Some(asked.call_id),
                _ => None,
            })
            .unwrap();
        let decide = Request::ApprovalDecide {
            thread,
            approval,
            decision: Answer::AllowOnce,
        };
        remote.request(&decide).await.unwrap();
        assert!(model.wait_for_requests(2, PATIENCE), "the turn went on");
        once(&remote, thread, Status::Idle).await;
    });
    assert!(place.project.join("ran.txt").exists());
}

#[test]
fn a_project_s_trust_is_asked_in_the_terminal_once() {
    let place = asking("tt");
    let _stopped = Stopped(&place);
    crate::project::bring_server(&place);
    let model = FakeServer::start(vec![Reply::text("Hi."), Reply::text("Hi again.")]).unwrap();
    let mut first = terminal(&place, &model, &["hello"]);
    typed(&mut first, "y\ny\n");
    let said = first.wait_with_output().unwrap();
    let shown = with_log(&place, &text(&said));
    assert!(said.status.success(), "{shown}");
    assert!(
        shown.contains("This project can run commands on your computer"),
        "{shown}"
    );
    assert!(shown.contains("Trust them?"), "{shown}");
    assert!(shown.contains("Hi."), "{shown}");
    let second = terminal(&place, &model, &["hello again"]);
    let said = second.wait_with_output().unwrap();
    let shown = text(&said);
    assert!(said.status.success(), "{shown}");
    assert!(!shown.contains("Trust them?"), "trusted once: {shown}");
    assert!(shown.contains("Hi again."), "{shown}");
}

#[test]
fn a_turn_in_the_home_folder_is_a_chat_and_asks_about_no_project() {
    let place = asking("th");
    let model = FakeServer::start(vec![Reply::text("Hello.")]).unwrap();
    // The person's server runs with the person's own environment; only this terminal's
    // home folder is the folder it runs in.
    let (_served, _) = crate::http::serve_http(&place, &model, &[]);
    let home = place.project.to_string_lossy().into_owned();
    let chat = terminal_with(&place, &model, &["hi"], &[("HOME", &home)]);
    let said = chat.wait_with_output().unwrap();
    let shown = with_log(&place, &text(&said));
    assert!(said.status.success(), "{shown}");
    assert!(!shown.contains("a glave project?"), "{shown}");
    assert!(shown.contains("Hello."), "{shown}");
    glave_ports::block_on!(async {
        let listed = api(&place)
            .request(&Request::ProjectList { limit: None })
            .await
            .unwrap();
        let Answered::Projects { projects, .. } = listed else {
            panic!("projects")
        };
        let kinds: Vec<_> = projects.iter().map(|project| project.kind).collect();
        assert_eq!(kinds, [glave_ports::ProjectKind::Personal], "only Chats");
    });
}

#[test]
fn with_no_one_at_the_terminal_a_turn_runs_here_declines_what_asks_and_leaves_no_server() {
    let place = asking("tx");
    let model = touching("I could not.", &[]);
    let said = glave(&place, &model, &["touch it"], "");
    let shown = text(&said);
    assert!(said.status.success(), "{shown}");
    assert!(
        shown.contains("not run: no one here to approve it"),
        "{shown}"
    );
    assert!(!place.project.join("ran.txt").exists(), "{shown}");
    assert!(
        !place.home.join("run/server.sock").exists(),
        "no server started"
    );
    assert!(
        !place.home.join("run/server.log").exists(),
        "no server started"
    );
}

#[test]
fn glave_alone_opens_the_folder_s_project_asking_once_and_the_home_folder_s_chats() {
    let place = asking("ta");
    let _stopped = Stopped(&place);
    let model = FakeServer::start(vec![Reply::text("Hi.")]).unwrap();
    // No screen here, so the link is printed rather than opened.
    let mut declined = terminal(&place, &model, &[]);
    typed(&mut declined, "n\n");
    let shown = text(&declined.wait_with_output().unwrap());
    assert!(shown.contains("a glave project? [y/N]"), "{shown}");
    assert!(!shown.contains("#code="), "nothing opens: {shown}");
    let mut made = terminal(&place, &model, &[]);
    typed(&mut made, "y\n");
    let shown = text(&made.wait_with_output().unwrap());
    let project = glave_ports::block_on!(async {
        let path = std::fs::canonicalize(&place.project).unwrap();
        let found = Request::ProjectFind {
            path: path.to_string_lossy().into_owned(),
        };
        let Answered::Projects { projects, .. } = api(&place).request(&found).await.unwrap() else {
            panic!("projects")
        };
        projects[0].id
    });
    assert!(shown.contains(&format!("/p/{project}#code=")), "{shown}");
    // With --continue, at the newest thread there.
    let turn = terminal(&place, &model, &["hello"]);
    assert!(turn.wait_with_output().unwrap().status.success());
    let continued = terminal(&place, &model, &["-c"]);
    let shown = text(&continued.wait_with_output().unwrap());
    let thread = glave_ports::block_on!(newest_thread(&api(&place), &place));
    assert!(shown.contains(&format!("/t/{thread}#code=")), "{shown}");
    // The home folder is never a project: its page is the person's Chats.
    let home = place.project.to_string_lossy().into_owned();
    let chats = terminal_with(&place, &model, &[], &[("HOME", &home)]);
    let shown = text(&chats.wait_with_output().unwrap());
    assert!(!shown.contains("a glave project?"), "{shown}");
    assert!(shown.contains("/#code="), "{shown}");
    assert!(!shown.contains("/p/"), "{shown}");
}

#[test]
fn a_model_glave_does_not_run_is_refused_naming_those_it_does() {
    let place = asking("tm");
    let model = FakeServer::start(Vec::new()).unwrap();
    let (_served, _) = crate::http::serve_http(&place, &model, &[]);
    let said = glave(&place, &model, &["--model", "nobody's", "hi"], "");
    let shown = text(&said);
    assert!(!said.status.success(), "{shown}");
    assert!(shown.contains("it runs m"), "{shown}");
    assert!(model.received().is_empty(), "{shown}");
}

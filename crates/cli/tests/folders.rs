//! A folder's project, the real binary in a fresh home (`specs/one-server.md`, "A folder's
//! project"): the home folder is never one, even as a repository, so a turn there is a chat;
//! the commands that list make none, in the home folder or a new one, with no server running or
//! through the person's; and a task, a schedule or `glave` alone in a new folder asks first at
//! a terminal, and with no one there to ask makes none.

use glave_ports::ProjectKind;
use glave_provider_openai::fake::{FakeServer, Reply};
use glave_session::protocol::{Reply as Answered, Request};

use crate::http::{only_the_server_answers, serve_http};
use crate::support::*;
use crate::turns::{api, asking};

/// The commands that list.
const LISTING: [&[&str]; 4] = [&["threads"], &["memory"], &["tasks"], &["schedule", "list"]];

/// What a command that finds no project for its folder says.
pub(crate) const NO_PROJECT: &str = "this folder isn't a glave project";

/// How many projects the person's server in `place` has that are not the person's Chats.
fn made(place: &Place) -> usize {
    glave_ports::block_on!(async {
        let listed = Request::ProjectList { limit: None };
        let Answered::Projects { projects, .. } = api(place).request(&listed).await.unwrap() else {
            panic!("projects")
        };
        let folders = projects
            .iter()
            .filter(|project| project.kind == ProjectKind::Project);
        folders.count()
    })
}

/// Runs every listing command in `home`, the home folder, and in `new`, a folder in it.
fn list_everything(home: &Place, new: &Place, model: &FakeServer) {
    let folder = home.project.to_string_lossy().into_owned();
    for words in LISTING {
        for (place, in_home) in [(home, true), (new, false)] {
            let running = start_with(place, model, words, &[("HOME", &folder)]);
            let output = running.wait_with_output().unwrap();
            let shown = text(&output);
            assert!(output.status.success(), "{words:?}: {shown}");
            let said = shown.contains(NO_PROJECT);
            let says = !in_home && matches!(words, ["threads" | "memory"]);
            assert_eq!(said, says, "{words:?}, home folder {in_home}: {shown}");
        }
    }
}

#[test]
fn listing_makes_no_project_in_the_home_folder_or_a_new_one_and_a_turn_at_home_is_a_chat() {
    let home = place("fl");
    choose(&home, "project_memory", "\"shared\"");
    choose(&home, "model", "\"m\"");
    choose(&home, "titles", "false");
    let new = Place {
        project: home.project.join("new"),
        home: home.home.clone(),
        lifeline: home.lifeline.clone(),
        folder: None,
    };
    std::fs::create_dir_all(&new.project).unwrap();
    let model = FakeServer::start(vec![Reply::text("Hello.")]).unwrap();
    let folder = home.project.to_string_lossy().into_owned();
    // With no server and no one at the terminal, each in its own process.
    let chat = start_with(&home, &model, &["hi"], &[("HOME", &folder)]);
    let said = chat.wait_with_output().unwrap();
    assert!(said.status.success(), "{}", text(&said));
    assert!(text(&said).contains("Hello."), "{}", text(&said));
    list_everything(&home, &new, &model);
    // Through the person's server.
    let (_served, _) = serve_http(&home, &model, &[]);
    only_the_server_answers(&home);
    list_everything(&home, &new, &model);
    // A home folder that is itself a repository is no project either: in a folder in it,
    // the commands mean the person's Chats.
    std::fs::create_dir_all(home.project.join(".git")).unwrap();
    let threads = start_with(&new, &model, &["threads"], &[("HOME", &folder)]);
    let output = threads.wait_with_output().unwrap();
    let shown = text(&output);
    assert!(output.status.success(), "{shown}");
    assert!(!shown.contains(NO_PROJECT), "{shown}");
    assert_eq!(shown.lines().count(), 1, "the chat: {shown}");
    assert_eq!(made(&home), 0, "only the person's Chats");
}

#[test]
fn a_task_a_schedule_or_glave_alone_in_a_new_folder_asks_first_and_none_is_made_unasked() {
    let place = asking("fa");
    let _stopped = Stopped(&place);
    let model = FakeServer::start(Vec::new()).unwrap();
    let mut declined = terminal(&place, &model, &["-b", "tidy the notes"]);
    typed(&mut declined, "n\n");
    let shown = text(&declined.wait_with_output().unwrap());
    assert!(shown.contains("a glave project? [y/N]"), "{shown}");
    assert!(shown.contains("nothing was made"), "{shown}");
    assert_eq!(made(&place), 0);
    // The person's server the terminal started still runs; with no one at the terminal, no
    // one is asked, and nothing is made: `glave` alone opens the person's Chats (no screen
    // here, so no browser opens).
    let alone = start_with(&place, &model, &[], &[("SSH_CONNECTION", "test")]);
    let shown = text(&alone.wait_with_output().unwrap());
    assert!(shown.contains("opening your Chats"), "{shown}");
    assert!(!shown.contains("a glave project?"), "{shown}");
    let schedule = ["schedule", "add", "0 9 * * *", "check", "the", "build"];
    let refused = glave(&place, &model, &schedule, "");
    let shown = text(&refused);
    assert!(!refused.status.success(), "{shown}");
    assert!(shown.contains(NO_PROJECT), "{shown}");
    assert!(shown.contains("no one at the terminal"), "{shown}");
    assert_eq!(made(&place), 0);
    let mut made_one = terminal(&place, &model, &schedule);
    typed(&mut made_one, "y\n");
    let said = made_one.wait_with_output().unwrap();
    let shown = text(&said);
    assert!(said.status.success(), "{shown}");
    assert!(shown.contains("a glave project? [y/N]"), "{shown}");
    assert!(shown.contains("check the build"), "{shown}");
    assert_eq!(made(&place), 1);
    assert!(model.received().is_empty(), "nothing ran");
}

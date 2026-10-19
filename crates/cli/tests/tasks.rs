//! The real `glave` binary with tasks, through the person's server: `--background` hands a turn
//! to it and returns, refused when it does not run and no one is at the terminal to start it;
//! the server runs the task with no one attached; `glave tasks` lists it; `glave schedule`
//! adds, lists and removes a schedule; and the folder's task host is gone.

use glave_provider_openai::fake::{FakeServer, Reply};

use crate::http::{only_the_server_answers, serve_http};
use crate::support::*;

/// Makes the project a repository with a commit, so tasks have a copy to work on.
fn committed(place: &Place) {
    let ran = std::process::Command::new("bash")
        .args([
            "-c",
            "git init -q && git add . && \
                 git -c user.name=t -c user.email=t@example.com commit -q -m one",
        ])
        .current_dir(&place.project)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(ran.success());
}

#[test]
fn a_background_turn_is_refused_when_no_one_is_at_the_terminal_to_start_glave() {
    let place = place("background-refused");
    committed(&place);
    let model = FakeServer::start(Vec::new()).unwrap();
    let refused = glave(&place, &model, &["-b", "tidy the notes"], "");
    assert!(!refused.status.success());
    let said = text(&refused);
    assert!(
        said.contains("only a command run in a terminal starts it"),
        "{said}"
    );
    assert!(!place.home.join("run/server.sock").exists(), "none started");
    assert!(!place.home.join("run/server.log").exists(), "none started");
    assert!(model.received().is_empty(), "nothing ran");
}

#[test]
fn a_background_turn_runs_in_the_person_s_server_after_the_command_returns() {
    let place = place("background");
    let model = FakeServer::start(vec![Reply::text("Tidied.")]).unwrap();
    let (_served, _) = serve_http(&place, &model, &["--model", "m"]);
    // A task works on a copy, which a folder that is no repository cannot give.
    let refused = glave(&place, &model, &["-b", "tidy the notes"], "");
    assert!(!refused.status.success());
    let said = text(&refused);
    assert!(
        said.contains("not a git repository with a commit"),
        "{said}"
    );
    committed(&place);
    let started = glave(&place, &model, &["-b", "tidy the notes"], "");
    let shown = text(&started);
    assert!(started.status.success(), "{shown}");
    assert!(shown.contains("tidy the notes"), "{shown}");
    assert!(shown.contains("glave tasks stop"), "{shown}");
    // The server runs it; the command that started it is gone.
    only_the_server_answers(&place);
    let limit = std::time::Duration::from_secs(20);
    assert!(
        model.wait_for_requests(1, limit),
        "the task called the model"
    );
    let listed = (0..200)
        .map(|_| text(&glave(&place, &model, &["tasks"], "")))
        .find(|listed| listed.contains("completed"))
        .unwrap();
    assert!(
        listed.contains("agent") && listed.contains("tidy the notes"),
        "{listed}"
    );
    assert_eq!(model.received().len(), 1);
    the_task_host_is_gone(&place, &model);
}

/// Nothing of a folder's task host is left: no `glave serve --socket`, and in `run` only
/// what the person's server keeps.
fn the_task_host_is_gone(place: &Place, model: &FakeServer) {
    let host = glave(place, model, &["serve", "--socket"], "");
    assert!(!host.status.success(), "{}", text(&host));
    for entry in std::fs::read_dir(place.home.join("run")).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        assert!(
            name.starts_with("server.") || name == "processes",
            "{name} in run"
        );
    }
}

#[test]
fn schedules_are_added_listed_and_removed_in_the_person_s_server() {
    let place = place("schedule");
    let model = FakeServer::start(Vec::new()).unwrap();
    let (_served, _) = serve_http(&place, &model, &["--model", "m"]);
    only_the_server_answers(&place);
    let added = glave(
        &place,
        &model,
        &[
            "schedule",
            "add",
            "0 9 * * 1-5",
            "check",
            "the",
            "build",
            "--budget",
            "5000",
        ],
        "",
    );
    let shown = text(&added);
    assert!(added.status.success(), "{shown}");
    assert!(shown.contains("cron \"0 9 * * 1-5\" (UTC)"), "{shown}");
    assert!(shown.contains("budget 5.0k"), "{shown}");
    let listed = text(&glave(&place, &model, &["schedule", "list"], ""));
    assert!(listed.contains("check the build"), "{listed}");
    let id = listed.split_whitespace().next().unwrap().to_owned();
    let removed = glave(&place, &model, &["schedule", "rm", &id], "");
    assert!(removed.status.success(), "{}", text(&removed));
    let empty = text(&glave(&place, &model, &["schedule", "list"], ""));
    assert!(empty.contains("No schedules here"), "{empty}");
    let refused = glave(&place, &model, &["schedule", "add", "every day", "x"], "");
    assert!(!refused.status.success());
    assert!(text(&refused).contains("five fields"), "{}", text(&refused));
    assert!(model.received().is_empty(), "nothing ran");
}

#[test]
fn with_no_server_and_no_one_at_the_terminal_a_schedule_is_kept_by_glave_here() {
    let place = place("schedule-here");
    let model = FakeServer::start(Vec::new()).unwrap();
    let args = [
        "schedule",
        "add",
        "--at",
        "2030-01-01T09:00:00Z",
        "plan",
        "the",
        "year",
    ];
    let added = glave(&place, &model, &args, "");
    assert!(added.status.success(), "{}", text(&added));
    let listed = text(&glave(&place, &model, &["schedule", "list"], ""));
    assert!(listed.contains("plan the year"), "{listed}");
    assert!(!place.home.join("run/server.sock").exists(), "none started");
}

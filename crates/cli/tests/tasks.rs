//! The real `glave` binary with tasks: `--background` hands a turn to the folder's task host and
//! returns; the host runs it with no one attached; `glave tasks` lists it; the next session
//! here says it finished; and `glave schedule` adds, lists and removes a schedule.

#[cfg(test)]
#[path = "support/binary.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod glave_binary {
    use std::time::Duration;

    use glave_provider_openai::fake::{FakeServer, Reply};

    use super::support::*;

    /// The model every glave here uses, the task host included, from `config.toml`; no
    /// titles, whose calls would race the next run for the fake model's replies.
    fn with_model(place: &Place) {
        std::fs::create_dir_all(&place.home).unwrap();
        let config = "model = \"m\"\ntitles = false\n";
        std::fs::write(place.home.join("config.toml"), config).unwrap();
    }

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
    fn a_background_turn_runs_in_the_task_host_after_the_command_returns() {
        let place = place("background");
        with_model(&place);
        let server =
            FakeServer::start(vec![Reply::text("Tidied."), Reply::text("Hello.")]).unwrap();
        // A task works on a copy, which a folder that is no repository cannot give.
        let refused = glave(&place, &server, &["-b", "tidy the notes"], "");
        assert!(!refused.status.success());
        assert!(
            text(&refused).contains("not a git repository with a commit"),
            "{}",
            text(&refused)
        );
        committed(&place);
        let started = glave(&place, &server, &["-b", "tidy the notes"], "");
        let shown = text(&started);
        assert!(started.status.success(), "{shown}");
        assert!(shown.contains("tidy the notes"), "{shown}");
        assert!(shown.contains("glave tasks stop"), "{shown}");
        // The host runs it; the command that started it is gone.
        let mut listed = String::new();
        for _ in 0..200 {
            listed = text(&glave(&place, &server, &["tasks"], ""));
            if listed.contains("completed") {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(listed.contains("completed"), "{listed}");
        assert!(
            listed.contains("agent") && listed.contains("tidy the notes"),
            "{listed}"
        );
        assert_eq!(server.received().len(), 1);
        // The next session here says it finished, once.
        let next = glave(&place, &server, &["hi"], "");
        let said = text(&next);
        assert!(next.status.success(), "{said}");
        assert!(said.contains("task finished:"), "{said}");
        assert!(said.contains("tidy the notes"), "{said}");
    }

    #[test]
    fn schedules_are_added_listed_and_removed() {
        let place = place("schedule");
        with_model(&place);
        let server = FakeServer::start(Vec::new()).unwrap();
        let added = glave(
            &place,
            &server,
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
        let listed = text(&glave(&place, &server, &["schedule", "list"], ""));
        assert!(listed.contains("check the build"), "{listed}");
        let id = listed.split_whitespace().next().unwrap().to_owned();
        let removed = glave(&place, &server, &["schedule", "rm", &id], "");
        assert!(removed.status.success(), "{}", text(&removed));
        let empty = text(&glave(&place, &server, &["schedule", "list"], ""));
        assert!(empty.contains("No schedules here"), "{empty}");
        let refused = glave(&place, &server, &["schedule", "add", "every day", "x"], "");
        assert!(!refused.status.success());
        assert!(text(&refused).contains("five fields"), "{}", text(&refused));
        assert!(server.received().is_empty(), "nothing ran");
    }
}

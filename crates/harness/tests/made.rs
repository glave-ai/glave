//! Files a turn made for the person (`specs/session.md`, "Files it made"): as a turn that called
//! a tool that can change files ends, each document, sheet, deck or PDF in no hidden folder of
//! the thread's, changed since the turn started, is kept for the thread and named in its
//! `turn.ended`, whatever made it (a script, a command run from another folder); one too large,
//! or holding a secret's value, is named and left on the computer. A turn that only talks looks
//! at nothing; a turn keeps at most eight and counts the rest; a file one turn named isn't named
//! again by the next; while a command in the background still runs, it shows what settled at
//! once and leaves what may still be being written for once the command ends, even past a
//! restart, or as the next turn starts once glave stopping ended it.

use std::path::Path;
use std::time::{Duration, SystemTime};

use glave_event::{Body, Event, FileMade, Kept, MadeLeft, Principal, Secret, StopReason};
use glave_harness::{AllowAll, NoOneToAsk, Workspace};
use glave_ports::secrets::{Known, KnownSecrets};
use glave_ports::testing::any_secret;
use glave_ports::{BlobStore as _, Completion, InMemoryLog};
use serde_json::json;

use crate::support::*;

/// When the files the person had before a turn last changed: long before any turn.
const BEFORE: Duration = Duration::from_secs(1_700_000_000);

/// What the turns in `events` made, by turn, with what each leaves out; and what later
/// look-overs made, as the turns they name.
fn made(events: &[Event]) -> Vec<(Vec<FileMade>, Option<MadeLeft>)> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            Body::TurnEnded(ended) => Some((ended.made.clone(), ended.made_left)),
            Body::TurnMade(made) => Some((made.made.clone(), made.made_left)),
            _ => None,
        })
        .collect()
}

/// Each file's path and whether it was kept, left too large, or left holding a secret.
fn told(made: &[FileMade]) -> Vec<(&str, &str)> {
    made.iter()
        .map(|file| {
            let kept = match file.kept {
                Kept::Blob(_) => "kept",
                Kept::TooLarge => "too large",
                Kept::Secret => "secret",
                Kept::Unchecked => "unchecked",
            };
            (file.path.as_str(), kept)
        })
        .collect()
}

fn bash(command: &str) -> Completion {
    answer(
        None,
        vec![("bash", json!({ "command": command }))],
        StopReason::ToolUse,
    )
}

fn done() -> Completion {
    answer(Some("Done."), vec![], StopReason::EndTurn)
}

/// Runs one turn of `answers` on `workspace`, then reads what it made, checking each file kept
/// is kept apart from the computer.
async fn made_by(
    workspace: Workspace,
    answers: Vec<Completion>,
) -> (Vec<FileMade>, Option<MadeLeft>) {
    let (harness, _) = harness_with(
        workspace,
        answers,
        AllowAll,
        InMemoryLog::new(),
        1,
        settings(),
    );
    let mut thread = harness.start_thread(scope()).await.unwrap();
    turn(&harness, &mut thread, "make the files").await;
    let made = made(&events(harness.log(), thread.id).await);
    let (made, left) = made.into_iter().next().unwrap_or_default();
    for file in &made {
        if let Kept::Blob(blob) = &file.kept {
            let found = harness.blobs().get(&scope(), &blob.sha256).await.unwrap();
            assert!(
                found.is_some(),
                "{} kept apart from the computer",
                file.path
            );
        }
    }
    (made, left)
}

/// Writes `content` at `path` in `dir`, as the person had it before the turn.
fn had(dir: &Path, path: &str, content: &str) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + BEFORE).unwrap();
}

#[test]
fn what_a_turn_made_in_its_folder_is_kept_however_it_was_made() {
    let dir = folder("made");
    had(&dir, "given.pdf", "%PDF-1.7 given");
    had(&dir, "changed.pdf", "%PDF-1.7 changed");
    had(&dir, "out/keep", "");
    had(&dir, ".cache/keep", "");
    had(
        &dir,
        "make.sh",
        "printf 'PK made by a script' > report.docx\n",
    );
    block_on!(async {
        let answers = [
            "sh make.sh",
            "cd out && printf '%%PDF-1.7 plan' > plan.pdf",
            "printf '%%PDF-1.7 scratch' > .cache/scratch.pdf",
            "printf 'PK lock' > '~$report.docx'",
            "ln -s .cache/scratch.pdf link.pdf && ln -s /etc/hosts away.pdf",
            "cat given.pdf",
            "printf ' again' >> changed.pdf",
            "head -c 10485761 /dev/zero > big.pdf",
            "printf 'a,b' > notes.txt",
        ];
        let mut answers: Vec<Completion> = answers.into_iter().map(bash).collect();
        answers.push(done());
        let (made, left) = made_by(workspace(&dir).await, answers).await;
        assert_eq!(
            told(&made),
            [
                ("big.pdf", "too large"),
                ("changed.pdf", "kept"),
                ("out/plan.pdf", "kept"),
                ("report.docx", "kept"),
            ]
        );
        assert_eq!(left, None);
        let Some(Kept::Blob(plan)) = made.get(2).map(|file| &file.kept) else {
            panic!("the plan was kept")
        };
        assert_eq!(plan.media_type, "application/pdf");
        assert_eq!(made.first().map(|file| file.bytes), Some(10_485_761));
    });
}

#[test]
fn a_turn_that_only_talks_looks_at_nothing() {
    let dir = folder("made-talks");
    std::fs::write(dir.join("fresh.pdf"), "%PDF-1.7 fresh").unwrap();
    block_on!(async {
        let (made, left) = made_by(workspace(&dir).await, vec![done()]).await;
        assert_eq!((made, left), (vec![], None));
    });
}

#[test]
fn a_turn_keeps_eight_files_counts_the_rest_and_keeps_none_holding_a_secret() {
    let dir = folder("made-many");
    let value = any_secret();
    let known = KnownSecrets::default();
    known.set(vec![Known::new(
        "API_KEY".into(),
        Secret::new(value.clone()),
    )]);
    block_on!(async {
        let answers = vec![
            bash("for i in 1 2 3 4 5 6 7 8 9; do printf 'n,%s' \"$i\" > \"f$i.csv\"; done"),
            bash("printf 'key,%s' \"$API_KEY\" > a-key.csv"),
            done(),
        ];
        let workspace = workspace(&dir).await.hiding(known);
        let (made, left) = made_by(workspace, answers).await;
        let paths: Vec<(&str, &str)> = told(&made);
        assert_eq!(paths.len(), 8, "{paths:?}");
        assert_eq!(paths.first(), Some(&("a-key.csv", "secret")));
        assert_eq!(paths.last(), Some(&("f7.csv", "kept")));
        assert_eq!(left, Some(MadeLeft::More { files: 2 }));
    });
}

/// Writes `content` at `path` in `dir` a minute before it was written: settled.
fn settled(dir: &Path, path: &str, content: &str) {
    let path = dir.join(path);
    std::fs::write(&path, content).unwrap();
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    let written = file.metadata().unwrap().modified().unwrap();
    file.set_modified(written - Duration::from_secs(60))
        .unwrap();
}

#[test]
fn a_turn_shows_what_settled_at_once_and_what_a_command_may_still_write_once_it_ends() {
    let dir = folder("made-marked");
    settled(&dir, "early.pdf", "%PDF-1.7 early");
    block_on!(async {
        let running = answer(
            None,
            vec![("bash", json!({"command": "sleep 30", "background": true}))],
            StopReason::ToolUse,
        );
        let answers = vec![running, done()];
        let harness = harness(&dir, answers, NoOneToAsk).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "make it in the background").await;
        // What the command writes after the turn's look-over, as one it was still writing is.
        std::fs::write(dir.join("late.pdf"), "%PDF-1.7 late").unwrap();
        let logged = events(harness.log(), thread.id).await;
        let ended = logged.iter().find_map(|event| match &event.body {
            Body::TurnEnded(ended) => Some(ended.clone()),
            _ => None,
        });
        let ended = ended.unwrap();
        assert_eq!(
            told(&ended.made),
            [("early.pdf", "kept")],
            "what settled, at once"
        );
        assert!(ended.made_until_ns.is_some(), "where the look stopped");
        assert!(ended.made_waits, "what changed after is owed a look");

        // Read again from its log, as after a restart: the command ends while the thread is
        // idle, and what was made since the mark is named for the turn.
        let mut resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
        let running = resumed.commands().running_now().await;
        let commands = resumed.commands();
        assert!(
            commands
                .stop_for(running[0].id, Principal::glave())
                .await
                .unwrap()
        );
        harness.record_ended(&mut resumed).await.unwrap();
        let events = events(harness.log(), thread.id).await;
        let later = events.iter().find_map(|event| match &event.body {
            Body::TurnMade(made) => Some(made.clone()),
            _ => None,
        });
        let later = later.unwrap();
        let first = events
            .iter()
            .find(|event| matches!(event.body, Body::TurnStarted(_)))
            .unwrap();
        assert_eq!(later.turn, first.header.seq);
        assert_eq!(told(&later.made), [("late.pdf", "kept")]);
        assert!(
            later.made_until_ns > ended.made_until_ns,
            "no command runs: looked up to its end"
        );
    });
}

#[test]
fn what_a_command_glave_stopping_ended_had_made_is_its_turn_s_as_the_next_turn_starts() {
    let dir = folder("made-orphaned");
    block_on!(async {
        let running = answer(
            None,
            vec![("bash", json!({"command": "sleep 30", "background": true}))],
            StopReason::ToolUse,
        );
        let answers = vec![running, done(), done()];
        let harness = harness(&dir, answers, NoOneToAsk).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "make it in the background").await;
        // What the command wrote after the turn's look-over, before glave stopped.
        std::fs::write(dir.join("late.pdf"), "%PDF-1.7 late").unwrap();
        // Glave stops, taking the command with it, and nothing records its end: read again from
        // its log, the thread still has it running.
        let ending = harness.resume_thread(scope(), thread.id).await.unwrap();
        let running = ending.commands().running_now().await;
        let commands = ending.commands();
        assert!(
            commands
                .stop_for(running[0].id, Principal::glave())
                .await
                .unwrap()
        );
        let mut resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
        // The next turn only talks, but what the command made is looked over as it starts.
        turn(&harness, &mut resumed, "thanks").await;
        let events = events(harness.log(), thread.id).await;
        let turns: Vec<_> = events
            .iter()
            .filter(|event| matches!(event.body, Body::TurnStarted(_)))
            .map(|event| event.header.seq)
            .collect();
        let later = events.iter().find_map(|event| match &event.body {
            Body::TurnMade(made) => Some((event.header.seq, made.clone())),
            _ => None,
        });
        let (at, later) = later.unwrap();
        assert_eq!(Some(&later.turn), turns.first(), "the turn that ran it");
        assert!(
            turns.get(1).is_some_and(|second| at < *second),
            "before the next"
        );
        assert_eq!(told(&later.made), [("late.pdf", "kept")]);
    });
}

#[test]
fn a_file_one_turn_named_is_not_named_again_by_the_next() {
    let dir = folder("made-twice");
    block_on!(async {
        let answers = vec![
            bash("printf '%%PDF-1.7 first' > first.pdf"),
            done(),
            bash("printf '%%PDF-1.7 second' > second.pdf"),
            done(),
        ];
        let harness = harness(&dir, answers, NoOneToAsk).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "make the first").await;
        // The next starts at once: within the margin a look reaches back before its start.
        turn(&harness, &mut thread, "make the second").await;
        let made = made(&events(harness.log(), thread.id).await);
        let paths: Vec<Vec<(&str, &str)>> = made.iter().map(|(files, _)| told(files)).collect();
        assert_eq!(
            paths,
            [vec![("first.pdf", "kept")], vec![("second.pdf", "kept")]]
        );
    });
}

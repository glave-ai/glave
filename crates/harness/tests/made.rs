//! Files a turn made for the person (`specs/session.md`, "Files it made"): a document, sheet,
//! deck or PDF new or changed in the thread's folder between the turn's first call and its end
//! is kept for the thread and named in its `turn.ended`, whatever made it (a script, a command
//! run from another folder); one too large, or holding a secret's value, is named and left on
//! the computer. What is in a hidden folder, what a link points to, what a command only read,
//! and other kinds of file never count, and a turn keeps at most eight.

use std::path::Path;

use glave_event::{Body, Event, FileMade, Kept, Secret, StopReason};
use glave_harness::{AllowAll, Workspace};
use glave_ports::secrets::{Known, KnownSecrets};
use glave_ports::testing::any_secret;
use glave_ports::{BlobStore as _, InMemoryLog};
use serde_json::json;

use crate::support::*;

/// What the turns in `events` made, by turn.
fn made(events: &[Event]) -> Vec<Vec<FileMade>> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            Body::TurnEnded(ended) => Some(ended.made.clone()),
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
            };
            (file.path.as_str(), kept)
        })
        .collect()
}

/// Runs one turn of `commands` on `workspace`, then reads what it made.
async fn made_by(workspace: Workspace, commands: &[&str]) -> Vec<FileMade> {
    let mut answers: Vec<_> = commands
        .iter()
        .map(|command| {
            let call = ("bash", json!({ "command": command }));
            answer(None, vec![call], StopReason::ToolUse)
        })
        .collect();
    answers.push(answer(Some("Done."), vec![], StopReason::EndTurn));
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
    let made = made.into_iter().next().unwrap_or_default();
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
    made
}

fn write(dir: &Path, path: &str, content: &str) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

#[test]
fn what_a_turn_made_in_its_folder_is_kept_however_it_was_made() {
    let dir = folder("made");
    write(&dir, "given.pdf", "%PDF-1.7 given");
    write(&dir, "changed.pdf", "%PDF-1.7 changed");
    write(&dir, "out/keep", "");
    write(&dir, ".cache/keep", "");
    write(
        &dir,
        "make.sh",
        "printf 'PK made by a script' > report.docx\n",
    );
    block_on!(async {
        let commands = [
            "sh make.sh",
            "cd out && printf '%%PDF-1.7 plan' > plan.pdf",
            "printf '%%PDF-1.7 scratch' > .cache/scratch.pdf",
            "ln -s .cache/scratch.pdf link.pdf && ln -s /etc/hosts away.pdf",
            "cat given.pdf",
            "printf ' again' >> changed.pdf",
            "head -c 10485761 /dev/zero > big.pdf",
            "printf 'a,b' > notes.txt",
        ];
        let made = made_by(workspace(&dir).await, &commands).await;
        assert_eq!(
            told(&made),
            [
                ("big.pdf", "too large"),
                ("changed.pdf", "kept"),
                ("out/plan.pdf", "kept"),
                ("report.docx", "kept"),
            ]
        );
        let Some(Kept::Blob(plan)) = made.get(2).map(|file| &file.kept) else {
            panic!("the plan was kept")
        };
        assert_eq!(plan.media_type, "application/pdf");
        assert_eq!(made.first().map(|file| file.bytes), Some(10_485_761));
    });
}

#[test]
fn a_turn_keeps_eight_files_and_none_holding_a_secret() {
    let dir = folder("made-many");
    let value = any_secret();
    let known = KnownSecrets::default();
    known.set(vec![Known::new(
        "API_KEY".into(),
        Secret::new(value.clone()),
    )]);
    block_on!(async {
        let commands = [
            "for i in 1 2 3 4 5 6 7 8 9; do printf 'n,%s' \"$i\" > \"f$i.csv\"; done",
            "printf 'key,%s' \"$API_KEY\" > a-key.csv",
        ];
        let workspace = workspace(&dir).await.hiding(known);
        let made = made_by(workspace, &commands).await;
        let paths: Vec<(&str, &str)> = told(&made);
        assert_eq!(paths.len(), 8, "{paths:?}");
        assert_eq!(paths.first(), Some(&("a-key.csv", "secret")));
        assert_eq!(paths.last(), Some(&("f7.csv", "kept")));
    });
}

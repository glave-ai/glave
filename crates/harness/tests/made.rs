//! Files a turn made for the person (`specs/session.md`, "Files it made"): as a turn that called
//! a tool that can change files ends, each document, sheet, deck or PDF in no hidden folder of
//! the thread's, changed since the turn started, is kept for the thread and named in its
//! `turn.ended`, whatever made it (a script, a command run from another folder), with what was
//! read of it apart from glave; one too large, or holding a secret's value, is named and left on
//! the computer. A turn that only talks looks at nothing; a turn keeps at most eight and counts
//! the rest; while a command in the background still runs, it shows what settled at once and
//! leaves what may still be being written for once the command ends, even past a restart. Its
//! files are read by one deadline, and a turn the person stopped keeps them unread.

use std::future::{Future, pending};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use glave_event::{
    Body, Event, FileMade, Kept, MadeLeft, Outcome, Parts, Principal, Secret, StopReason,
};
use glave_harness::{AllowAll, Cancel, Harness, NoOneToAsk, Parts as HarnessParts, Workspace};
use glave_ports::secrets::{Known, KnownSecrets};
use glave_ports::testing::{CountingEntropy, ManualClock, any_secret};
use glave_ports::{
    BlobStore as _, Completion, Delta, InMemoryBlobs, InMemoryLog, MadeRead, ModelProvider,
    ModelRequest, ProviderError, ReadMade, read_all,
};
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

/// Runs one turn of `answers` on `workspace`, its files read by `reader`, then reads what it
/// made, checking each file kept is kept apart from the computer.
async fn made_by(
    workspace: Workspace,
    answers: Vec<Completion>,
    reader: Option<ReadMade>,
) -> (Vec<FileMade>, Option<MadeLeft>) {
    let (harness, _) = harness_with(
        workspace,
        answers,
        AllowAll,
        InMemoryLog::new(),
        1,
        settings(),
    );
    let harness = harness.with_reader(reader);
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
        let (made, left) = made_by(workspace(&dir).await, answers, None).await;
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
        // With no reader, a file is kept unread.
        assert!(made.iter().all(|file| file.read.is_none()));
    });
}

#[test]
fn a_turn_that_only_talks_looks_at_nothing() {
    let dir = folder("made-talks");
    std::fs::write(dir.join("fresh.pdf"), "%PDF-1.7 fresh").unwrap();
    block_on!(async {
        let (made, left) = made_by(workspace(&dir).await, vec![done()], None).await;
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
        let (made, left) = made_by(workspace, answers, None).await;
        let paths: Vec<(&str, &str)> = told(&made);
        assert_eq!(paths.len(), 8, "{paths:?}");
        assert_eq!(paths.first(), Some(&("a-key.csv", "secret")));
        assert_eq!(paths.last(), Some(&("f7.csv", "kept")));
        assert_eq!(left, Some(MadeLeft::More { files: 2 }));
    });
}

#[test]
fn what_is_read_of_a_file_is_kept_with_it_and_a_file_that_could_not_be_read_is_kept_unread() {
    let dir = folder("made-read");
    std::fs::write(dir.join("plan.csv"), "Item,Total\nFlights,780\n").unwrap();
    std::fs::write(dir.join("broken.csv"), "unreadable").unwrap();
    block_on!(async {
        let reader: ReadMade = Arc::new(|_, bytes, media_type| {
            Box::pin(async move {
                let readable = bytes.starts_with(b"Item") && media_type == "text/csv";
                readable.then(|| MadeRead {
                    title: Some("Trip budget".into()),
                    parts: Some(Parts::Sheets(1)),
                    markdown: Some("| Item | Total |".into()),
                    cut: true,
                })
            })
        });
        let (harness, _) = harness_with(
            workspace(&dir).await,
            vec![bash("true"), done()],
            AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        );
        let harness = harness.with_reader(Some(reader));
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "read them").await;
        let made = made(&events(harness.log(), thread.id).await);
        let files = made.first().map(|(files, _)| files.clone()).unwrap();
        let [broken, plan] = files.as_slice() else {
            panic!("two files: {files:?}")
        };
        assert_eq!(
            broken.read, None,
            "a file that could not be read is kept unread"
        );
        let read = plan.read.clone().unwrap();
        assert_eq!(read.title.as_deref(), Some("Trip budget"));
        assert_eq!(read.parts, Some(Parts::Sheets(1)));
        let preview = read.preview.unwrap();
        assert!(preview.cut && preview.blob.media_type == "text/markdown");
        let mut kept = harness
            .blobs()
            .get(&scope(), &preview.blob.sha256)
            .await
            .unwrap()
            .unwrap();
        let markdown = read_all(kept.bytes.as_mut(), 1024).await.unwrap();
        assert_eq!(markdown, b"| Item | Total |");
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
        // Changed later than now, as a file still being written may seem to be.
        let late = "printf '%%PDF-1.7 late' > late.pdf && touch -t 203001010000 late.pdf";
        let answers = vec![bash(late), running, done()];
        let harness = harness(&dir, answers, NoOneToAsk).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "make it in the background").await;
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
        assert_eq!(
            later.made_until_ns, None,
            "no command runs: looked up to its end"
        );
    });
}

/// A model that asks for a file to be written, then stops its turn as the person does, and
/// never answers again.
struct WritesThenStops {
    cancel: Cancel,
    asked: AtomicUsize,
}

impl ModelProvider for WritesThenStops {
    fn complete(
        &self,
        _request: &ModelRequest,
        _on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> impl Future<Output = Result<Completion, ProviderError>> + Send {
        let first = self.asked.fetch_add(1, Ordering::SeqCst) == 0;
        let write = json!({"path": "plan.csv", "content": "Item,Total\n"});
        let cancel = self.cancel.clone();
        Box::pin(async move {
            if first {
                return Ok(answer(None, vec![("write", write)], StopReason::ToolUse));
            }
            cancel.cancel();
            pending().await
        })
    }
}

/// A reader that reads every file, as far as it is asked to.
fn reads_all() -> ReadMade {
    Arc::new(|_, _, _| Box::pin(async { Some(MadeRead::default()) }))
}

#[test]
fn a_turn_the_person_stopped_keeps_what_it_made_unread() {
    let dir = folder("made-stopped");
    block_on!(async {
        let cancel = Cancel::new();
        let parts = HarnessParts {
            provider: WritesThenStops {
                cancel: cancel.clone(),
                asked: AtomicUsize::new(0),
            },
            log: InMemoryLog::new(),
            blobs: InMemoryBlobs::new(),
            approver: AllowAll,
            clock: Box::new(ManualClock::ticking(1_791_000_000_000)),
            entropy: Box::new(CountingEntropy::starting_at(1)),
            workspace: workspace(&dir).await,
        };
        let harness = Harness::new(parts, settings()).with_reader(Some(reads_all()));
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let summary = harness
            .run_turn(&mut thread, "write the plan", &cancel, &mut |_| {})
            .await
            .unwrap();
        assert_eq!(summary.outcome, Outcome::Cancelled);
        assert_eq!(told(&summary.made), [("plan.csv", "kept")]);
        assert!(
            summary.made.iter().all(|file| file.read.is_none()),
            "kept unread"
        );
    });
}

#[test]
fn files_past_the_look_over_s_deadline_are_kept_unread() {
    let dir = folder("made-deadline");
    std::fs::write(dir.join("plan.csv"), "Item,Total\n").unwrap();
    block_on!(async {
        let hangs: ReadMade = Arc::new(|_, _, _| Box::pin(pending()));
        let (harness, _) = harness_with(
            workspace(&dir).await,
            vec![bash("true"), done()],
            AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        );
        let harness = harness
            .with_reader(Some(hangs))
            .reading_within(Duration::from_millis(100));
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "make it").await;
        let made = made(&events(harness.log(), thread.id).await);
        let (files, _) = made.into_iter().next().unwrap();
        assert_eq!(told(&files), [("plan.csv", "kept")]);
        assert_eq!(files.first().and_then(|file| file.read.clone()), None);
    });
}

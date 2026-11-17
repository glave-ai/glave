//! Files a turn made, read apart from glave (`specs/session.md`, "Files it made"): what is read
//! of each, its title, its parts and its preview, is kept with it; one that couldn't be read is
//! kept unread; one whose reading shows one of the person's secrets, as its text joined up does
//! where its bytes never held it whole, is left on the computer. A look-over's files are read by
//! one deadline; a turn the person stopped, or one stopped while its files are read, owes its
//! look, which the next turn's start makes.

use std::fmt::Write as _;
use std::future::{Future, pending};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use glave_event::{FileMade, Kept, Outcome, Parts, Secret, StopReason};
use glave_harness::{AllowAll, Cancel, Harness, Parts as HarnessParts};
use glave_ports::secrets::{Known, KnownSecrets};
use glave_ports::testing::{CountingEntropy, ManualClock, any_secret};
use glave_ports::{
    BlobStore as _, Completion, Delta, InMemoryBlobs, InMemoryLog, MadeRead, ModelProvider,
    ModelRequest, ProviderError, ReadMade, read_all,
};
use serde_json::json;

use crate::made::{bash, done, made, told};
use crate::support::*;

/// Runs one turn of `answers` in `dir`, its files read by `reader`, with `known` secrets hidden:
/// the files it made.
async fn read_by(
    dir: &std::path::Path,
    answers: Vec<Completion>,
    reader: ReadMade,
    known: KnownSecrets,
) -> Vec<FileMade> {
    let (harness, _) = harness_with(
        workspace(dir).await.hiding(known),
        answers,
        AllowAll,
        InMemoryLog::new(),
        1,
        settings(),
    );
    let harness = harness.with_reader(Some(reader));
    let mut thread = harness.start_thread(scope()).await.unwrap();
    turn(&harness, &mut thread, "make them").await;
    let made = made(&events(harness.log(), thread.id).await);
    made.into_iter()
        .next()
        .map(|(files, _)| files)
        .unwrap_or_default()
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

#[test]
fn a_file_whose_reading_shows_a_secret_its_bytes_never_held_whole_is_left_on_the_computer() {
    let dir = folder("made-read-secret");
    let value = any_secret();
    let known = KnownSecrets::default();
    known.set(vec![Known::new(
        "API_KEY".into(),
        Secret::new(value.clone()),
    )]);
    // Word splits the value across runs of text, and another file writes it as character
    // references: neither file's bytes hold it, but what a reader reads joins it up.
    let (head, tail) = value.split_at(value.len() / 2);
    std::fs::write(
        dir.join("split.docx"),
        format!("<w:t>{head}</w:t><w:t>{tail}</w:t>"),
    )
    .unwrap();
    let referenced = value.chars().fold(String::new(), |mut written, c| {
        let _ = write!(written, "&#{};", u32::from(c));
        written
    });
    std::fs::write(dir.join("titled.docx"), referenced).unwrap();
    std::fs::write(dir.join("clean.docx"), "<w:t>nothing to hide</w:t>").unwrap();
    block_on!(async {
        let shown = value.clone();
        let reader: ReadMade = Arc::new(move |_, bytes, _| {
            let shown = shown.clone();
            Box::pin(async move {
                let text = String::from_utf8(bytes).ok()?;
                Some(if text.starts_with("<w:t>nothing") {
                    MadeRead {
                        markdown: Some("nothing to hide".into()),
                        ..MadeRead::default()
                    }
                } else if text.starts_with("&#") {
                    MadeRead {
                        title: Some(format!("Keys: {shown}")),
                        ..MadeRead::default()
                    }
                } else {
                    MadeRead {
                        markdown: Some(format!("The key is {shown}.")),
                        ..MadeRead::default()
                    }
                })
            })
        });
        let files = read_by(&dir, vec![bash("true"), done()], reader, known).await;
        assert_eq!(
            told(&files),
            [
                ("clean.docx", "kept"),
                ("split.docx", "secret"),
                ("titled.docx", "secret")
            ]
        );
        let left = files.iter().filter(|file| file.kept == Kept::Secret);
        assert!(
            left.clone().all(|file| file.read.is_none()),
            "nothing of it kept"
        );
    });
}

/// A model that asks for a file to be written, then stops its turn as the person does, and
/// answers the next turn at once.
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
        let asked = self.asked.fetch_add(1, Ordering::SeqCst);
        let write = json!({"path": "plan.csv", "content": "Item,Total\n"});
        let cancel = self.cancel.clone();
        Box::pin(async move {
            match asked {
                0 => Ok(answer(None, vec![("write", write)], StopReason::ToolUse)),
                1 => {
                    cancel.cancel();
                    pending().await
                }
                _ => Ok(answer(Some("Done."), vec![], StopReason::EndTurn)),
            }
        })
    }
}

/// A reader that reads every file, as far as it is asked to.
fn reads_all() -> ReadMade {
    Arc::new(|_, _, _| Box::pin(async { Some(MadeRead::default()) }))
}

#[test]
fn a_turn_the_person_stopped_owes_its_look_which_the_next_turn_s_start_makes() {
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
        assert_eq!(told(&summary.made), [], "stopped, its look is owed");
        assert!(summary.made_waits);
        // The next turn's start looks for it: what the stopped turn made is that turn's, read.
        harness
            .run_turn(&mut thread, "go on", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let made = made(&events(harness.log(), thread.id).await);
        let files: Vec<FileMade> = made.into_iter().flat_map(|(files, _)| files).collect();
        assert_eq!(told(&files), [("plan.csv", "kept")]);
        assert!(files.iter().all(|file| file.read.is_some()), "and read");
    });
}

#[test]
fn a_turn_stopped_while_its_files_are_read_ends_at_once_owing_its_look() {
    let dir = folder("made-stopped-reading");
    std::fs::write(dir.join("plan.csv"), "Item,Total\n").unwrap();
    block_on!(async {
        let cancel = Cancel::new();
        // The person stops the turn while its file is read, by a reader that never ends.
        let stopping = cancel.clone();
        let reader: ReadMade = Arc::new(move |_, _, _| {
            stopping.cancel();
            Box::pin(pending())
        });
        let (harness, _) = harness_with(
            workspace(&dir).await,
            vec![bash("true"), done()],
            AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        );
        let harness = harness
            .with_reader(Some(reader))
            .reading_within(Duration::from_secs(3_600));
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let summary = harness
            .run_turn(&mut thread, "make it", &cancel, &mut |_| {})
            .await
            .unwrap();
        assert_eq!(told(&summary.made), []);
        assert!(summary.made_waits, "its look is owed");
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

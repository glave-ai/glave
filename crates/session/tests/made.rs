//! Files it made (`specs/session.md`, "Files it made"): what a turn and its worker made in the
//! thread's folder is kept with the thread; `thread/files` gives it by turn, each with its kind,
//! name, parts, whether it previews and where it is kept, in the server's sentences, as its
//! reader read it; `file/read` reads it back and `file/preview` gives the preview its turn kept;
//! a file the server doesn't preview is refused plainly, and one its reader couldn't read only
//! downloads, said why. A command that waits for the person makes a file once they allow it. A
//! message sent while what a command made is looked over stops the look and starts a turn, whose
//! start makes it. A turn the person stopped keeps what it made once it has ended, with no message
//! after it. A chat archived while glave looks over what it made is archived at once, the look
//! stopped and made again after.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::{AssetKind, Body, Outcome, Parts, RawEvent, ThreadId, decode};
use glave_ports::{MadeRead, ReadMade};
use glave_session::Answering;
use glave_session::protocol::{Answer, MadeFile, Reply, Request, code};
use serde_json::json;
use tokio::sync::Notify;

use crate::support::*;

/// A reader standing in for glave's own: a CSV file is one sheet, previewed as its text, a PDF
/// one page; a file that says it can't be read can't.
fn reader() -> ReadMade {
    Arc::new(|bytes, media_type| {
        Box::pin(async move {
            let text = String::from_utf8(bytes).ok()?;
            match media_type.as_str() {
                _ if text.contains("unreadable") => None,
                "text/csv" => Some(MadeRead {
                    parts: Some(Parts::Sheets(1)),
                    markdown: Some(text),
                    ..MadeRead::default()
                }),
                _ => Some(MadeRead {
                    parts: Some(Parts::Pages(1)),
                    ..MadeRead::default()
                }),
            }
        })
    })
}

/// A command that waits for the person, its turn stopped in the log until they allow it, makes
/// a file once it goes on: the turn that went on names it.
#[test]
fn a_file_a_command_made_after_the_person_allowed_it_is_kept() {
    let dir = folder("made-allowed");
    block_on!(async {
        let make = json!({"command": "printf 'Item,Total' > allowed.csv"});
        let script = vec![
            answer(None, Some(("bash", make))),
            answer(Some("Made it."), None),
        ];
        let mut setup = Setup::asking(script, Answering::Clients);
        setup.settings.titles = false;
        let (server, _) = setup.build(&dir).await;
        let thread = start(&server, false).await;
        let mut stream = stream_turn(&server, thread, "make it", None).await.unwrap();
        let approval = approval_asked(&mut stream).await;
        let decide = Request::ApprovalDecide {
            thread,
            approval,
            decision: Answer::AllowOnce,
        };
        server.request(decide).await.unwrap();
        stream.finish().await.unwrap();
        assert_eq!(made_names(&server, thread).await, ["allowed.csv"]);
    });
}

/// The names of the files `thread`'s turns made, as `thread/files` lists them.
async fn made_names(server: &Tested, thread: ThreadId) -> Vec<String> {
    let listed = server.request(Request::ThreadFiles { thread }).await;
    let Ok(Reply::Files { made }) = listed else {
        panic!("files, not {listed:?}")
    };
    made.iter()
        .flat_map(|turn| &turn.files)
        .map(|file| file.file_name.clone())
        .collect()
}

/// The budget the turn writes.
const TABLE: &str = "Item,Total\nFlights,780\n";

/// A turn that writes a budget, makes a PDF with a command, and has a worker write a team list.
fn script() -> Vec<glave_ports::Completion> {
    let agent = json!({"description": "list the team", "prompt": "Write team.csv."});
    let write = |path: &str, content: &str| json!({"path": path, "content": content});
    vec![
        answer(None, Some(("write", write("budget.csv", TABLE)))),
        answer(
            None,
            Some((
                "bash",
                json!({"command": "printf '%%PDF-1.7' > plan.pdf; printf unreadable > odd.pdf"}),
            )),
        ),
        answer(None, Some(("agent", agent))),
        answer(None, Some(("write", write("team.csv", "Name\nAna\n")))),
        answer(Some("Wrote it."), None),
        answer(Some("Made a budget, a plan and a team list."), None),
    ]
}

/// Each of `files` by its file's name, its kind, its line and whether it previews.
fn told(files: &[MadeFile]) -> Vec<(&str, AssetKind, &str, bool)> {
    files
        .iter()
        .map(|file| {
            let line = &*file.sentence;
            (file.file_name.as_str(), file.kind, line, file.previews)
        })
        .collect()
}

#[test]
fn what_a_turn_and_its_worker_made_is_listed_read_back_and_previewed() {
    let dir = folder("made");
    block_on!(async {
        let table = TABLE;
        let mut setup = Setup::new(script());
        setup.settings.titles = false;
        setup.settings.subagents = true;
        setup.reader = Some(reader());
        let (server, _) = setup.build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "make a budget").await.unwrap();
        let listed = server
            .request(Request::ThreadFiles { thread })
            .await
            .unwrap();
        let Reply::Files { made } = listed else {
            panic!("files, not {listed:?}")
        };
        let [turn] = made.as_slice() else {
            panic!("one turn, not {made:?}")
        };
        assert_eq!(
            told(&turn.files),
            [
                (
                    "budget.csv",
                    AssetKind::Spreadsheet,
                    "Made a spreadsheet: budget (1 sheet)",
                    true
                ),
                ("odd.pdf", AssetKind::Pdf, "Made a PDF: odd", false),
                (
                    "plan.pdf",
                    AssetKind::Pdf,
                    "Made a PDF: plan (1 page)",
                    true
                ),
                (
                    "team.csv",
                    AssetKind::Spreadsheet,
                    "Made a spreadsheet: team (1 sheet)",
                    true
                ),
            ]
        );
        let [budget, odd, plan, _] = turn.files.as_slice() else {
            panic!("four files")
        };
        assert_eq!(
            odd.unread.as_deref(),
            Some("glave couldn't read this file to show it: download it to open it"),
            "kept, it only downloads"
        );
        assert!(odd.sha256.is_some() && budget.unread.is_none());
        assert_eq!((budget.bytes, &*budget.kept), (23, "Kept with this chat"));
        assert_eq!(budget.name, "budget");
        let budget_hash = budget.sha256.clone().unwrap();

        let read = Request::FileRead {
            thread,
            sha256: budget_hash.clone(),
            offset: None,
        };
        let Reply::FilePiece { piece } = server.request(read).await.unwrap() else {
            panic!("a piece")
        };
        assert_eq!(STANDARD.decode(piece.data).unwrap(), table.as_bytes());

        let preview = |sha256| Request::FilePreview { thread, sha256 };
        let previewed = server.request(preview(budget_hash)).await;
        let Ok(Reply::FilePreview { preview: shown }) = previewed else {
            panic!("a preview, not {previewed:?}")
        };
        assert_eq!(shown.markdown, TABLE, "the preview its turn kept");
        assert!(shown.cut.is_none());
        let refused = server.request(preview(plan.sha256.clone().unwrap())).await;
        assert!(
            matches!(&refused, Err(error) if error.code == code::INVALID_PARAMS),
            "a PDF is drawn by the client: {refused:?}"
        );
    });
}

/// How long a test waits for what the server does by itself: seconds, however busy the machine.
const TOLD: Duration = Duration::from_secs(10);

/// A server whose thread's turn started a command that makes late.pdf in the background once the
/// turn's own look at it is past, with its blobs' puts held from then on, and the thread.
async fn making_late(dir: &Path) -> (Tested, SharedBlobs, ThreadId) {
    // Past the first look the tool takes at a command it starts, which would see it end.
    let late = "sleep 3; printf '%%PDF-1.7' > late.pdf";
    let answers = vec![
        answer(
            None,
            Some(("bash", json!({"command": late, "background": true}))),
        ),
        answer(Some("It is making it."), None),
        answer(Some("Here it is."), None),
    ];
    let mut setup = Setup::new(answers);
    setup.settings.titles = false;
    let blobs = setup.blobs.clone();
    let (server, _) = setup.build(dir).await;
    let thread = start(&server, false).await;
    turn(&server, thread, "make it in the background")
        .await
        .unwrap();
    // The command ends while the thread is idle: its file is looked over, and kept slowly.
    blobs.hold_puts();
    (server, blobs, thread)
}

/// A message sent while the thread is idle but for the look over what a command made once it
/// ended stops the look and starts a turn, whose start makes the look: never refused as busy,
/// nor held up behind it.
#[test]
fn a_message_sent_while_a_command_s_files_are_looked_over_starts_a_turn_after() {
    let dir = folder("made-waits");
    block_on!(async {
        let (server, blobs, thread) = making_late(&dir).await;
        tokio::time::timeout(TOLD, blobs.put_held())
            .await
            .expect("the file the command made is kept within seconds");
        let sending = stream_turn(&server, thread, "where is it?", None);
        let letting_go = async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            blobs.let_go();
        };
        let (sent, ()) = tokio::join!(sending, letting_go);
        let mut stream = sent.expect("taken, the look stopped, not refused as busy");
        stream.finish().await.unwrap();
        let events = events(&server, thread).await;
        assert_eq!(
            turns(&events),
            ["started", "ended", "made", "started", "ended"]
        );
    });
}

/// The turns' starts and ends in `events`, and the looks made after them.
fn turns(events: &[RawEvent]) -> Vec<&'static str> {
    events
        .iter()
        .filter_map(
            |event| match decode(event.clone()).map(|event| event.body) {
                Ok(Body::TurnMade(_)) => Some("made"),
                Ok(Body::TurnStarted(_)) => Some("started"),
                Ok(Body::TurnEnded(_)) => Some("ended"),
                _ => None,
            },
        )
        .collect()
}

/// A server whose turn wrote notes.csv and was stopped by the person, its blobs' puts held from
/// before the stop when `held`, and the turn's thread.
async fn stopped_after_writing(dir: &Path, held: bool) -> (Tested, SharedBlobs, ThreadId) {
    let write = json!({"path": "notes.csv", "content": TABLE});
    let answers = vec![
        answer(None, Some(("write", write))),
        answer(Some("Done."), None),
    ];
    let gate = Arc::new(Notify::new());
    let mut setup = Setup::new(answers);
    setup.settings.titles = false;
    setup.gate = Some(Arc::clone(&gate));
    setup.reader = Some(reader());
    let blobs = setup.blobs.clone();
    let (server, seen) = setup.build(dir).await;
    let thread = start(&server, false).await;
    let mut running = stream_turn(&server, thread, "write it", None)
        .await
        .unwrap();
    // The first answer writes the file; the second is never let through, so the turn runs until
    // the person stops it.
    gate.notify_one();
    asked(&seen, 2).await;
    if held {
        blobs.hold_puts();
    }
    server
        .request(Request::TurnCancel { thread })
        .await
        .unwrap();
    assert_eq!(running.finish().await.unwrap().outcome, Outcome::Cancelled);
    (server, blobs, thread)
}

/// A turn the person stops after it wrote a file owes the look over what it made: made once its
/// end is in the log, so the file is kept though they send nothing more.
#[test]
fn a_file_a_turn_the_person_stopped_made_is_kept_with_no_message_after() {
    let dir = folder("made-stopped");
    block_on!(async {
        let (server, _, thread) = stopped_after_writing(&dir, false).await;
        let looked = async {
            loop {
                let events = events(&server, thread).await;
                if turns(&events).contains(&"made") {
                    return events;
                }
                tokio::task::yield_now().await;
            }
        };
        let events = tokio::time::timeout(TOLD, looked)
            .await
            .expect("the stopped turn's files are looked over within seconds");
        assert_eq!(turns(&events), ["started", "ended", "made"]);
        assert_eq!(made_names(&server, thread).await, ["notes.csv"]);
    });
}

/// Archives `thread` while glave's own look over what it made keeps a file, held: the archive
/// answers at once, the look stopped, and the look made again after it keeps `name`.
async fn archived_while_looked_over(
    server: &Tested,
    blobs: &SharedBlobs,
    thread: ThreadId,
    name: &str,
) {
    tokio::time::timeout(TOLD, blobs.put_held())
        .await
        .expect("the look keeps the file within seconds");
    let archive = server.request(Request::ThreadArchive { thread });
    let archived = tokio::time::timeout(TOLD, archive).await;
    assert!(
        matches!(archived, Ok(Ok(Reply::Done {}))),
        "archived at once, never refused or held up by the look: {archived:?}"
    );
    blobs.let_go();
    let kept = async {
        while made_names(server, thread).await != [name] {
            tokio::task::yield_now().await;
        }
    };
    tokio::time::timeout(TOLD, kept)
        .await
        .expect("what it made is kept once it is archived");
}

#[test]
fn a_chat_archived_right_after_its_turn_is_stopped_is_archived_at_once_and_keeps_its_files() {
    let dir = folder("made-stopped-archived");
    block_on!(async {
        let (server, blobs, thread) = stopped_after_writing(&dir, true).await;
        archived_while_looked_over(&server, &blobs, thread, "notes.csv").await;
    });
}

#[test]
fn a_chat_archived_while_a_command_s_files_are_looked_over_is_archived_at_once_and_keeps_them() {
    let dir = folder("made-command-archived");
    block_on!(async {
        let (server, blobs, thread) = making_late(&dir).await;
        archived_while_looked_over(&server, &blobs, thread, "late.pdf").await;
    });
}

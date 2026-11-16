//! Files it made (`specs/session.md`, "Files it made"): what a turn and its worker made in the
//! thread's folder is kept with the thread; `thread/files` gives it by turn, each with its kind,
//! name, parts, whether it previews and where it is kept, in the server's sentences; `file/read`
//! reads it back and `file/preview` gives it as a table; a file the server doesn't preview is
//! refused plainly. A command that waits for the person makes a file once they allow it.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::AssetKind;
use glave_session::Answering;
use glave_session::protocol::{Answer, Reply, Request, code};
use serde_json::json;

use crate::support::*;

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
        let listed = server.request(Request::ThreadFiles { thread }).await;
        let Ok(Reply::Files { made }) = listed else {
            panic!("files, not {listed:?}")
        };
        let names: Vec<&str> = made
            .iter()
            .flat_map(|turn| &turn.files)
            .map(|file| file.file_name.as_str())
            .collect();
        assert_eq!(names, ["allowed.csv"]);
    });
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
            Some(("bash", json!({"command": "printf '%%PDF-1.7' > plan.pdf"}))),
        ),
        answer(None, Some(("agent", agent))),
        answer(None, Some(("write", write("team.csv", "Name\nAna\n")))),
        answer(Some("Wrote it."), None),
        answer(Some("Made a budget, a plan and a team list."), None),
    ]
}

#[test]
fn what_a_turn_and_its_worker_made_is_listed_read_back_and_previewed() {
    let dir = folder("made");
    block_on!(async {
        let table = TABLE;
        let mut setup = Setup::new(script());
        setup.settings.titles = false;
        setup.settings.subagents = true;
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
        let told: Vec<(&str, AssetKind, &str, bool)> = turn
            .files
            .iter()
            .map(|file| {
                (
                    file.file_name.as_str(),
                    file.kind,
                    &*file.sentence,
                    file.previews,
                )
            })
            .collect();
        assert_eq!(
            told,
            [
                (
                    "budget.csv",
                    AssetKind::Spreadsheet,
                    "Made a spreadsheet: budget",
                    true
                ),
                ("plan.pdf", AssetKind::Pdf, "Made a PDF: plan", true),
                (
                    "team.csv",
                    AssetKind::Spreadsheet,
                    "Made a spreadsheet: team",
                    true
                ),
            ]
        );
        let [budget, plan, _] = turn.files.as_slice() else {
            panic!("three files")
        };
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
        assert_eq!(
            shown.markdown,
            "| Item | Total |\n|---|---|\n| Flights | 780 |"
        );
        assert!(shown.cut.is_none());
        let refused = server.request(preview(plan.sha256.clone().unwrap())).await;
        assert!(
            matches!(&refused, Err(error) if error.code == code::INVALID_PARAMS),
            "a PDF is drawn by the client: {refused:?}"
        );
    });
}

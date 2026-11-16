//! Files it made (`specs/session.md`, "Files it made"): a spreadsheet a `write` made is kept
//! with the thread, `thread/files` gives it by turn with its kind, title, size and where it is
//! kept, `file/read` reads it back and `file/preview` gives it as a table; a file the server
//! doesn't preview is refused plainly.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::FileKind;
use glave_session::protocol::{Reply, Request, code};
use serde_json::json;

use crate::support::*;

#[test]
fn a_file_a_turn_made_is_listed_read_back_and_previewed() {
    let dir = folder("made");
    block_on!(async {
        let table = "Item,Total\nFlights,780\n";
        let script = vec![
            answer(
                None,
                Some(("write", json!({"path": "budget.csv", "content": table}))),
            ),
            answer(
                None,
                Some(("bash", json!({"command": "printf '%%PDF-1.7' > plan.pdf"}))),
            ),
            answer(Some("Made a budget and a plan."), None),
        ];
        let mut setup = Setup::new(script);
        setup.settings.titles = false;
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
        let [budget, plan] = turn.files.as_slice() else {
            panic!("two files, not {:?}", turn.files)
        };
        assert_eq!(
            (budget.name.as_str(), budget.title.as_str(), budget.kind),
            ("budget.csv", "budget", FileKind::Spreadsheet)
        );
        assert_eq!((budget.bytes, &*budget.kept), (23, "Kept with this chat"));
        assert_eq!(
            (plan.kind, plan.media_type.as_str()),
            (FileKind::Pdf, "application/pdf")
        );

        let read = Request::FileRead {
            thread,
            sha256: budget.sha256.clone(),
            offset: None,
        };
        let Reply::FilePiece { piece } = server.request(read).await.unwrap() else {
            panic!("a piece")
        };
        assert_eq!(STANDARD.decode(piece.data).unwrap(), table.as_bytes());

        let preview = |sha256| Request::FilePreview { thread, sha256 };
        let previewed = server.request(preview(budget.sha256.clone())).await;
        let Ok(Reply::FilePreview { markdown }) = previewed else {
            panic!("a preview, not {previewed:?}")
        };
        assert_eq!(markdown, "| Item | Total |\n|---|---|\n| Flights | 780 |");
        let refused = server.request(preview(plan.sha256.clone())).await;
        assert!(
            matches!(&refused, Err(error) if error.code == code::INVALID_PARAMS),
            "a PDF is drawn by the client: {refused:?}"
        );
    });
}

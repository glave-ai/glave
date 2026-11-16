//! A person's Library (`specs/session.md`, "Library"): each picture an image tool makes is kept
//! there, its item named in the tool's result as `asset`; `library/list` shows every project's,
//! newest first, each with the chat it came from while that chat is there; deleting the chat
//! keeps the picture, and removing it from the Library lets go of it, a removal cut short
//! finished by removing it again.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::{AssetKind, Body, Part, ThreadId, decode};
use glave_ports::testing::FaultyLog;
use glave_ports::{BlobStore as _, EventLog, UNREFERENCED_FOR_MS, digest};
use glave_session::protocol::{ApiError, FilePiece, LibraryItem, Reply, Request, code};
use serde_json::json;

use crate::support::*;

/// An answer that draws `prompt`, then one that ends the turn.
fn draws(prompt: &str) -> Vec<glave_ports::Completion> {
    vec![
        answer(None, Some(("image_generate", json!({"prompt": prompt})))),
        answer(Some("Here it is."), None),
    ]
}

async fn items<L: EventLog + 'static>(server: &Tested<L>) -> Vec<LibraryItem> {
    match server.request(Request::LibraryList {}).await.unwrap() {
        Reply::Library { items } => items,
        other => panic!("the Library, not {other:?}"),
    }
}

async fn read(server: &Tested, item: &LibraryItem) -> Result<FilePiece, ApiError> {
    let read = Request::LibraryRead {
        project: item.project,
        asset: item.asset,
        offset: None,
    };
    match server.request(read).await? {
        Reply::FilePiece { piece } => Ok(piece),
        other => panic!("a piece, not {other:?}"),
    }
}

/// `library/remove` of `item`.
fn remove(item: &LibraryItem) -> Request {
    Request::LibraryRemove {
        project: item.project,
        asset: item.asset,
    }
}

/// What the image tool's call in `thread` answered the model.
async fn result(server: &Tested, thread: ThreadId) -> serde_json::Value {
    events(server, thread)
        .await
        .into_iter()
        .find_map(|raw| match decode(raw).ok()?.body {
            Body::ToolCallCompleted(done) => match done.output.first()? {
                Part::Text { text } => serde_json::from_str(text).ok(),
                _ => None,
            },
            _ => None,
        })
        .unwrap()
}

#[test]
fn a_picture_is_kept_beyond_its_chat_until_it_is_removed() {
    let dir = folder("library-kept");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.model = Some(drawing(draws("a fox at dawn")));
        let (blobs, clock) = (setup.blobs.clone(), setup.clock.clone());
        let (server, _) = setup.build(&dir).await;
        assert!(items(&server).await.is_empty(), "nothing yet");

        let thread = start(&server, false).await;
        turn(&server, thread, "draw me a fox").await.unwrap();
        let listed = items(&server).await;
        let [item] = listed.as_slice() else {
            panic!("one picture: {listed:?}");
        };
        assert_eq!(item.kind, AssetKind::Picture);
        assert_eq!(item.name, "A fox at dawn");
        assert_eq!(item.file_name, "a-fox-at-dawn.png");
        assert_eq!(item.media_type, "image/png");
        assert_eq!(item.project, scope().project());
        assert_eq!(item.chat.as_ref().map(|chat| chat.thread), Some(thread));
        assert_eq!(
            result(&server, thread).await["files"][0]["asset"],
            format!("asset:{}", item.asset)
        );
        let pixel = STANDARD.decode(PIXEL).unwrap();
        let piece = read(&server, item).await.unwrap();
        assert_eq!(STANDARD.decode(&piece.data).unwrap(), pixel);

        // Deleting the chat keeps the picture, however long after.
        let delete = Request::ThreadDelete { thread };
        assert!(matches!(server.request(delete).await, Ok(Reply::Done {})));
        clock.advance_millis(2 * UNREFERENCED_FOR_MS);
        blobs.sweep(scope().org()).await.unwrap();
        let listed = items(&server).await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].chat, None, "its chat is gone");
        assert!(read(&server, item).await.is_ok(), "the Library holds it");

        // Removed, it is gone from the Library, and nothing holds it any more.
        assert!(matches!(
            server.request(remove(item)).await,
            Ok(Reply::Done {})
        ));
        assert!(items(&server).await.is_empty());
        assert_eq!(read(&server, item).await.unwrap_err().code, code::NO_FILE);
        assert_eq!(
            server.request(remove(item)).await.unwrap_err().code,
            code::NO_FILE
        );
        let elsewhere = Request::LibraryRemove {
            project: glave_ports::ProjectId::from_uuid(uuid::Uuid::from_u128(0xdead)),
            asset: item.asset,
        };
        assert_eq!(
            server.request(elsewhere).await.unwrap_err().code,
            code::NO_PROJECT,
            "an item is named with the project it is in"
        );
        blobs.sweep(scope().org()).await.unwrap();
        let kept = blobs.get(&scope(), &digest(&pixel)).await.unwrap();
        assert!(kept.is_none(), "let go of");
    });
}

#[test]
fn the_library_shows_every_project_newest_first() {
    let dir = folder("library-projects");
    block_on!(async {
        let mut answers = draws("a fox");
        answers.extend(draws("a hare"));
        let mut setup = Setup::new(Vec::new());
        setup.model = Some(drawing(answers));
        let (server, _) = setup.build(&dir).await;
        let first = start(&server, false).await;
        turn(&server, first, "draw a fox").await.unwrap();
        let chats = Request::ThreadStart {
            project: None,
            background: false,
            worktree: None,
            computer: None,
            model: None,
            permissions: None,
            effort: None,
        };
        let Ok(Reply::Thread { thread: chat }) = server.request(chats).await else {
            panic!("a chat");
        };
        turn(&server, chat.id, "draw a hare").await.unwrap();

        let listed = items(&server).await;
        let names: Vec<&str> = listed.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, ["A hare", "A fox"], "newest first");
        assert!(
            listed[0].made_at > listed[1].made_at,
            "newest first, by when each was made"
        );
        assert_eq!(listed[0].project, chat.project, "the person's Chats");
        assert_eq!(listed[1].project, scope().project());
        assert_ne!(listed[0].asset, listed[1].asset);
    });
}

#[test]
fn a_removal_cut_short_is_finished_by_removing_again() {
    let dir = folder("library-retried");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.model = Some(drawing(draws("a fox")));
        let (blobs, clock) = (setup.blobs.clone(), setup.clock.clone());
        let log = FaultyLog::default();
        let (server, _) = setup.build_on(&dir, log.clone()).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "draw a fox").await.unwrap();
        let listed = items(&server).await;
        let [item] = listed.as_slice() else {
            panic!("one picture: {listed:?}");
        };
        let delete = Request::ThreadDelete { thread };
        assert!(matches!(server.request(delete).await, Ok(Reply::Done {})));

        // Letting go of it fails: nothing is recorded, so it is still listed to remove again.
        blobs.fail_release();
        assert!(server.request(remove(item)).await.is_err());
        assert_eq!(items(&server).await.len(), 1, "still in the Library");
        // Let go of, then its record fails: still listed, so it can be removed again.
        log.fail_with("library.removed");
        assert!(server.request(remove(item)).await.is_err());
        assert_eq!(items(&server).await.len(), 1, "still in the Library");
        assert!(matches!(
            server.request(remove(item)).await,
            Ok(Reply::Done {})
        ));
        assert!(items(&server).await.is_empty());
        clock.advance_millis(2 * UNREFERENCED_FOR_MS);
        blobs.sweep(scope().org()).await.unwrap();
        let pixel = STANDARD.decode(PIXEL).unwrap();
        let kept = blobs.get(&scope(), &digest(&pixel)).await.unwrap();
        assert!(kept.is_none(), "nothing holds it any more");
    });
}

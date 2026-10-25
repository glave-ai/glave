//! Files added to a message through the session API: `file/upload` keeps one in the thread's
//! project, `turn/start` names it, and the model is sent it; a video, a file too large, one
//! not kept here, one in another project, and an image for a model that does not read images
//! are refused plainly, before anything is sent.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::{Body, Part, ThreadId, decode};
use glave_session::protocol::{
    ApiError, AttachedFile, FileInfo, Meta, ModelInfo, Reply, Request, code,
};

use crate::support::*;

async fn upload<L: glave_ports::EventLog + 'static>(
    server: &Tested<L>,
    name: &str,
    media_type: &str,
    bytes: &[u8],
) -> Result<FileInfo, ApiError> {
    let uploaded = server
        .request(Request::FileUpload {
            project: Some(scope().project()),
            name: name.into(),
            media_type: media_type.into(),
            data: STANDARD.encode(bytes),
        })
        .await?;
    match uploaded {
        Reply::File { file } => Ok(file),
        other => panic!("a file, not {other:?}"),
    }
}

fn attached(file: &FileInfo) -> AttachedFile {
    AttachedFile {
        sha256: file.sha256.clone(),
        name: file.name.clone(),
    }
}

async fn send<L: glave_ports::EventLog + 'static>(
    server: &Tested<L>,
    thread: ThreadId,
    files: Vec<AttachedFile>,
    model: Option<&str>,
) -> Result<Reply, ApiError> {
    server
        .request(Request::TurnStart {
            thread,
            prompt: "what is this?".into(),
            files,
            background: false,
            budget: None,
            meta: Meta::default(),
            model: model.map(str::to_owned),
            effort: None,
            permissions: None,
            unattended: false,
        })
        .await
}

fn setup() -> Setup {
    let mut setup = Setup::new(vec![answer(Some("A cat."), None)]);
    setup.models = vec![ModelInfo {
        images: Some(false),
        ..ModelInfo::new("text-only")
    }];
    setup
}

#[test]
fn an_uploaded_image_is_recorded_with_the_message_and_sent_to_the_model() {
    let dir = folder("files-sent");
    block_on!(async {
        let (server, seen) = setup().build(&dir).await;
        let thread = start(&server, false).await;
        let image = upload(&server, " cat.png ", "image/png", b"\x89PNG")
            .await
            .unwrap();
        assert_eq!((image.name.as_str(), image.bytes), ("cat.png", 4));
        send(&server, thread, vec![attached(&image)], None)
            .await
            .unwrap();
        let events = server
            .request(Request::ThreadEvents {
                thread,
                after: None,
                limit: None,
            })
            .await
            .unwrap();
        let Reply::Events { events, .. } = events else {
            panic!("events")
        };
        let parts = events
            .iter()
            .find_map(|raw| match decode(raw.clone()).ok()?.body {
                Body::MessageAdded(added) => Some(added.parts),
                _ => None,
            })
            .unwrap();
        assert!(
            matches!(parts.last(), Some(Part::File { blob, name })
                if blob.sha256 == image.sha256 && name.as_deref() == Some("cat.png")),
            "{parts:?}"
        );
        let requests = seen.lock().unwrap();
        let [inline] = requests.first().unwrap().files.as_slice() else {
            panic!("one image sent inline")
        };
        assert_eq!(inline.data, "iVBORw==");
    });
}

#[test]
fn files_are_refused_plainly_before_anything_is_sent() {
    let dir = folder("files-refused");
    block_on!(async {
        let (server, seen) = setup().build(&dir).await;
        let thread = start(&server, false).await;
        let video = upload(&server, "clip.mp4", "video/mp4", b"....").await;
        assert!(
            matches!(&video, Err(error) if error.code == code::INVALID_PARAMS
                && error.message.contains("videos")),
            "{video:?}"
        );
        let huge = vec![0u8; 5 * 1024 * 1024 + 1];
        let large = upload(&server, "big.png", "image/png", &huge).await;
        assert!(
            matches!(&large, Err(error) if error.code == code::LIMIT
                && error.message.contains("5 MB")),
            "{large:?}"
        );
        let image = upload(&server, "cat.png", "image/png", b"\x89PNG")
            .await
            .unwrap();
        let blind = send(&server, thread, vec![attached(&image)], Some("text-only")).await;
        assert!(
            matches!(&blind, Err(error) if error.code == code::NO_IMAGES
                && error.message.contains("can't read images")),
            "{blind:?}"
        );
        let never = AttachedFile {
            sha256: glave_ports::digest(b"never uploaded"),
            name: "notes.txt".into(),
        };
        let gone = send(&server, thread, vec![never], None).await;
        assert!(
            matches!(&gone, Err(error) if error.code == code::NO_FILE
                && error.message.contains("notes.txt")),
            "{gone:?}"
        );
        let task = server
            .request(Request::TurnStart {
                thread,
                prompt: "later".into(),
                files: vec![attached(&image)],
                background: true,
                budget: None,
                meta: Meta::default(),
                model: None,
                effort: None,
                permissions: None,
                unattended: false,
            })
            .await;
        assert!(matches!(&task, Err(error) if error.code == code::INVALID_PARAMS));
        assert!(seen.lock().unwrap().is_empty(), "nothing reached the model");
    });
}

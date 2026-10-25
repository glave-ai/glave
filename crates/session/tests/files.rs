//! Files added to a message through the session API: `file/upload` keeps one in the thread's
//! project as what it holds says it is, `turn/start` names it, and the model is sent it, a
//! photo as a copy made once beside the untouched original; a video, a file too large, one
//! glave does not read, an image that cannot be opened, one not kept here, and an image for a
//! model that does not read images are refused plainly, before anything is sent.

use std::io::Cursor;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::{Body, Part, ThreadId, decode};
use glave_session::protocol::{
    ApiError, AttachedFile, FileInfo, Meta, ModelInfo, Reply, Request, code,
};

use crate::support::*;

/// A one-pixel PNG, as a screenshot would come.
const PIXEL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

/// A stand-in for a photo: 2400 by 1800, with grain, as a JPEG.
fn photo() -> Vec<u8> {
    let mut seed: u32 = 0x9e37_79b9;
    let image = image::RgbImage::from_fn(2400, 1800, |x, y| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let grain = u8::try_from(seed % 24).unwrap();
        let shade = |value: u32, of: u32| u8::try_from(value * 200 / of).unwrap() + grain;
        image::Rgb([shade(x, 2400), shade(y, 1800), 128])
    });
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
    out.into_inner()
}

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

/// The person's message in `thread`, as the log holds it: its parts, and the event's JSON.
async fn recorded<L: glave_ports::EventLog + 'static>(
    server: &Tested<L>,
    thread: ThreadId,
) -> (Vec<Part>, String) {
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
    events
        .iter()
        .find_map(|raw| match decode(raw.clone()).ok()?.body {
            Body::MessageAdded(added) => Some((added.parts, serde_json::to_string(raw).unwrap())),
            _ => None,
        })
        .unwrap()
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
        let pixel = STANDARD.decode(PIXEL).unwrap();
        // Named as anything, it is kept as what it holds.
        let image = upload(&server, " cat.png ", "application/octet-stream", &pixel)
            .await
            .unwrap();
        assert_eq!(
            (image.name.as_str(), image.media_type.as_str()),
            ("cat.png", "image/png")
        );
        send(&server, thread, vec![attached(&image)], None)
            .await
            .unwrap();
        let (parts, _) = recorded(&server, thread).await;
        assert!(
            matches!(parts.last(), Some(Part::File { blob, name, resized: None })
                if blob.sha256 == image.sha256 && name.as_deref() == Some("cat.png")),
            "small enough to send as it is: {parts:?}"
        );
        let requests = seen.lock().unwrap();
        let [inline] = requests.first().unwrap().files.as_slice() else {
            panic!("one image sent inline")
        };
        assert_eq!(inline.data, PIXEL);
    });
}

#[test]
fn a_photo_is_kept_as_it_is_and_the_model_sent_a_copy_made_once() {
    let dir = folder("files-resized");
    block_on!(async {
        let (server, seen) = setup().build(&dir).await;
        let thread = start(&server, false).await;
        let photo = photo();
        let kept = upload(&server, "IMG_0001.jpg", "image/jpeg", &photo)
            .await
            .unwrap();
        assert_eq!(kept.bytes, photo.len() as u64, "the original, untouched");
        assert_eq!(kept.sha256, glave_ports::digest(&photo));
        send(&server, thread, vec![attached(&kept)], None)
            .await
            .unwrap();

        let (parts, json) = recorded(&server, thread).await;
        let Some(Part::File {
            blob,
            resized: Some(copy),
            ..
        }) = parts.last()
        else {
            panic!("the photo with its copy: {parts:?}")
        };
        assert_eq!((&blob.sha256, blob.bytes), (&kept.sha256, kept.bytes));
        assert_eq!(copy.media_type, "image/jpeg");
        assert!(
            copy.bytes < 600_000,
            "a few hundred kilobytes: {}",
            copy.bytes
        );
        assert!(
            json.len() < 2_000 && json.contains(copy.sha256.as_str()),
            "the log holds references, never bytes: {} bytes",
            json.len()
        );

        let requests = seen.lock().unwrap();
        let [inline] = requests.first().unwrap().files.as_slice() else {
            panic!("one image sent inline")
        };
        assert_eq!(inline.sha256, copy.sha256, "the copy, not the original");
        let sent = image::load_from_memory(&STANDARD.decode(&inline.data).unwrap()).unwrap();
        assert_eq!((sent.width(), sent.height()), (1568, 1176));
    });
}

#[test]
fn uploads_glave_cannot_read_are_refused_plainly() {
    let dir = folder("files-refused-uploads");
    block_on!(async {
        let (server, _) = setup().build(&dir).await;
        let refused = |answered: &Result<FileInfo, ApiError>, wanted: i64, words: &str| {
            assert!(
                matches!(answered, Err(error) if error.code == wanted
                    && error.message.contains(words)),
                "{answered:?}"
            );
        };
        let video = upload(&server, "clip.mp4", "video/mp4", b"....").await;
        refused(&video, code::INVALID_PARAMS, "videos");
        let huge = vec![b'a'; 10 * 1024 * 1024 + 1];
        let large = upload(&server, "big.txt", "text/plain", &huge).await;
        refused(&large, code::LIMIT, "10 MB");
        let zip = upload(
            &server,
            "code.zip",
            "application/zip",
            b"PK\x03\x04\0\0\0\0",
        )
        .await;
        refused(&zip, code::INVALID_PARAMS, "a PDF or a text file");
        let heic = upload(
            &server,
            "IMG_0002.heic",
            "image/heic",
            b"\0\0\0\x18ftypheic",
        )
        .await;
        refused(&heic, code::INVALID_PARAMS, "PNG, JPEG, GIF or WebP");
    });
}

#[test]
fn messages_with_files_that_cannot_be_sent_are_refused_before_anything_is() {
    let dir = folder("files-refused-sends");
    block_on!(async {
        let (server, seen) = setup().build(&dir).await;
        let thread = start(&server, false).await;
        let cut = STANDARD.decode(PIXEL).unwrap();
        let cut = upload(&server, "cut.png", "image/png", cut.get(..40).unwrap())
            .await
            .unwrap();
        let broken = send(&server, thread, vec![attached(&cut)], None).await;
        assert!(
            matches!(&broken, Err(error) if error.code == code::INVALID_PARAMS
                && error.message.contains("\"cut.png\" can't be opened")),
            "{broken:?}"
        );
        let mut wide = Cursor::new(Vec::new());
        let strip = image::GrayImage::new(glave_harness::MAX_SIDE + 1, 1);
        strip.write_to(&mut wide, image::ImageFormat::Png).unwrap();
        let wide = upload(&server, "strip.png", "image/png", wide.get_ref())
            .await
            .unwrap();
        let huge = send(&server, thread, vec![attached(&wide)], None).await;
        assert!(
            matches!(&huge, Err(error) if error.code == code::LIMIT
                && error.message.contains("too large to open")),
            "{huge:?}"
        );
        let pixel = STANDARD.decode(PIXEL).unwrap();
        let image = upload(&server, "cat.png", "image/png", &pixel)
            .await
            .unwrap();
        let pdf = upload(&server, "terms.pdf", "application/pdf", b"%PDF-1.7\n")
            .await
            .unwrap();
        for file in [&image, &pdf] {
            let blind = send(&server, thread, vec![attached(file)], Some("text-only")).await;
            assert!(
                matches!(&blind, Err(error) if error.code == code::NO_IMAGES
                    && error.message.contains("can't read images or PDFs")),
                "{blind:?}"
            );
        }
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

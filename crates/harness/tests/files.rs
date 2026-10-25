//! Files in a thread, sent to the model as it can read them: a person's, recorded as `file`
//! parts after their message's text, an image or a PDF inline, a text file as its text; an
//! image `read` reads, kept as a copy fit to send and handed over in its result; a message
//! read mid-turn with its files (#727); and every file by its name to a model that reads no
//! images.

use std::io::Cursor;

use glave_event::{BlobRef, Body, EventId, Part, StopReason};
use glave_harness::{AllowAll, Cancel, Live, Queued, Said};
use glave_ports::{BlobStore as _, Caller, InMemoryLog, Message, Pieces};
use serde_json::json;

use crate::support::*;

fn file(blob: BlobRef, name: &str) -> Part {
    Part::File {
        blob,
        name: Some(name.into()),
        resized: None,
    }
}

/// The person's first message as `events` record it.
fn first_message(events: &[glave_event::Event]) -> Vec<Part> {
    events
        .iter()
        .find_map(|event| match &event.body {
            Body::MessageAdded(added) => Some(added.parts.clone()),
            _ => None,
        })
        .unwrap()
}

/// A PNG of `width` by `height`, in shades.
fn png(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbImage::from_fn(width, height, |x, y| {
        image::Rgb([
            u8::try_from(x % 256).unwrap(),
            u8::try_from(y % 256).unwrap(),
            128,
        ])
    });
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

#[test]
fn an_image_and_a_pdf_go_inline_and_a_text_file_as_text() {
    let dir = folder("files");
    block_on!(async {
        let answers = vec![answer(Some("A cat."), Vec::new(), StopReason::EndTurn)];
        let (harness, seen) =
            harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let blobs = harness.blobs();
        let upload = |bytes: &'static [u8], media_type: &'static str| async move {
            blobs
                .put(
                    &scope(),
                    None,
                    media_type,
                    1024,
                    Box::new(Pieces::from(bytes.to_vec())),
                )
                .await
                .unwrap()
        };
        let files = [
            file(upload(b"\x89PNG", "image/png").await, "cat.png"),
            file(upload(b"fn main() {}", "text/x-rust").await, "main.rs"),
            file(upload(b"%PDF", "application/pdf").await, "terms.pdf"),
        ];
        let said = Said {
            text: "what is this?",
            files: &files,
        };
        harness
            .run_turn_with_files(
                &mut thread,
                said,
                &Caller::default(),
                &Cancel::new(),
                &mut |_| {},
            )
            .await
            .unwrap();

        let message = first_message(&events(harness.log(), thread.id).await);
        let mut expected = vec![Part::Text {
            text: "what is this?".into(),
        }];
        expected.extend(files.iter().cloned());
        assert_eq!(message, expected, "the files are recorded after the text");

        let requests = seen.lock().unwrap();
        let request = requests.first().unwrap();
        let Some(Message::User { parts }) = request.messages.last() else {
            panic!("the person's message is sent last")
        };
        assert_eq!(parts.first(), expected.first());
        assert_eq!(parts.get(1), files.first(), "the image stays a file part");
        assert_eq!(
            parts.get(2),
            Some(&Part::Text {
                text: "<file name=\"main.rs\">\nfn main() {}\n</file>".into()
            }),
            "a text file is sent as its text"
        );
        assert_eq!(parts.get(3), files.get(2), "the PDF stays a file part");
        let inline: Vec<(&str, &str)> = request
            .files
            .iter()
            .map(|file| (file.media_type.as_str(), file.data.as_str()))
            .collect();
        assert_eq!(
            inline,
            [("application/pdf", "JVBERg=="), ("image/png", "iVBORw==")],
            "the newest first"
        );
    });
}

/// Runs one turn of `harness` for the message "what is this?" with `files`.
async fn ask_about<P: glave_ports::ModelProvider>(
    harness: &glave_harness::Harness<P, InMemoryLog, glave_ports::InMemoryBlobs, AllowAll>,
    files: &[Part],
) -> glave_harness::TurnSummary {
    let mut thread = harness.start_thread(scope()).await.unwrap();
    let said = Said {
        text: "what is this?",
        files,
    };
    harness
        .run_turn_with_files(
            &mut thread,
            said,
            &Caller::default(),
            &Cancel::new(),
            &mut |_| {},
        )
        .await
        .unwrap()
}

/// `bytes` kept as an upload of `media_type`, as a file part named `name`.
async fn uploaded<P: glave_ports::ModelProvider>(
    harness: &glave_harness::Harness<P, InMemoryLog, glave_ports::InMemoryBlobs, AllowAll>,
    bytes: &[u8],
    media_type: &str,
    name: &str,
) -> Part {
    let blob = harness
        .blobs()
        .put(
            &scope(),
            None,
            media_type,
            1024,
            Box::new(Pieces::from(bytes.to_vec())),
        )
        .await
        .unwrap();
    file(blob, name)
}

#[test]
fn a_model_that_reads_no_images_is_told_each_is_not_shown() {
    let dir = folder("files-blind");
    block_on!(async {
        let answers = vec![answer(Some("Text only."), Vec::new(), StopReason::EndTurn)];
        let (harness, seen) =
            harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
        harness.set_without_images(vec!["test-model".into()]);
        let image = uploaded(&harness, b"\x89PNG", "image/png", "cat.png").await;
        ask_about(&harness, &[image]).await;
        let requests = seen.lock().unwrap();
        let request = requests.first().unwrap();
        assert!(request.files.is_empty(), "nothing inline");
        let Some(Message::User { parts }) = request.messages.last() else {
            panic!("the person's message is sent last")
        };
        assert_eq!(
            parts.last(),
            Some(&Part::Text {
                text: "[cat.png is not shown: this model reads no images]".into()
            })
        );
    });
}

#[test]
fn a_provider_that_refuses_what_went_inline_is_asked_again_with_names() {
    let dir = folder("files-refused");
    block_on!(async {
        let refused = glave_ports::ProviderError::Http {
            status: 400,
            body: "unsupported content type: file".into(),
        };
        let model = ScriptedModel::answering(vec![
            Err(refused),
            Ok(answer(Some("By name."), Vec::new(), StopReason::EndTurn)),
        ]);
        let script = model.seen();
        let parts = glave_harness::Parts {
            provider: model,
            log: InMemoryLog::new(),
            blobs: glave_ports::InMemoryBlobs::new(),
            approver: AllowAll,
            clock: Box::new(glave_ports::testing::ManualClock::ticking(
                1_791_000_000_000,
            )),
            entropy: Box::new(glave_ports::testing::CountingEntropy::starting_at(1)),
            workspace: workspace(&dir).await,
        };
        let harness = glave_harness::Harness::new(parts, settings());
        let pdf = uploaded(&harness, b"%PDF", "application/pdf", "terms.pdf").await;
        ask_about(&harness, &[pdf]).await;
        let asked = script.lock().unwrap();
        let files: Vec<usize> = asked.iter().map(|request| request.files.len()).collect();
        assert_eq!(files, [1, 0], "inline, then by its name");
        assert!(!harness.reads_images("test-model"), "taken as reading none");
    });
}

#[test]
fn reading_an_image_hands_the_model_a_copy_fit_to_send() {
    let dir = folder("files-read");
    let shot = png(2000, 1000);
    std::fs::write(dir.join("shot.png"), &shot).unwrap();
    block_on!(async {
        let answers = vec![
            answer(
                None,
                vec![("read", json!({"path": "shot.png"}))],
                StopReason::ToolUse,
            ),
            answer(Some("A gradient."), Vec::new(), StopReason::EndTurn),
        ];
        let (harness, seen) =
            harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        harness
            .run_turn(
                &mut thread,
                "what is in shot.png?",
                &Cancel::new(),
                &mut |_| {},
            )
            .await
            .unwrap();

        let events = events(harness.log(), thread.id).await;
        let output = events
            .iter()
            .find_map(|event| match &event.body {
                Body::ToolCallCompleted(completed) => Some(completed.output.clone()),
                _ => None,
            })
            .unwrap();
        let [
            Part::Text { text },
            Part::File {
                blob,
                name,
                resized: None,
            },
        ] = output.as_slice()
        else {
            panic!("the line saying what it is, then the image: {output:?}")
        };
        assert_eq!(
            text,
            &format!("shot.png is an image of {} bytes.", shot.len())
        );
        assert_eq!(
            (blob.media_type.as_str(), name.as_deref()),
            ("image/jpeg", Some("shot.png"))
        );

        assert!(blob.bytes < 1_000_000, "{} bytes", blob.bytes);
        let requests = seen.lock().unwrap();
        let second = requests.get(1).unwrap();
        let [sent] = second.files.as_slice() else {
            panic!("the copy inline: {:?}", second.files)
        };
        assert_eq!(sent.sha256, blob.sha256);
    });
}

#[test]
fn a_message_read_mid_turn_keeps_its_files() {
    let dir = folder("files-steer");
    block_on!(async {
        let answers = vec![
            answer(
                None,
                vec![("bash", json!({"command": "echo hi"}))],
                StopReason::ToolUse,
            ),
            answer(Some("Seen."), Vec::new(), StopReason::EndTurn),
        ];
        let (harness, seen) =
            harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let blob = harness
            .blobs()
            .put(
                &scope(),
                None,
                "image/png",
                1024,
                Box::new(Pieces::from(b"\x89PNG".to_vec())),
            )
            .await
            .unwrap();
        let queued = Queued {
            id: EventId::new(jiff::Timestamp::UNIX_EPOCH, [7; 10]).unwrap(),
            text: "and this one".into(),
            files: vec![file(blob.clone(), "cat.png")],
            caller: Caller::default(),
        };
        let inbox = thread.inbox().clone();
        let mut queue = Some(queued);
        let mut live = move |live: Live| {
            if matches!(live, Live::ToolFinished { .. })
                && let Some(queued) = queue.take()
            {
                inbox.push(queued).unwrap();
            }
        };
        harness
            .run_turn(&mut thread, "say hi", &Cancel::new(), &mut live)
            .await
            .unwrap();

        let expected = vec![
            Part::Text {
                text: "and this one".into(),
            },
            file(blob.clone(), "cat.png"),
        ];
        let events = events(harness.log(), thread.id).await;
        let recorded = events.iter().rev().find_map(|event| match &event.body {
            Body::MessageAdded(added) if added.role == glave_event::Role::User => {
                Some(added.parts.clone())
            }
            _ => None,
        });
        assert_eq!(recorded, Some(expected.clone()), "recorded with its file");
        let requests = seen.lock().unwrap();
        let second = requests.get(1).unwrap();
        assert_eq!(
            second.messages.last(),
            Some(&Message::User { parts: expected }),
            "read with its file"
        );
        assert_eq!(
            second.files.first().map(|file| &file.sha256),
            Some(&blob.sha256)
        );
    });
}

#[test]
fn an_image_made_fit_to_send_keeps_its_original_beside_the_copy() {
    let dir = folder("files-copy");
    block_on!(async {
        let (harness, _) = harness_seeing(
            &dir,
            Vec::new(),
            AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        )
        .await;
        let original = png(3000, 1500);
        let blobs = harness.blobs();
        let put = Box::new(Pieces::from(original.clone()));
        let blob = blobs
            .put(&scope(), None, "image/png", 20_000_000, put)
            .await
            .unwrap();
        let copy = harness.resized(&scope(), &blob).await.unwrap().unwrap();
        assert_eq!(copy.media_type, "image/jpeg");
        let read = |sha256| async move {
            let mut found = blobs.get(&scope(), sha256).await.unwrap().unwrap();
            glave_ports::read_all(found.bytes.as_mut(), u64::MAX)
                .await
                .unwrap()
        };
        assert_eq!(
            read(&blob.sha256).await,
            original,
            "the original, untouched"
        );
        let sent = image::load_from_memory(&read(&copy.sha256).await).unwrap();
        assert_eq!((sent.width(), sent.height()), (1568, 784));
        let small = Box::new(Pieces::from(png(64, 32)));
        let small = blobs
            .put(&scope(), None, "image/png", 20_000_000, small)
            .await
            .unwrap();
        let none = harness.resized(&scope(), &small).await.unwrap();
        assert_eq!(none, None, "sent as it is");
    });
}

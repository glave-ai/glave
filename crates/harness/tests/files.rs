//! Files a person adds to a message: recorded as `file` parts after its text, and sent to the
//! model as it can read them: an image inline, a text file as its text, anything else by name.

use glave_event::{BlobRef, Body, Part, StopReason};
use glave_harness::{AllowAll, Cancel, Said};
use glave_ports::{BlobStore as _, Caller, InMemoryLog, Message, Pieces};

use crate::support::*;

#[test]
fn an_image_goes_inline_a_text_file_as_text_and_anything_else_by_name() {
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
        let file = |blob: BlobRef, name: &str| Part::File {
            blob,
            name: Some(name.into()),
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

        let recorded = events(harness.log(), thread.id).await;
        let message = recorded
            .iter()
            .find_map(|event| match &event.body {
                Body::MessageAdded(added) => Some(added.parts.clone()),
                _ => None,
            })
            .unwrap();
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
        assert_eq!(parts.get(3), files.get(2), "a PDF is sent by its name");
        let [image] = request.files.as_slice() else {
            panic!("one image inline: {:?}", request.files)
        };
        assert_eq!(
            (image.media_type.as_str(), image.data.as_str()),
            ("image/png", "iVBORw==")
        );
    });
}

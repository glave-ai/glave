//! glave's media tools in a turn (`specs/gateway.md`, "Media tools"): offered only where the
//! provider serves them; a call made through the provider as the thread's work, recorded as a
//! model call of the turn; what it made kept whole as a blob of the thread's and handed to the
//! model as a `file` part with a copy fit to send; an edit sending its image first.

use std::io::Cursor;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::{
    Body, Cost, Currency, ModelRef, Money, Nanos, Part, StopReason, TokenCounts, Usage,
};
use glave_harness::AllowAll;
use glave_ports::{
    BlobStore as _, Completion, InMemoryLog, Made, MadeFile, Message, Pieces, Purpose, digest,
};
use serde_json::json;

use crate::support::*;

/// A PNG of `width` by `height`, in shades.
fn png(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbImage::from_fn(width, height, |x, y| {
        image::Rgb([
            u8::try_from(x % 256).unwrap(),
            u8::try_from(y % 256).unwrap(),
            40,
        ])
    });
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// What a provider made: `image`, at four cents.
fn made(image: &[u8]) -> Made {
    let counts = TokenCounts {
        input: 12,
        output: 4_175,
        ..TokenCounts::default()
    };
    Made {
        files: vec![MadeFile {
            media_type: Some("image/png".into()),
            data: STANDARD.encode(image),
        }],
        completion: Completion {
            model: ModelRef {
                provider: "script".into(),
                name: "image-model".into(),
            },
            response_id: None,
            parts: Vec::new(),
            tool_calls: Vec::new(),
            stop: StopReason::EndTurn,
            provider_stop: None,
            usage: Usage::new(counts, false, None).unwrap(),
            cost: Some(Cost {
                amount: Money {
                    nanos: Nanos::new(40_000_000),
                    currency: Currency::USD,
                },
                price_list: "script:reported".into(),
            }),
            served: None,
            upstream: None,
        },
    }
}

/// An answer that calls `tool` with `arguments`, then one that ends the turn.
fn calls(tool: &str, arguments: serde_json::Value) -> Vec<Completion> {
    vec![
        answer(None, vec![(tool, arguments)], StopReason::ToolUse),
        answer(Some("Here it is."), Vec::new(), StopReason::EndTurn),
    ]
}

#[test]
fn a_made_image_is_kept_whole_shown_to_the_model_and_counted_in_the_turn() {
    let dir = folder("media-made");
    block_on!(async {
        let image = png(2_000, 1_000);
        let arguments = json!({"prompt": "a fox at dawn", "aspect_ratio": "3:2"});
        let script =
            ScriptedModel::new(calls("image_generate", arguments)).making(Ok(made(&image)));
        let asked = script.seen_made();
        let (harness, seen) = scripted(
            workspace(&dir).await,
            script,
            AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        );
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let result = turn(&harness, &mut thread, "draw me a fox").await;

        let (request, ask) = asked.lock().unwrap().first().cloned().unwrap();
        assert_eq!(request.purpose, Purpose::ImageGenerate);
        assert_eq!(ask.aspect_ratio.as_deref(), Some("3:2"));
        assert_eq!(texts(&request.messages[0]), ["a fox at dawn"]);
        assert!(request.payer.is_some(), "the thread's work, charged as it");
        assert_eq!(request.max_output_tokens, Some(16_384));

        let requests = seen.lock().unwrap().clone();
        let offered: Vec<&str> = requests[0].tools.iter().map(|t| t.name.as_str()).collect();
        assert!(offered.contains(&"image_generate") && offered.contains(&"image_edit"));
        let Some(Message::Tool {
            output, is_error, ..
        }) = requests[1].messages.last()
        else {
            panic!("the result is sent back");
        };
        assert!(!is_error, "{output:?}");
        let sha = digest(&image);
        let said = json!({"files": [{"file": format!("blob:{}", sha.as_str()), "media_type": "image/png"}]});
        assert_eq!(
            output[0],
            Part::Text {
                text: said.to_string()
            }
        );
        let Part::File { blob, resized, .. } = &output[1] else {
            panic!("the image follows as a file: {output:?}");
        };
        assert_eq!(blob.sha256, sha);
        assert!(
            resized.is_some(),
            "a copy fit to send, the original kept whole"
        );
        assert!(
            requests[1]
                .files
                .iter()
                .any(|file| Some(&file.sha256) == resized.as_ref().map(|r| &r.sha256)),
            "the model sees what it made"
        );
        let mut kept = harness.blobs().get(&scope(), &sha).await.unwrap().unwrap();
        let bytes = glave_ports::read_all(kept.bytes.as_mut(), u64::MAX)
            .await
            .unwrap();
        assert_eq!(bytes, image, "kept whole, as the provider made it");

        assert_eq!(
            result.model_calls, 3,
            "two turns of the model's and the image"
        );
        let log = events(harness.log(), thread.id).await;
        let responded = log
            .iter()
            .filter(|event| matches!(&event.body, Body::ModelResponded(r) if r.model.name == "image-model"))
            .count();
        assert_eq!(responded, 1, "recorded as a model call");
    });
}

#[test]
fn an_edit_sends_its_image_first_and_what_goes_wrong_is_told_to_the_model() {
    let dir = folder("media-edit");
    block_on!(async {
        let original = png(64, 32);
        let style = png(16, 16);
        std::fs::write(dir.path().join("style.png"), &style).unwrap();
        let held = format!("blob:{}", digest(&original).as_str());
        let edit = json!({"image": held, "prompt": "make it night", "references": ["style.png"]});
        let answers = vec![
            answer(None, vec![("image_edit", edit)], StopReason::ToolUse),
            answer(
                None,
                vec![("image_edit", json!({"prompt": "again"}))],
                StopReason::ToolUse,
            ),
            answer(Some("Sorry."), Vec::new(), StopReason::EndTurn),
        ];
        let refused = glave_ports::ProviderError::Unserved {
            failure: glave_ports::Failure::Rejected,
            reason: "this server has no model for image_edit".into(),
        };
        let script = ScriptedModel::new(answers).making(Err(refused));
        let asked = script.seen_made();
        let (harness, seen) = scripted(
            workspace(&dir).await,
            script,
            AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        );
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let here = scope();
        let put = harness.blobs().put(
            &here,
            Some(thread.id),
            "image/png",
            1 << 20,
            Box::new(Pieces::from(original.clone())),
        );
        put.await.unwrap();
        turn(&harness, &mut thread, "edit it").await;

        let made = asked.lock().unwrap().clone();
        assert_eq!(
            made.len(),
            1,
            "a call whose arguments don't fit is never sent"
        );
        let (request, _) = made.into_iter().next().unwrap();
        assert_eq!(request.purpose, Purpose::ImageEdit);
        let inline: Vec<_> = request
            .files
            .iter()
            .map(|file| file.sha256.clone())
            .collect();
        assert_eq!(
            inline,
            [digest(&original), digest(&style)],
            "the image first"
        );
        let told = results(&seen.lock().unwrap()[2].messages);
        assert!(told[0].contains("No image was made"), "{told:?}");
        assert!(told[1].contains("Name the image"), "{told:?}");
    });
}

#[test]
fn a_provider_that_makes_nothing_offers_no_media_tool() {
    let dir = folder("media-none");
    block_on!(async {
        let answers = vec![answer(Some("Hi."), Vec::new(), StopReason::EndTurn)];
        let (harness, seen) =
            harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "hello").await;
        let requests = seen.lock().unwrap().clone();
        assert!(
            !requests[0]
                .tools
                .iter()
                .any(|tool| tool.name.starts_with("image_")),
            "no media tool where nothing serves one"
        );
    });
}

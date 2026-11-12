//! The image API's request, checked as JSON.

use glave_event::{BlobRef, MaxPrice, Nanos, Sort};
use glave_ports::{Purpose, ToolChoice};
use serde_json::json;

use super::*;

/// A request to edit an image, with one reference after it.
fn editing(route: Option<Route>) -> ModelRequest {
    let image = BlobRef {
        sha256: glave_ports::digest(b"png"),
        bytes: 3,
        media_type: "image/png".into(),
    };
    let style = BlobRef {
        sha256: glave_ports::digest(b"jpg"),
        bytes: 3,
        media_type: "image/jpeg".into(),
    };
    let part = |blob: &BlobRef| Part::File {
        blob: blob.clone(),
        name: None,
        resized: None,
    };
    let inline = |blob: &BlobRef, data: &str| InlineFile {
        sha256: blob.sha256.clone(),
        media_type: blob.media_type.clone(),
        data: data.into(),
    };
    ModelRequest {
        model: "google/nano-banana".into(),
        system: String::new(),
        messages: vec![Message::User {
            parts: vec![
                Part::Text {
                    text: "Make the sky purple, in the second image's style.".into(),
                },
                part(&image),
                part(&style),
            ],
        }],
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        max_output_tokens: Some(16_384),
        purpose: Purpose::ImageEdit,
        effort: None,
        files: vec![inline(&style, "anBn"), inline(&image, "cG5n")],
        payer: None,
        route,
    }
}

#[test]
fn an_image_is_asked_for_with_its_prompt_shape_and_references_in_order() {
    let ask = MediaAsk {
        aspect_ratio: Some("16:9".into()),
        transparent: true,
        settings: json!({"quality": "high"}).as_object().cloned().unwrap(),
        ..MediaAsk::default()
    };
    let route = Route {
        only: Some(vec!["google-vertex".into()]),
        ignore: vec!["somewhere".into()],
        sort: Some(Sort::Speed),
        ..Route::default()
    };
    let request = editing(Some(route));
    let sent = serde_json::to_value(images_request(&request, &ask).unwrap()).unwrap();
    assert_eq!(
        sent,
        json!({
            "model": "google/nano-banana",
            "prompt": "Make the sky purple, in the second image's style.",
            "n": 1,
            "aspect_ratio": "16:9",
            "input_references": [
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,cG5n"}},
                {"type": "image_url", "image_url": {"url": "data:image/jpeg;base64,anBn"}},
            ],
            "background": "transparent",
            "output_format": "png",
            "provider": {"only": ["google-vertex"], "ignore": ["somewhere"], "sort": "throughput"},
            "quality": "high",
        })
    );
    let plain = editing(None);
    let sent = serde_json::to_value(images_request(&plain, &MediaAsk::default()).unwrap());
    let sent = sent.unwrap();
    assert!(sent.get("aspect_ratio").is_none() && sent.get("provider").is_none());
    assert!(sent.get("background").is_none() && sent.get("output_format").is_none());
}

#[test]
fn filters_the_image_api_cannot_be_told_refuse_the_call() {
    let refused = [
        Route {
            zdr: true,
            ..Route::default()
        },
        Route {
            quantizations: vec!["fp8".into()],
            ..Route::default()
        },
        Route {
            max_price: Some(MaxPrice {
                input: Some(Nanos::new(1)),
                output: None,
            }),
            ..Route::default()
        },
    ];
    for route in refused {
        let request = editing(Some(route.clone()));
        let error = images_request(&request, &MediaAsk::default()).unwrap_err();
        assert!(
            matches!(&error, ProviderError::Unserved { reason, .. } if reason.contains("loosen")),
            "{route:?}: {error:?}"
        );
    }
}

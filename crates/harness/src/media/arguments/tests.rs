//! The media tools' arguments read, and a call in a line.

use serde_json::json;

use super::*;

fn arguments(value: Value) -> Arguments {
    let Value::Object(map) = value else {
        return Arguments::Malformed(value.to_string());
    };
    Arguments::Json(map)
}

#[test]
fn an_edit_asks_its_image_first_then_its_references_with_its_shape_tier_and_background() {
    let given = arguments(json!({
        "image": "blob:abc", "prompt": "make it night", "aspect_ratio": "16:9",
        "references": ["art/style.png"], "quality": "high", "transparent": true,
    }));
    let edit = asked(Purpose::ImageEdit, &given).unwrap();
    assert_eq!(edit.prompt, "make it night");
    assert_eq!(edit.images, ["blob:abc", "art/style.png"]);
    assert_eq!(edit.ask.aspect_ratio.as_deref(), Some("16:9"));
    assert_eq!(edit.ask.quality, Quality::High);
    assert!(edit.ask.transparent && edit.ask.settings.is_empty());
    let own = arguments(json!({"image": "a.png", "prompt": "brighter"}));
    let own = asked(Purpose::ImageEdit, &own).unwrap();
    assert_eq!(
        own.ask.aspect_ratio, None,
        "an edit keeps the image's own shape"
    );
    let made = asked(
        Purpose::ImageGenerate,
        &arguments(json!({"prompt": "a fox"})),
    )
    .unwrap();
    assert!(made.images.is_empty() && !made.ask.transparent);
    assert_eq!(
        made.ask.aspect_ratio.as_deref(),
        Some("1:1"),
        "a new image is square"
    );
    assert_eq!(made.ask.quality, Quality::Medium);
}

#[test]
fn what_does_not_fit_a_tool_is_told_to_the_model() {
    let wrong = [
        (Purpose::ImageGenerate, json!({"prompt": "  "}), "prompt"),
        (
            Purpose::ImageGenerate,
            json!({"prompt": "x", "aspect_ratio": "5:7"}),
            "5:7",
        ),
        (
            Purpose::ImageGenerate,
            json!({"prompt": "x", "image": "a.png"}),
            "image_edit",
        ),
        (
            Purpose::ImageGenerate,
            json!({"prompt": "x", "size": "4K"}),
            "size",
        ),
        (Purpose::ImageEdit, json!({"prompt": "x"}), "image"),
        (
            Purpose::ImageGenerate,
            json!({"prompt": "x", "quality": "best"}),
            "low, medium or high",
        ),
        (
            Purpose::ImageGenerate,
            json!({"prompt": "x".repeat(8_001)}),
            "8000",
        ),
        (
            Purpose::ImageGenerate,
            json!({"prompt": "x", "references": vec!["a"; 9]}),
            "At most 8",
        ),
    ];
    for (tool, given, says) in wrong {
        let told = asked(tool, &arguments(given.clone())).unwrap_err();
        assert!(told.contains(says), "{given}: {told}");
    }
    let malformed = Arguments::Malformed("{".into());
    assert!(asked(Purpose::ImageEdit, &malformed).is_err());
}

#[test]
fn each_tool_is_named_for_its_purpose_and_a_call_reads_as_its_prompt() {
    for each in [Purpose::ImageGenerate, Purpose::ImageEdit] {
        let spec = spec(each);
        assert_eq!(spec.name, each.id());
        assert_eq!(purpose(&spec.name), Some(each));
    }
    assert_eq!(purpose(" Image_Edit "), Some(Purpose::ImageEdit));
    assert_eq!(purpose("image"), None);
    let long = "a  red\nfox ".repeat(30);
    let line = describe(IMAGE_GENERATE, &arguments(json!({ "prompt": long })));
    assert!(
        line.starts_with("image_generate: a red fox a red fox"),
        "{line}"
    );
    assert!(line.ends_with('…') && !line.contains('\n'), "{line}");
}

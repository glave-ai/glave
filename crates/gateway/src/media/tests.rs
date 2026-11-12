use std::sync::Arc;

use glave_ports::testing::ScriptedModel;
use serde_json::json;

use super::*;

/// A target for `model` in the tier `quality`, making transparent images or not.
fn tiered(model: &str, quality: Quality, transparent: bool) -> Target {
    let handle = Arc::new(ScriptedModel::new(Vec::new()));
    let settings = json!({"quality": model}).as_object().cloned().unwrap();
    Target::new(handle, "openrouter", model).in_tier(Tier {
        quality,
        settings,
        transparent,
        output_tokens: 1_000,
        edit_aspect_ratio: transparent.then(|| "auto".to_owned()),
    })
}

fn route() -> Vec<Target> {
    vec![
        tiered("banana", Quality::Low, false),
        tiered("flare", Quality::Medium, true),
        tiered("sunburst", Quality::High, true),
    ]
}

fn models(targets: &[Target]) -> Vec<&str> {
    targets.iter().map(|target| target.model.as_str()).collect()
}

#[test]
fn a_call_goes_to_its_tier_and_to_another_only_for_what_its_tier_cannot_do() {
    let asked = |quality, transparent| MediaAsk {
        quality,
        transparent,
        ..MediaAsk::default()
    };
    let low = chosen(
        &route(),
        Purpose::ImageGenerate,
        &asked(Quality::Low, false),
    )
    .unwrap();
    assert_eq!(models(&low), ["banana"]);
    let high = chosen(
        &route(),
        Purpose::ImageGenerate,
        &asked(Quality::High, false),
    )
    .unwrap();
    assert_eq!(models(&high), ["sunburst"]);
    let clear = chosen(&route(), Purpose::ImageGenerate, &asked(Quality::Low, true)).unwrap();
    assert_eq!(
        models(&clear),
        ["flare", "sunburst"],
        "the first that can, in order"
    );
    let none = vec![tiered("banana", Quality::Low, false)];
    let refused = chosen(&none, Purpose::ImageEdit, &asked(Quality::Low, true)).unwrap_err();
    assert!(
        matches!(&refused, ProviderError::Unserved { reason, .. } if reason.contains("transparent")),
        "{refused:?}"
    );
    let empty = chosen(&[], Purpose::ImageEdit, &asked(Quality::Medium, false)).unwrap_err();
    assert!(
        matches!(&empty, ProviderError::Unserved { reason, .. } if reason.contains("image_edit")),
        "{empty:?}"
    );
}

#[test]
fn a_target_is_sent_its_settings_and_its_shape_for_an_edit_that_asks_none() {
    let [banana, flare, _] = <[Target; 3]>::try_from(route()).unwrap();
    let plain = MediaAsk::default();
    let edit = sent_to(&plain, &flare, Purpose::ImageEdit);
    assert_eq!(edit.aspect_ratio.as_deref(), Some("auto"));
    assert_eq!(edit.settings.get("quality"), Some(&json!("flare")));
    let made = sent_to(&plain, &flare, Purpose::ImageGenerate);
    assert_eq!(
        made.aspect_ratio, None,
        "a new image's shape is the tool's to say"
    );
    assert_eq!(
        sent_to(&plain, &banana, Purpose::ImageEdit).aspect_ratio,
        None
    );
    let wide = MediaAsk {
        aspect_ratio: Some("16:9".into()),
        ..MediaAsk::default()
    };
    let wide = sent_to(&wide, &flare, Purpose::ImageEdit);
    assert_eq!(wide.aspect_ratio.as_deref(), Some("16:9"));
}

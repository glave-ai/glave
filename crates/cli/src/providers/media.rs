//! Which models serve glave's media tools (`specs/gateway.md`, "Media tools"): OpenRouter's
//! image models, through the first OpenRouter account the lineup reads (the server's own,
//! else the person's), each quality tier its own model and settings. A tool with no such
//! account is not offered.

use std::sync::Arc;

use glave_config::policy::{Constraints, Source, check_model};
use glave_event::{Catalog, CatalogProvider};
use glave_gateway::{Target, Tier};
use glave_ports::{Purpose, Quality};
use serde_json::{Map, Value};

/// One tier as the server's settings give it by default: its model, what it is sent beside
/// each call, whether it makes a transparent background, the most tokens of images it gives
/// out, and the shape it is asked for an edit that asks none.
struct Default {
    quality: Quality,
    model: &'static str,
    setting: (&'static str, &'static str),
    transparent: bool,
    output_tokens: u32,
    edit_aspect_ratio: Option<&'static str>,
}

/// The image tiers, as the owner's design of 2026-11-12 maps them: Google's for trying ideas,
/// OpenAI's for the rest.
const IMAGES: [Default; 3] = [
    Default {
        quality: Quality::Low,
        model: "google/gemini-nano-banana-2.1",
        setting: ("resolution", "1K"),
        transparent: false,
        output_tokens: 1_700,
        edit_aspect_ratio: None,
    },
    Default {
        quality: Quality::Medium,
        model: "openai/gpt-image-2.6-flare",
        setting: ("quality", "high"),
        transparent: true,
        output_tokens: 8_192,
        edit_aspect_ratio: Some("auto"),
    },
    Default {
        quality: Quality::High,
        model: "openai/gpt-image-2.6-sunburst",
        setting: ("quality", "max"),
        transparent: true,
        output_tokens: 32_768,
        edit_aspect_ratio: Some("auto"),
    },
];

/// The media tools' routes through the OpenRouter account reached by `handle`: each image
/// tool's the three tiers, as its `catalog` describes their models where it lists them, those
/// the policy allows.
pub(super) fn routes(
    handle: &Arc<dyn glave_gateway::Model>,
    catalog: Option<&Catalog>,
    providers: &Arc<[CatalogProvider]>,
    constraints: &Constraints,
) -> Vec<(Purpose, Vec<Target>)> {
    let tiers: Vec<Target> = IMAGES
        .iter()
        .map(|tier| {
            let target =
                super::lineup::target_at(handle, "openrouter", tier.model, catalog, providers);
            let (key, value) = tier.setting;
            let settings = Map::from_iter([(key.to_owned(), Value::from(value))]);
            target.in_tier(Tier {
                quality: tier.quality,
                settings,
                transparent: tier.transparent,
                output_tokens: tier.output_tokens,
                edit_aspect_ratio: tier.edit_aspect_ratio.map(str::to_owned),
            })
        })
        .filter(|target| check_model(&target.name, &Source::Default, constraints).is_ok())
        .collect();
    if tiers.is_empty() {
        return Vec::new();
    }
    [Purpose::ImageGenerate, Purpose::ImageEdit]
        .into_iter()
        .map(|purpose| (purpose, tiers.clone()))
        .collect()
}

//! Which models serve glave's media tools (`specs/gateway.md`, "Media tools"): OpenRouter's
//! image models, through the first OpenRouter account the lineup reads (the server's own,
//! else the person's), each tool its own model. A tool with no such account is not offered.

use std::sync::Arc;

use glave_config::policy::{Constraints, Source, check_model};
use glave_event::{Catalog, CatalogProvider};
use glave_gateway::Target;
use glave_ports::Purpose;

/// The model each media tool uses at OpenRouter: the image models the owner chose on
/// 2026-11-02, OpenAI's for making an image and Google's for changing one.
const MODELS: [(Purpose, &str); 2] = [
    (Purpose::ImageGenerate, "openai/gpt-image-2.5-sunburst"),
    (Purpose::ImageEdit, "google/gemini-nano-banana-2.1"),
];

/// The media tools' targets through the OpenRouter account reached by `handle`, as its
/// `catalog` describes their models where it lists them, those the policy allows.
pub(super) fn targets(
    handle: &Arc<dyn glave_gateway::Model>,
    catalog: Option<&Catalog>,
    providers: &Arc<[CatalogProvider]>,
    constraints: &Constraints,
) -> Vec<(Purpose, Target)> {
    MODELS
        .into_iter()
        .map(|(purpose, model)| {
            let target = super::lineup::target_at(handle, "openrouter", model, catalog, providers);
            (purpose, target)
        })
        .filter(|(_, target)| check_model(&target.name, &Source::Default, constraints).is_ok())
        .collect()
}

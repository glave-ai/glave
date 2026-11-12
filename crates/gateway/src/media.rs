//! A media tool's targets (`specs/gateway.md`, "Media tools"): each serves one quality tier,
//! with the settings the server gives its model there, and says what it can't do. A call goes
//! to its own tier's targets; when none of them can honour it (a transparent image from a model
//! that makes none), to those of the first other tier that can, never further; when none can,
//! it is refused in one sentence.

use glave_ports::{Failure, MediaAsk, ProviderError, Purpose, Quality};
use serde_json::{Map, Value};

use crate::target::Target;

/// The most one image sent in reads, in tokens, as a call's worst case counts it: GPT Image reads
/// every image in at high fidelity, 6,240 tokens for a 1536x1024 one beside its tiles' own
/// (OpenAI's image guide, 2026-11-12), Nano Banana Pro 560.
pub(crate) const IMAGE_IN_TOKENS: u64 = 8_192;

/// What a target serving a media tool does, as the server's settings say.
#[derive(Debug, Clone, PartialEq)]
pub struct Tier {
    /// The tier it serves.
    pub quality: Quality,
    /// What it is sent beside the call, as its service names it (`quality`, `resolution`).
    pub settings: Map<String, Value>,
    /// Whether its model makes a transparent background.
    pub transparent: bool,
    /// The most tokens of images it gives out: what a call's worst case holds for them.
    pub output_tokens: u32,
    /// The shape it is asked for an edit that asks none, when its model keeps the image's own
    /// only so (GPT Image's `auto`); none leaves it out.
    pub edit_aspect_ratio: Option<String>,
}

/// The targets of `route` a media call asking `ask` may go to, in order: its tier's that can
/// honour it, else every other that can; refused when none can.
pub(crate) fn chosen(
    route: &[Target],
    purpose: Purpose,
    ask: &MediaAsk,
) -> Result<Vec<Target>, ProviderError> {
    let able = |target: &&Target| {
        !ask.transparent || target.tier.as_ref().is_some_and(|tier| tier.transparent)
    };
    let own = |target: &&Target| {
        target
            .tier
            .as_ref()
            .is_some_and(|tier| tier.quality == ask.quality)
    };
    let mine: Vec<Target> = route.iter().filter(own).filter(able).cloned().collect();
    if !mine.is_empty() {
        return Ok(mine);
    }
    // One tier on at most, so a failure that may pass never climbs to a dearer one still.
    let next = route
        .iter()
        .find(able)
        .and_then(|target| target.tier.as_ref());
    let next = next.map(|tier| tier.quality);
    let others: Vec<Target> = route
        .iter()
        .filter(able)
        .filter(|target| target.tier.as_ref().map(|tier| tier.quality) == next)
        .cloned()
        .collect();
    if !others.is_empty() {
        return Ok(others);
    }
    let reason = if route.is_empty() {
        format!("this server has no model for {}", purpose.id())
    } else {
        "no model here makes an image with a transparent background: ask for one without".to_owned()
    };
    Err(ProviderError::Unserved {
        failure: Failure::Rejected,
        reason,
    })
}

/// `ask` as `target` is sent it: with its tier's settings, and its shape for an edit that asks
/// none.
pub(crate) fn sent_to(ask: &MediaAsk, target: &Target, purpose: Purpose) -> MediaAsk {
    let Some(tier) = target.tier.as_deref() else {
        return ask.clone();
    };
    let aspect_ratio = match (&ask.aspect_ratio, purpose) {
        (None, Purpose::ImageEdit) => tier.edit_aspect_ratio.clone(),
        (asked, _) => asked.clone(),
    };
    MediaAsk {
        aspect_ratio,
        settings: tier.settings.clone(),
        ..ask.clone()
    }
}

#[cfg(test)]
mod tests;

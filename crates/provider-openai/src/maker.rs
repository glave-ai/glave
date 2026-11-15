//! What a model's maker documents for calls to it and no service's catalog says, by family,
//! whichever service serves it (`specs/gateway.md`, "Decoding"): the sampler it is meant to
//! run with, sent on every call rather than left to each host's defaults, and whether every
//! earlier reply carries its reasoning. Never a penalty: only what the maker documents.

/// How a family's maker says its tokens are drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sampler {
    pub temperature: f64,
    pub top_p: f64,
}

/// One family: the start its models' ids share, as [`own_id`] writes them, and its rules.
struct Family {
    prefix: &'static str,
    sampler: Option<Sampler>,
    /// Every earlier assistant message carries its reasoning, empty when it had none.
    every_turn: bool,
}

/// DeepSeek V3.2's and V4.1's model cards: temperature 1.0, top_p 0.95, the values their
/// agentic evaluations ran at.
const DEEPSEEK: Sampler = Sampler {
    temperature: 1.0,
    top_p: 0.95,
};

/// The families, the more particular first. DeepSeek's thinking mode refuses a request with
/// tools unless every earlier reply carries `reasoning_content` ("Thinking Mode", its API
/// docs), and its open weights are trained to expect it wherever they run.
const FAMILIES: &[Family] = &[
    Family {
        prefix: "deepseek-v4",
        sampler: Some(DEEPSEEK),
        every_turn: true,
    },
    Family {
        prefix: "deepseek-v3-2",
        sampler: Some(DEEPSEEK),
        every_turn: true,
    },
    Family {
        prefix: "deepseek",
        sampler: None,
        every_turn: true,
    },
];

/// A model's id without the service's prefix, in lower case, dots as hyphens: OpenRouter's
/// `deepseek/deepseek-v4.1-flash` and DeepInfra's `deepseek-ai/DeepSeek-V4.1-Flash` alike.
fn own_id(model: &str) -> String {
    let own = model.rsplit('/').next().unwrap_or(model);
    own.to_ascii_lowercase().replace('.', "-")
}

fn family(model: &str) -> Option<&'static Family> {
    let own = own_id(model);
    FAMILIES
        .iter()
        .find(|family| own.starts_with(family.prefix))
}

/// The sampler `model`'s maker documents, if any.
pub(crate) fn sampler(model: &str) -> Option<Sampler> {
    family(model).and_then(|family| family.sampler)
}

/// Whether `model`'s maker wants every earlier assistant message to carry its reasoning.
pub(crate) fn every_turn(model: &str) -> bool {
    family(model).is_some_and(|family| family.every_turn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_family_is_known_by_its_id_at_any_service() {
        for id in [
            "deepseek/deepseek-v4.1-flash",
            "deepseek-ai/DeepSeek-V4.1-Flash",
            "deepseek-v4-pro",
            "deepseek-ai/DeepSeek-V3.2",
        ] {
            assert_eq!(sampler(id), Some(DEEPSEEK), "{id}");
            assert!(every_turn(id), "{id}");
        }
        assert_eq!(sampler("deepseek-reasoner"), None, "no card says");
        assert!(every_turn("deepseek-reasoner"));
        for id in [
            "anthropic/claude-haiku-5.5",
            "openai/gpt-6-luna",
            "gemma-4-12b",
        ] {
            assert_eq!(sampler(id), None, "{id}");
            assert!(!every_turn(id), "{id}");
        }
    }
}

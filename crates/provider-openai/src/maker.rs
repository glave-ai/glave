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

/// One family: the ids its models go by, as [`own_id`] writes them, and its rules.
struct Family {
    /// The start its ids share, or with [`Family::whole`] the whole of its one id.
    name: &'static str,
    whole: bool,
    sampler: Option<Sampler>,
    /// Every earlier assistant message carries its reasoning, empty when it had none.
    every_turn: bool,
}

/// DeepSeek V3.2's and V4.1 Flash's model cards: temperature 1.0, top_p 0.95, the values
/// their agentic evaluations ran at.
const AGENTIC: Sampler = Sampler {
    temperature: 1.0,
    top_p: 0.95,
};

/// DeepSeek V4 Flash's and V4 Pro's model cards: temperature 1.0, top_p 1.0.
const V4: Sampler = Sampler {
    temperature: 1.0,
    top_p: 1.0,
};

/// A DeepSeek family whose thinking mode takes tools, by the start of its ids.
const fn deepseek(name: &'static str, sampler: Option<Sampler>) -> Family {
    Family {
        name,
        whole: false,
        sampler,
        every_turn: true,
    }
}

/// The families, the more particular first. DeepSeek's thinking mode, from V3.2 on, refuses a
/// request with tools unless every earlier reply carries `reasoning_content` ("Thinking Mode",
/// its API docs). Earlier DeepSeek models (V3.1, R1 and the Llama and Qwen models distilled
/// from it) take no tools while thinking, so no rule of theirs is known.
const FAMILIES: &[Family] = &[
    deepseek("deepseek-v4-1", Some(AGENTIC)),
    deepseek("deepseek-v4p1", Some(AGENTIC)),
    // DeepSeek's API name for V4.1 Flash, and OpenRouter's `~deepseek/deepseek-flash-latest`.
    deepseek("deepseek-flash", Some(AGENTIC)),
    deepseek("deepseek-v4", Some(V4)),
    // DeepSeek's API name for V4 Pro, and OpenRouter's `~deepseek/deepseek-pro-latest`.
    deepseek("deepseek-pro", Some(V4)),
    // V3.2's experimental release: its card gives no sampler.
    deepseek("deepseek-v3-2-exp", None),
    deepseek("deepseek-v3-2", Some(AGENTIC)),
    // Fireworks' name for V3.2.
    deepseek("deepseek-v3p2", Some(AGENTIC)),
    // DeepSeek's API names for its current model, which its own API samples.
    Family {
        whole: true,
        ..deepseek("deepseek-chat", None)
    },
    Family {
        whole: true,
        ..deepseek("deepseek-reasoner", None)
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
    FAMILIES.iter().find(|family| {
        if family.whole {
            own == family.name
        } else {
            own.starts_with(family.name)
        }
    })
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
        for (id, documented) in [
            ("deepseek/deepseek-v4.1-flash", AGENTIC),
            ("deepseek-ai/DeepSeek-V4.1-Flash", AGENTIC),
            ("accounts/fireworks/models/deepseek-v4p1-flash", AGENTIC),
            ("deepseek-flash", AGENTIC),
            ("~deepseek/deepseek-flash-latest", AGENTIC),
            ("deepseek/deepseek-v4-flash-0731", V4),
            ("deepseek-v4-pro", V4),
            ("~deepseek/deepseek-pro-latest", V4),
            ("deepseek-ai/DeepSeek-V3.2", AGENTIC),
            ("accounts/fireworks/models/deepseek-v3p2", AGENTIC),
        ] {
            assert_eq!(sampler(id), Some(documented), "{id}");
            assert!(every_turn(id), "{id}");
        }
        for id in [
            "deepseek-reasoner",
            "deepseek-chat",
            "deepseek/deepseek-v3.2-exp",
        ] {
            assert_eq!(sampler(id), None, "no card says, for {id}");
            assert!(every_turn(id), "{id}");
        }
        for id in [
            "anthropic/claude-haiku-5.5",
            "openai/gpt-6-luna",
            "gemma-4-12b",
            "deepseek-r1:8b",
            "deepseek/deepseek-r1-0528",
            "deepseek/deepseek-chat-v3.1",
            "deepseek-ai/DeepSeek-R1-Distill-Qwen-32B",
        ] {
            assert_eq!(sampler(id), None, "{id}");
            assert!(!every_turn(id), "{id}");
        }
    }
}

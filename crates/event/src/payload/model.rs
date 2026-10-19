//! `model.responded`: one finished model response, with its stop reason, usage and cost.
//! Content-free, so it can outlive the content it describes.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::error::InvalidValue;
use crate::money::Money;

pub(crate) const TYPE: &str = "model.responded";

/// Which model answered.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRef {
    pub provider: String,
    pub name: String,
}

/// A model response finished.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelResponded {
    pub model: ModelRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    pub stop: StopReason,
    /// The provider's exact stop value, kept beside the normalized one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_stop: Option<String>,
    pub usage: Usage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cost>,
    /// The gateway route the call was sent by: its purpose, such as `main`, `judge` or
    /// `summary`. Absent when no gateway was in between.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// The gateway target that answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// True when the gateway left opaque reasoning out of the request because the target
    /// that answered could not accept it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reasoning_stripped: bool,
}

/// Why a model stopped, normalized across providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
    ContextExceeded,
    ContentFilter,
    Refusal,
    Pause,
    MalformedToolCall,
    Error,
    Cancelled,
    Interrupted,
}

/// Token counts as inclusive totals with subsets: `input` includes `cache_read` and
/// `cache_write`, and `output` includes `reasoning`. Providers disagree on what their totals
/// include, so adapters normalize before writing.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct TokenCounts {
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
    pub reasoning: u64,
}

/// Validated usage for one response. `estimated` is true when a stream ended before the
/// provider reported usage; `raw` keeps the provider's own usage object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "UsageRepr", into = "UsageRepr")]
pub struct Usage {
    counts: TokenCounts,
    estimated: bool,
    raw: Option<Value>,
}

impl Usage {
    pub fn new(
        counts: TokenCounts,
        estimated: bool,
        raw: Option<Value>,
    ) -> Result<Self, InvalidValue> {
        let cached = counts.cache_read.checked_add(counts.cache_write);
        if cached.is_none_or(|cached| cached > counts.input) {
            return Err(InvalidValue::new(
                "usage",
                format!(
                    "cache_read ({}) plus cache_write ({}) exceeds input ({})",
                    counts.cache_read, counts.cache_write, counts.input
                ),
            ));
        }
        if counts.reasoning > counts.output {
            return Err(InvalidValue::new(
                "usage",
                format!(
                    "reasoning ({}) exceeds output ({})",
                    counts.reasoning, counts.output
                ),
            ));
        }
        Ok(Self {
            counts,
            estimated,
            raw,
        })
    }

    pub fn counts(&self) -> TokenCounts {
        self.counts
    }

    pub fn estimated(&self) -> bool {
        self.estimated
    }

    pub fn raw(&self) -> Option<&Value> {
        self.raw.as_ref()
    }
}

/// The stored shape of [`Usage`], checked on the way in.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UsageRepr {
    input: u64,
    cache_read: u64,
    cache_write: u64,
    output: u64,
    reasoning: u64,
    estimated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    raw: Option<Value>,
}

impl TryFrom<UsageRepr> for Usage {
    type Error = InvalidValue;

    fn try_from(repr: UsageRepr) -> Result<Self, Self::Error> {
        let counts = TokenCounts {
            input: repr.input,
            cache_read: repr.cache_read,
            cache_write: repr.cache_write,
            output: repr.output,
            reasoning: repr.reasoning,
        };
        Self::new(counts, repr.estimated, repr.raw)
    }
}

impl From<Usage> for UsageRepr {
    fn from(usage: Usage) -> Self {
        Self {
            input: usage.counts.input,
            cache_read: usage.counts.cache_read,
            cache_write: usage.counts.cache_write,
            output: usage.counts.output,
            reasoning: usage.counts.reasoning,
            estimated: usage.estimated,
            raw: usage.raw,
        }
    }
}

/// What a response cost, and the price list it was computed from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cost {
    pub amount: Money,
    pub price_list: String,
}

impl Payload for ModelResponded {
    const TYPE: &'static str = TYPE;
    const VERSION: u32 = 1;
}

pub(crate) fn decode(version: u32, data: &RawValue) -> Option<serde_json::Result<ModelResponded>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(
        input: u64,
        cache_read: u64,
        cache_write: u64,
        output: u64,
        reasoning: u64,
    ) -> TokenCounts {
        TokenCounts {
            input,
            cache_read,
            cache_write,
            output,
            reasoning,
        }
    }

    #[test]
    fn subsets_must_fit_inside_totals() {
        assert!(Usage::new(counts(43_003, 40_000, 3_000, 120, 0), false, None).is_ok());
        assert!(Usage::new(counts(100, 80, 30, 10, 0), false, None).is_err());
        assert!(Usage::new(counts(100, 0, 0, 10, 11), false, None).is_err());
        assert!(Usage::new(counts(u64::MAX, u64::MAX, 1, 0, 0), false, None).is_err());
    }

    #[test]
    fn invalid_usage_is_rejected_when_read() {
        let bad = r#"{"input":10,"cache_read":20,"cache_write":0,"output":0,"reasoning":0,"estimated":false}"#;
        assert!(serde_json::from_str::<Usage>(bad).is_err());
    }
}

//! `turn.started` and `turn.ended`: the bounds of one turn.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use super::approval::Lifted;
use super::model::{ModelRef, ModelResponded, TokenCounts};
use crate::blob::BlobRef;
use crate::codec::{Payload, parse};
use crate::error::InvalidValue;
use crate::money::{Currency, Money};
use crate::payload::principal::open_kind;

pub(crate) const STARTED: &str = "turn.started";
pub(crate) const ENDED: &str = "turn.ended";

/// A turn begins. `config` points to the turn's full configuration (system prompt, tools,
/// parameters), stored once by hash so every turn can be reproduced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnStarted {
    pub model: ModelRef,
    pub config: BlobRef,
    /// The W3C trace the turn belongs to, linking it to OpenTelemetry spans.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceId>,
    /// How the caller who started the turn signed in. Never a token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<Auth>,
}

/// A W3C trace id: 32 lowercase hex digits, not all zero.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TraceId(String);

impl TraceId {
    pub fn new(id: impl Into<String>) -> Result<Self, InvalidValue> {
        let id = id.into();
        let hex = id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if !hex || id.bytes().all(|b| b == b'0') {
            return Err(InvalidValue::new(
                "trace id",
                format!("must be 32 lowercase hex digits, not all zero, got {id:?}"),
            ));
        }
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for TraceId {
    type Error = InvalidValue;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        Self::new(id)
    }
}

impl From<TraceId> for String {
    fn from(id: TraceId) -> Self {
        id.0
    }
}

/// How a caller signed in. Names the login, never a secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Auth {
    pub method: AuthMethod,
    /// The identity provider, for `oidc` and `saml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    /// The caller's stable id at the identity provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// The sign-in session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

open_kind! {
    /// How a caller signed in.
    pub enum AuthMethod {
        /// The local person: the operating system user is the proof.
        Local => "local",
        /// OpenID Connect.
        Oidc => "oidc",
        /// A glave account: a passkey of the person's, or a browser session one began.
        Account => "account",
        /// SAML.
        Saml => "saml",
    }
}

impl Payload for TurnStarted {
    const TYPE: &'static str = STARTED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_started(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<TurnStarted>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}

/// A turn ended (`turn.ended`): how, and what it did and cost, its subagents' work included.
/// It is what a client that ran the turn is answered with too, so every client reads the same.
/// A turn recorded before it held more than `outcome` reads with nothing counted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TurnResult {
    pub outcome: Outcome,
    #[serde(default)]
    pub model_calls: u32,
    #[serde(default)]
    pub tool_calls: u32,
    /// The commands that ran outside the sandbox, each one allowed.
    #[serde(default)]
    pub outside_sandbox: u32,
    /// The commands that ran with the network the sandbox otherwise kept from them, each one
    /// allowed.
    #[serde(default)]
    pub with_network: u32,
    #[serde(default)]
    pub tokens: TokenCounts,
    /// What its model calls cost, when every one of them reported a cost in one currency;
    /// absent when one did not: the cost is not reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Money>,
    /// True when a usage count was estimated, because a stream ended before the provider
    /// reported it.
    #[serde(default)]
    pub estimated: bool,
    /// Where its model calls ran, once each in the order they first did, in a person's words:
    /// the service, and the provider a service that routes calls passed them on to
    /// (`OpenRouter (Anthropic)`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub via: Vec<String>,
    /// The files its `write` and `edit` calls changed, each once in the order first changed,
    /// as the call named it: what `thread/changes` gives as the turn's, so a client keeps their
    /// room before it has read them. Absent when it changed none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed: Vec<String>,
}

impl TurnResult {
    /// A turn as it begins: nothing done and nothing spent, completed unless it ends otherwise.
    pub fn begun() -> Self {
        Self {
            outcome: Outcome::Completed,
            model_calls: 0,
            tool_calls: 0,
            outside_sandbox: 0,
            with_network: 0,
            tokens: TokenCounts::default(),
            cost: Some(Money {
                micros: 0,
                currency: Currency::USD,
            }),
            estimated: false,
            via: Vec::new(),
            changed: Vec::new(),
        }
    }

    /// Counts the file at `path` as one the turn changed, once.
    pub fn changed_file(&mut self, path: &str) {
        if !self.changed.iter().any(|known| known == path) {
            self.changed.push(path.to_owned());
        }
    }

    /// Counts one command allowed to run without what `lifted` says of the sandbox.
    pub fn let_out(&mut self, lifted: Lifted) {
        let count = match lifted {
            Lifted::Sandbox => &mut self.outside_sandbox,
            Lifted::Network => &mut self.with_network,
        };
        *count = count.saturating_add(1);
    }

    /// Takes in what one model call `responded` used and cost, and where it ran. The call is
    /// counted apart: one that failed is a call too, with nothing to take in.
    pub fn take_in(&mut self, responded: &ModelResponded) {
        self.tokens += responded.usage.counts();
        self.estimated |= responded.usage.estimated();
        self.cost = self
            .cost
            .zip(responded.cost.as_ref())
            .and_then(|(sum, cost)| sum.checked_add(cost.amount));
        let service = responded.service.as_deref().filter(|name| !name.is_empty());
        if let Some(service) = service {
            let place = match responded.upstream.as_deref() {
                Some(upstream) if !upstream.is_empty() && upstream != service => {
                    format!("{service} ({upstream})")
                }
                _ => service.to_owned(),
            };
            self.ran_at(place);
        }
    }

    /// Adds a subagent's work to its parent's turn: its calls, what they used and cost, where
    /// they ran, and the files it changed.
    pub fn absorb(&mut self, child: &Self) {
        self.model_calls = self.model_calls.saturating_add(child.model_calls);
        self.tool_calls = self.tool_calls.saturating_add(child.tool_calls);
        self.outside_sandbox = self.outside_sandbox.saturating_add(child.outside_sandbox);
        self.with_network = self.with_network.saturating_add(child.with_network);
        self.tokens += child.tokens;
        self.estimated |= child.estimated;
        self.cost = self
            .cost
            .zip(child.cost)
            .and_then(|(sum, cost)| sum.checked_add(cost));
        for place in &child.via {
            self.ran_at(place.clone());
        }
        for path in &child.changed {
            self.changed_file(path);
        }
    }

    fn ran_at(&mut self, place: String) {
        if !self.via.contains(&place) {
            self.via.push(place);
        }
    }
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    Cancelled,
    Failed,
    Interrupted,
    LimitReached,
}

impl Payload for TurnResult {
    const TYPE: &'static str = ENDED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_ended(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<TurnResult>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::model::{Cost, StopReason, Usage};

    /// A call that used `input` tokens at `service`, and cost `micros` when it says; its usage
    /// estimated when no router passed it on.
    fn responded(
        input: u64,
        micros: Option<i64>,
        service: &str,
        upstream: Option<&str>,
    ) -> ModelResponded {
        let counts = TokenCounts {
            input,
            ..TokenCounts::default()
        };
        ModelResponded {
            model: ModelRef {
                provider: "p".into(),
                name: "m".into(),
            },
            response_id: None,
            stop: StopReason::EndTurn,
            provider_stop: None,
            usage: Usage::new(counts, upstream.is_none(), None).unwrap(),
            cost: micros.map(|micros| Cost {
                amount: Money {
                    micros,
                    currency: Currency::USD,
                },
                price_list: "list".into(),
            }),
            route: None,
            target: None,
            service: Some(service.into()),
            upstream: upstream.map(str::to_owned),
            effort: None,
            reasoning_stripped: false,
        }
    }

    #[test]
    fn a_turn_takes_in_each_call_and_names_where_they_ran_once_each() {
        let mut turn = TurnResult::begun();
        turn.take_in(&responded(10, Some(5), "OpenRouter", Some("Anthropic")));
        turn.take_in(&responded(20, Some(7), "OpenRouter", Some("Anthropic")));
        assert!(!turn.estimated);
        turn.take_in(&responded(1, Some(0), "Local server", None));
        assert_eq!(turn.tokens.input, 31);
        assert_eq!(turn.cost.map(|cost| cost.micros), Some(12));
        assert!(turn.estimated, "one call's usage was estimated");
        assert_eq!(turn.via, ["OpenRouter (Anthropic)", "Local server"]);
        turn.take_in(&responded(1, None, "OpenRouter", Some("OpenRouter")));
        assert_eq!(turn.cost, None, "a call that reported none: not reported");
        assert_eq!(
            turn.via,
            ["OpenRouter (Anthropic)", "Local server", "OpenRouter"],
            "a router that names itself is the service alone"
        );
    }

    #[test]
    fn a_subagents_work_is_its_parents_too() {
        let mut parent = TurnResult::begun();
        parent.take_in(&responded(10, Some(5), "OpenAI", Some("x")));
        parent.let_out(Lifted::Network);
        let mut child = TurnResult::begun();
        child.model_calls = 2;
        child.tool_calls = 3;
        child.take_in(&responded(4, Some(1), "OpenAI", Some("x")));
        child.let_out(Lifted::Sandbox);
        parent.changed_file("src/lib.rs");
        child.changed_file("src/main.rs");
        child.changed_file("src/lib.rs");
        parent.absorb(&child);
        assert_eq!(
            parent.changed,
            ["src/lib.rs", "src/main.rs"],
            "each file once"
        );
        assert_eq!((parent.model_calls, parent.tool_calls), (2, 3));
        assert_eq!((parent.outside_sandbox, parent.with_network), (1, 1));
        assert_eq!(parent.cost.map(|cost| cost.micros), Some(6));
        assert_eq!(parent.via, ["OpenAI (x)"]);
    }

    #[test]
    fn a_turn_recorded_with_its_outcome_alone_reads_with_nothing_counted() {
        let raw = RawValue::from_string(r#"{"outcome":"failed"}"#.into()).unwrap();
        let ended = decode_ended(1, &raw).unwrap().unwrap();
        assert_eq!(ended.outcome, Outcome::Failed);
        assert_eq!(
            (ended.model_calls, ended.cost, ended.via.len()),
            (0, None, 0)
        );
    }
}

//! What a turn did and what it cost: its calls, tokens and money, its subagents' added in.

use glave_event::{Currency, Lifted, Money, Outcome, TokenCounts};
use glave_ports::Completion;

/// What a turn did and what it cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnSummary {
    pub outcome: Outcome,
    pub model_calls: u32,
    pub tool_calls: u32,
    /// The commands that ran outside the sandbox, each one allowed.
    pub outside_sandbox: u32,
    /// The commands that ran with the network the sandbox otherwise kept from them, each one
    /// allowed.
    pub with_network: u32,
    pub tokens: TokenCounts,
    /// The total the provider reported, when it reported a cost for every call.
    pub cost: Option<Money>,
    /// True when any usage was estimated because a stream ended early.
    pub estimated: bool,
}

impl TurnSummary {
    pub(crate) fn started() -> Self {
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
        }
    }

    /// Counts one command allowed to run without what `lifted` says of the sandbox.
    pub(crate) fn let_out(&mut self, lifted: Lifted) {
        let count = match lifted {
            Lifted::Sandbox => &mut self.outside_sandbox,
            Lifted::Network => &mut self.with_network,
        };
        *count = count.saturating_add(1);
    }
}

/// Adds a child's work to its parent's turn: its calls, tokens and cost.
pub(crate) fn absorb(summary: &mut TurnSummary, child: &TurnSummary) {
    summary.model_calls = summary.model_calls.saturating_add(child.model_calls);
    summary.tool_calls = summary.tool_calls.saturating_add(child.tool_calls);
    summary.outside_sandbox = summary
        .outside_sandbox
        .saturating_add(child.outside_sandbox);
    summary.with_network = summary.with_network.saturating_add(child.with_network);
    summary.tokens += child.tokens;
    summary.estimated |= child.estimated;
    summary.cost = summary
        .cost
        .zip(child.cost)
        .and_then(|(sum, cost)| sum.checked_add(cost));
}

pub(crate) fn tally(summary: &mut TurnSummary, completion: &Completion) {
    summary.tokens += completion.usage.counts();
    summary.estimated |= completion.usage.estimated();
    summary.cost = summary
        .cost
        .zip(completion.cost.as_ref())
        .and_then(|(sum, cost)| sum.checked_add(cost.amount));
}

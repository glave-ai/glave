//! `[placement]` in `policy.toml`: where threads may work. `here` is the computer glave serves
//! from, `worker` a disposable computer of the thread's own, and `laptop` a person's own
//! computer dialled in to a server elsewhere (not available yet, so allowing it changes
//! nothing today). Rules intersect, as every allow-list does; a choice outside them is refused
//! naming the rule, and glave's own default gives way to what the rules leave.

use std::fmt;

use serde::Deserialize;

use crate::error::refuse;
use crate::policy::choices::{Choices, Chosen, Source};
use crate::policy::rules::{Constraints, Kind};
use crate::policy::workers::On;

/// A place a thread may work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Placement {
    Here,
    Worker,
    Laptop,
}

impl Placement {
    pub fn name(self) -> &'static str {
        match self {
            Self::Here => "here",
            Self::Worker => "worker",
            Self::Laptop => "laptop",
        }
    }

    /// Where the choice `on` puts threads.
    pub fn of(on: On) -> Self {
        match on {
            On::Here => Self::Here,
            On::Cloud => Self::Worker,
        }
    }
}

impl fmt::Display for Placement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// `[placement]` as written.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementRules {
    /// The places threads may work; every place when absent.
    pub allowed: Option<Vec<Placement>>,
}

impl Constraints {
    /// Whether every `[placement]` rule allows `place`.
    pub fn allows_placement(&self, place: Placement) -> bool {
        self.rules().iter().all(|rule| match &rule.kind {
            Kind::Placement(allowed) => allowed.contains(&place),
            _ => true,
        })
    }
}

/// `choices.on` within the `[placement]` rules: a choice someone made outside them is an error
/// naming the rule; glave's default, here, gives way to a worker when only that is allowed.
pub(crate) fn settle(mut choices: Choices, constraints: &Constraints) -> crate::Result<Choices> {
    let wanted = Placement::of(choices.on.value);
    if constraints.allows_placement(wanted) {
        return Ok(choices);
    }
    let rule = constraints
        .rules()
        .iter()
        .find(|rule| matches!(&rule.kind, Kind::Placement(allowed) if !allowed.contains(&wanted)));
    let Some(rule) = rule else {
        return Ok(choices);
    };
    if choices.on.from == Source::Default && constraints.allows_placement(Placement::Worker) {
        choices.on = Chosen {
            value: On::Cloud,
            from: Source::Narrowed(rule.from.clone()),
        };
        return Ok(choices);
    }
    refuse!(
        "threads working {} (on = {}, from {}) are not allowed: {rule}",
        wanted.name(),
        choices.on.value.name(),
        choices.on.from
    );
}

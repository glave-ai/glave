//! Choices within the constraints. A choice someone made that a rule does not allow is an
//! error naming the rule, its file, and where the choice came from; it is never replaced. Only
//! glave's own defaults give way, to the value the policy leaves.

use crate::error::refuse;
use glave_event::Permissions;
use glave_ports::settings::Judge;

use crate::policy::choices::{Choices, Chosen, Source};
use crate::policy::files::JudgeRule;
use crate::policy::rules::Constraints;

/// `choices`, kept within `constraints`. Fails on a choice outside them, and on any rule glave
/// cannot enforce yet.
pub(super) fn settle(mut choices: Choices, constraints: &Constraints) -> crate::Result<Choices> {
    if let Some((rule, why)) = constraints.not_enforced().first() {
        refuse!("glave will not run under {rule}: it is not enforced yet ({why})");
    }
    choices.permissions = permissions(choices.permissions, constraints)?;
    choices.judge = judge(choices.judge, constraints)?;
    choices.network = network(choices.network, constraints)?;
    crate::policy::memory::settle(&choices, constraints)?;
    crate::policy::placement::settle(choices, constraints)
}

/// `choices` within `constraints` for a server that already runs, and so cannot refuse to
/// start: a mode, a judge, the network or memory a rule refuses gives way as glave's default
/// would, each said in a sentence. A constraint added while it runs narrows its next turn
/// whatever the person's file says. Anything else outside them still fails.
pub(crate) fn give_way(
    mut choices: Choices,
    constraints: &Constraints,
) -> crate::Result<(Choices, Vec<String>)> {
    let mut aside = Vec::new();
    if let Err(refused) = permissions(choices.permissions.clone(), constraints) {
        aside.push(refused.to_string());
        choices.permissions.from = Source::Default;
    }
    if let Err(refused) = judge(choices.judge.clone(), constraints) {
        aside.push(refused.to_string());
        choices.judge = Chosen {
            value: Judge::Main,
            from: Source::Default,
        };
    }
    if let Err(refused) = network(choices.network.clone(), constraints) {
        aside.push(refused.to_string());
        choices.network.from = Source::Default;
    }
    if let Err(refused) = crate::policy::memory::settle(&choices, constraints) {
        aside.push(refused.to_string());
        choices.memory.from = Source::Default;
        choices.project_memory.from = Source::Default;
    }
    Ok((settle(choices, constraints)?, aside))
}

/// The mode within `constraints`: glave's default gives way to the highest a rule leaves,
/// and choosing one above it is refused.
fn permissions(
    chosen: Chosen<Permissions>,
    constraints: &Constraints,
) -> crate::Result<Chosen<Permissions>> {
    let (highest, Some(rule)) = constraints.highest_permissions() else {
        return Ok(chosen);
    };
    match (chosen.value <= highest, &chosen.from) {
        (true, _) => Ok(chosen),
        (false, Source::Default) => Ok(Chosen {
            value: highest,
            from: Source::Narrowed(rule.from.clone()),
        }),
        (false, from) => refuse!(
            "permissions {} (from {from}) is not allowed: {rule}",
            chosen.value.name()
        ),
    }
}

fn judge(chosen: Chosen<Judge>, constraints: &Constraints) -> crate::Result<Chosen<Judge>> {
    let Some((rule, path)) = constraints.judge()? else {
        return Ok(chosen);
    };
    let fits = match rule {
        JudgeRule::Required => chosen.value != Judge::Off,
        JudgeRule::Off => chosen.value == Judge::Off,
        JudgeRule::Allowed => true,
    };
    match (fits, &chosen.from) {
        (true, _) => Ok(chosen),
        // Only "off" can fail `required`, and the default is never off.
        (false, Source::Default) => Ok(Chosen {
            value: Judge::Off,
            from: Source::Narrowed(path.to_path_buf()),
        }),
        (false, from) => refuse!(
            "judge_model {} (from {from}) is not allowed: [approval] judge = {:?} in {}",
            chosen.value,
            rule.name(),
            path.display()
        ),
    }
}

/// The network commands get within `constraints`: glave's default gives way to off where a
/// rule takes it, and choosing it on there is refused.
fn network(chosen: Chosen<bool>, constraints: &Constraints) -> crate::Result<Chosen<bool>> {
    let Some(rule) = constraints.network_off() else {
        return Ok(chosen);
    };
    match (chosen.value, &chosen.from) {
        (false, _) => Ok(chosen),
        (true, Source::Default) => Ok(Chosen {
            value: false,
            from: Source::Narrowed(rule.from.clone()),
        }),
        (true, from) => refuse!("network on (from {from}) is not allowed: {rule}"),
    }
}

/// Refuses `model` (`provider/model`, chosen at `from`) when a rule does not allow it.
pub fn check_model(model: &str, from: &Source, constraints: &Constraints) -> crate::Result<()> {
    match constraints.refuses_model(model) {
        Some(rule) => refuse!("model {model} (from {from}) is not allowed: {rule}"),
        None => Ok(()),
    }
}

/// Refuses to start any of `servers`, people's own MCP servers, when a rule forbids them.
pub fn check_own_servers(servers: &[&str], constraints: &Constraints) -> crate::Result<()> {
    match (constraints.own_mcp_forbidden(), servers.is_empty()) {
        (Some(rule), false) => refuse!(
            "MCP servers {} are your own, and {rule} does not allow them: remove them from \
             mcp.json, or turn none on with mcp = [] in config.toml",
            servers.join(", ")
        ),
        _ => Ok(()),
    }
}

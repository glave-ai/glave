//! Choices within the constraints. A choice someone made that a rule does not allow is an
//! error naming the rule, its file, and where the choice came from; it is never replaced. Only
//! glave's own defaults give way, to the value the policy leaves.

use anyhow::bail;
use glave_harness::{ApprovalMode, SandboxLevel};

use crate::policy::choices::{Choices, Chosen, Judge, Source};
use crate::policy::files::JudgeRule;
use crate::policy::rules::Constraints;

/// `choices`, kept within `constraints`. Fails on a choice outside them, and on any rule glave
/// cannot enforce yet.
pub fn settle(mut choices: Choices, constraints: &Constraints) -> anyhow::Result<Choices> {
    if let Some((rule, why)) = constraints.not_enforced().first() {
        bail!("glave will not run under {rule}: it is not enforced yet ({why})");
    }
    choices.approval = approval(choices.approval, constraints)?;
    choices.judge = judge(choices.judge, constraints)?;
    choices.sandbox = sandbox(choices.sandbox, constraints)?;
    Ok(choices)
}

fn approval(
    chosen: Chosen<ApprovalMode>,
    constraints: &Constraints,
) -> anyhow::Result<Chosen<ApprovalMode>> {
    let (allowed, narrowing) = constraints.approval_modes();
    if allowed.contains(&chosen.value) {
        return Ok(chosen);
    }
    let rules: Vec<String> = narrowing.iter().map(ToString::to_string).collect();
    let rules = rules.join(", and ");
    match (allowed.first(), &chosen.from) {
        (None, _) => bail!("no approval mode is left: {rules}"),
        (Some(mode), Source::Default) => Ok(Chosen {
            value: *mode,
            from: narrowing
                .first()
                .map_or(Source::Default, |rule| Source::Narrowed(rule.from.clone())),
        }),
        (Some(_), from) => bail!(
            "approval {} (from {from}) is not allowed: {rules}",
            chosen.value.name()
        ),
    }
}

fn judge(chosen: Chosen<Judge>, constraints: &Constraints) -> anyhow::Result<Chosen<Judge>> {
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
        (false, from) => bail!(
            "judge_model {} (from {from}) is not allowed: [approval] judge = {:?} in {}",
            chosen.value,
            rule.name(),
            path.display()
        ),
    }
}

fn sandbox(
    chosen: Chosen<Option<SandboxLevel>>,
    constraints: &Constraints,
) -> anyhow::Result<Chosen<Option<SandboxLevel>>> {
    let Some(rule) = constraints.sandbox_required() else {
        return Ok(chosen);
    };
    match (chosen.value, &chosen.from) {
        (Some(_), _) => Ok(chosen),
        (None, Source::Default) => Ok(Chosen {
            value: Some(SandboxLevel::Project),
            from: Source::Narrowed(rule.from.clone()),
        }),
        (None, from) => bail!("sandbox off (from {from}) is not allowed: {rule}"),
    }
}

/// Refuses `model` (`provider/model`, chosen at `from`) when a rule does not allow it.
pub fn check_model(model: &str, from: &Source, constraints: &Constraints) -> anyhow::Result<()> {
    match constraints.refuses_model(model) {
        Some(rule) => bail!("model {model} (from {from}) is not allowed: {rule}"),
        None => Ok(()),
    }
}

/// Refuses to start any of `servers`, people's own MCP servers, when a rule forbids them.
pub fn check_own_servers(servers: &[&str], constraints: &Constraints) -> anyhow::Result<()> {
    match (constraints.own_mcp_forbidden(), servers.is_empty()) {
        (Some(rule), false) => bail!(
            "MCP servers {} are your own, and {rule} does not allow them: remove them from \
             mcp.json, or turn none on with mcp = [] in config.toml",
            servers.join(", ")
        ),
        _ => Ok(()),
    }
}

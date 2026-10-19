//! Choices within the constraints. A choice someone made that a rule does not allow is an
//! error naming the rule, its file, and where the choice came from; it is never replaced. Only
//! glave's own defaults give way, to the value the policy leaves.

use crate::error::refuse;
use glave_event::ApprovalMode;
use glave_ports::computer::SandboxLevel;
use glave_ports::settings::Judge;

use crate::policy::choices::{Choices, Chosen, Source};
use crate::policy::files::JudgeRule;
use crate::policy::rules::Constraints;

/// `choices`, kept within `constraints`. Fails on a choice outside them, and on any rule glave
/// cannot enforce yet.
pub fn settle(mut choices: Choices, constraints: &Constraints) -> crate::Result<Choices> {
    if let Some((rule, why)) = constraints.not_enforced().first() {
        refuse!("glave will not run under {rule}: it is not enforced yet ({why})");
    }
    choices.approval = approval(choices.approval, constraints)?;
    choices.judge = judge(choices.judge, constraints)?;
    choices.sandbox = sandbox(choices.sandbox, constraints)?;
    crate::policy::memory::settle(&choices, constraints)?;
    let choices = crate::policy::placement::settle(choices, constraints)?;
    Ok(asks_unconfined(choices, constraints))
}

/// `choices` within `constraints` for a server that already runs, and so cannot refuse to
/// start: an approval mode, a judge, a sandbox or memory a rule refuses gives way as glave's
/// default would, each said in a sentence. A constraint added while it runs narrows its next
/// turn whatever the person's file says. Anything else outside them still fails.
pub fn give_way(
    mut choices: Choices,
    constraints: &Constraints,
) -> crate::Result<(Choices, Vec<String>)> {
    let mut aside = Vec::new();
    if let Err(refused) = approval(choices.approval.clone(), constraints) {
        aside.push(refused.to_string());
        choices.approval.from = Source::Default;
    }
    if let Err(refused) = judge(choices.judge.clone(), constraints) {
        aside.push(refused.to_string());
        choices.judge = Chosen {
            value: Judge::Main,
            from: Source::Default,
        };
    }
    if let Err(refused) = sandbox(choices.sandbox.clone(), constraints) {
        aside.push(refused.to_string());
        choices.sandbox.from = Source::Default;
    }
    if let Err(refused) = crate::policy::memory::settle(&choices, constraints) {
        aside.push(refused.to_string());
        choices.memory.from = Source::Default;
        choices.project_memory.from = Source::Default;
    }
    Ok((settle(choices, constraints)?, aside))
}

/// `choices` once glave's default sandbox turned out not to run here, for the reason `why`:
/// no sandbox, and commands asking unless someone chose otherwise. A sandbox someone chose,
/// or a policy requires, never gets here: without it glave refuses to run commands.
pub fn unsandboxed(mut choices: Choices, why: &str, constraints: &Constraints) -> Choices {
    if choices.sandbox.from == Source::Default {
        choices.sandbox = Chosen {
            value: None,
            from: Source::Unavailable(why.to_owned()),
        };
    }
    asks_unconfined(choices, constraints)
}

/// Auto approval runs commands without asking, which is safe inside the sandbox. Outside it,
/// a command would run as the person with no one asked, so only someone who chose both
/// `sandbox = "off"` and `approval = "auto"` gets that, or a policy that leaves only `auto`.
/// Otherwise approval becomes `ask`.
fn asks_unconfined(mut choices: Choices, constraints: &Constraints) -> Choices {
    let chosen = |from: &Source| matches!(from, Source::Flag(_) | Source::File(_));
    let unconfined_by_choice = chosen(&choices.sandbox.from) && chosen(&choices.approval.from);
    let (allowed, _) = constraints.approval_modes();
    if choices.sandbox.value.is_none()
        && choices.approval.value == ApprovalMode::Auto
        && !unconfined_by_choice
        && allowed.contains(&ApprovalMode::Ask)
    {
        choices.approval = Chosen {
            value: ApprovalMode::Ask,
            from: Source::Unsandboxed,
        };
    }
    choices
}

/// The approval modes a thread may choose (`approval` in the session API): those the policy
/// leaves, cautious first. Auto runs only behind a wall: without the sandbox, a thread may
/// choose it only when glave already runs in auto (someone chose both, or the policy left only
/// auto).
pub fn thread_modes(choices: &Choices, constraints: &Constraints) -> Vec<ApprovalMode> {
    let (allowed, _) = constraints.approval_modes();
    let walled = choices.sandbox.value.is_some() || choices.approval.value == ApprovalMode::Auto;
    allowed
        .into_iter()
        .filter(|mode| *mode != ApprovalMode::Auto || walled)
        .collect()
}

fn approval(
    chosen: Chosen<ApprovalMode>,
    constraints: &Constraints,
) -> crate::Result<Chosen<ApprovalMode>> {
    let (allowed, narrowing) = constraints.approval_modes();
    if allowed.contains(&chosen.value) {
        return Ok(chosen);
    }
    let rules: Vec<String> = narrowing.iter().map(ToString::to_string).collect();
    let rules = rules.join(", and ");
    match (allowed.first(), &chosen.from) {
        (None, _) => refuse!("no approval mode is left: {rules}"),
        (Some(mode), Source::Default) => Ok(Chosen {
            value: *mode,
            from: narrowing
                .first()
                .map_or(Source::Default, |rule| Source::Narrowed(rule.from.clone())),
        }),
        (Some(_), from) => refuse!(
            "approval {} (from {from}) is not allowed: {rules}",
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

fn sandbox(
    chosen: Chosen<Option<SandboxLevel>>,
    constraints: &Constraints,
) -> crate::Result<Chosen<Option<SandboxLevel>>> {
    let Some(rule) = constraints.sandbox_required() else {
        return Ok(chosen);
    };
    match (chosen.value, &chosen.from) {
        // The default sandbox is now the policy's: it may not give way when it cannot run.
        (Some(level), Source::Default) => Ok(Chosen {
            value: Some(level),
            from: Source::Narrowed(rule.from.clone()),
        }),
        (Some(_), _) => Ok(chosen),
        (None, Source::Default) => Ok(Chosen {
            value: Some(SandboxLevel::Project),
            from: Source::Narrowed(rule.from.clone()),
        }),
        (None, from) => refuse!("sandbox off (from {from}) is not allowed: {rule}"),
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

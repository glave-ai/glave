//! `glave policy explain`: every effective setting and where it came from, every constraint and
//! the file that imposed it, and anything that stops glave from running.

use std::fmt::Write as _;

use crate::policy::Loaded;
use crate::policy::choices::{Choices, Source};
use crate::policy::effective::check_model;

/// The explanation of `loaded`, with models named `provider/model` when `provider` is known.
/// The second value is false when glave would refuse to run.
pub fn explain(loaded: &Loaded, provider: Option<&str>) -> (String, bool) {
    let settled = loaded.settle();
    let mut problems: Vec<String> = Vec::new();
    let choices = match &settled {
        Ok(choices) => choices,
        Err(error) => {
            problems.push(error.chain());
            &loaded.choices
        }
    };
    if let (Ok(_), Some(provider)) = (&settled, provider) {
        problems.extend(refused_models(choices, provider, loaded));
    }
    let mut out = String::new();
    out.push_str("Settings (the nearest wins: flags, environment, project, person, default)\n");
    out.push_str(&table(&settings(choices)));
    out.push_str("\nConfig files\n");
    if loaded.configs.is_empty() {
        out.push_str("  none\n");
    }
    for path in &loaded.configs {
        let _ = writeln!(out, "  {}", path.display());
    }
    if !loaded.person_hooks.is_empty() {
        out.push_str(
            "\nYour hooks (from your config.toml; a trusted project's are under Project)\n",
        );
        for hook in &loaded.person_hooks {
            let matcher = hook
                .matcher
                .as_deref()
                .map(|matcher| format!(" (for tools matching {matcher})"))
                .unwrap_or_default();
            let point = glave_harness::point_name(hook.point);
            let _ = writeln!(out, "  {point}  {}{matcher}", hook.command);
        }
        if let Some(rule) = loaded.constraints.own_hooks_forbidden() {
            problems.push(format!("your hooks are not allowed: {rule}"));
        }
    }
    if let Some(project) = &loaded.project {
        let _ = writeln!(out, "\nProject {}", project.root.display());
        let width = project
            .rows
            .iter()
            .map(|[what, _]| what.len())
            .max()
            .unwrap_or(0);
        for [what, standing] in &project.rows {
            let _ = writeln!(out, "  {what:<width$}  {standing}");
        }
    }
    out.push_str("\nConstraints (they only narrow, and a deny always wins)\n");
    let rules: Vec<[String; 3]> = loaded
        .constraints
        .rules()
        .iter()
        .map(|rule| {
            [
                rule.kind.to_string(),
                String::new(),
                rule.from.display().to_string(),
            ]
        })
        .collect();
    if rules.is_empty() {
        out.push_str("  none: no policy applies here, so everything is allowed\n");
    } else {
        out.push_str(&table(&rules));
    }
    if !problems.is_empty() {
        out.push_str("\nglave will not run here:\n");
        for problem in &problems {
            let _ = writeln!(out, "  {problem}");
        }
    }
    (out, problems.is_empty())
}

/// Each setting as `[name, value, source]`.
fn settings(choices: &Choices) -> Vec<[String; 3]> {
    let row = |name: &str, value: String, from: &Source| [name.to_owned(), value, from.to_string()];
    let sandbox = choices
        .sandbox
        .value
        .map_or("off", glave_harness::SandboxLevel::name);
    let mut rows = vec![
        match &choices.model {
            Some(model) => row("model", model.value.clone(), &model.from),
            None => row(
                "model",
                "none: set GLAVE_MODEL or pass --model".to_owned(),
                &Source::Default,
            ),
        },
        row(
            "judge_model",
            choices.judge.value.to_string(),
            &choices.judge.from,
        ),
        row(
            "approval",
            choices.approval.value.name().to_owned(),
            &choices.approval.from,
        ),
        row("sandbox", sandbox.to_owned(), &choices.sandbox.from),
        row(
            "network",
            if choices.network.value { "on" } else { "off" }.to_owned(),
            &choices.network.from,
        ),
        match &choices.mcp {
            Some(mcp) => row("mcp", format!("{:?}", mcp.value), &mcp.from),
            None => row(
                "mcp",
                "every server in mcp.json".to_owned(),
                &Source::Default,
            ),
        },
        row(
            "worktree",
            if choices.worktree.value { "on" } else { "off" }.to_owned(),
            &choices.worktree.from,
        ),
        row("on", choices.on.value.name().to_owned(), &choices.on.from),
        row("web", choices.web.value.describe(), &choices.web.from),
        row(
            "memory",
            if choices.memory.value { "on" } else { "off" }.to_owned(),
            &choices.memory.from,
        ),
        row(
            "space_memory",
            choices.space_memory.value.as_str().to_owned(),
            &choices.space_memory.from,
        ),
    ];
    if let Some(gateway) = &choices.gateway {
        rows.push(row("gateway", gateway.value.clone(), &gateway.from));
    }
    if let Some(store) = &choices.store {
        let shown = crate::policy::redacted(&store.value);
        rows.push(row("store", shown, &store.from));
    }
    if let Some(telemetry) = &choices.telemetry {
        rows.push(row("telemetry", telemetry.value.clone(), &telemetry.from));
    }
    rows
}

/// Why the main and judge models are refused, if they are. Through a gateway elsewhere, the
/// gateway decides, by its own policy, for each caller.
fn refused_models(choices: &Choices, provider: &str, loaded: &Loaded) -> Vec<String> {
    let Some(main) = &choices.model else {
        return Vec::new();
    };
    if choices.gateway.is_some() {
        return Vec::new();
    }
    let mut models = vec![(main.value.clone(), &main.from)];
    if let Some(judge) = choices.judge.value.model(&main.value)
        && judge != main.value
    {
        models.push((judge, &choices.judge.from));
    }
    models
        .into_iter()
        .filter_map(|(model, from)| {
            check_model(&format!("{provider}/{model}"), from, &loaded.constraints)
                .err()
                .map(|error| error.to_string())
        })
        .collect()
}

/// Rows in aligned columns, indented, with "from" before each source.
fn table(rows: &[[String; 3]]) -> String {
    let width = |column: usize| {
        rows.iter()
            .map(|row| row.get(column).map_or(0, String::len))
            .max()
            .unwrap_or(0)
    };
    let (first, second) = (width(0), width(1));
    let mut out = String::new();
    for [name, value, from] in rows {
        let line = if value.is_empty() {
            format!("  {name:<first$}  from {from}")
        } else {
            format!("  {name:<first$}  {value:<second$}  from {from}")
        };
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

//! The oracle — two-sided, behavior-based verdict.
//!   FAIL_TO_PASS  the targeted tests must now pass
//!   PASS_TO_PASS  the previously-passing tests must stay passing
//! Verdict is by exit code per group: deterministic, parser-free.

use anyhow::Result;

use crate::ports::WorkspaceRuntime;
use crate::task::TaskSpec;

#[allow(dead_code)] // p2p_output surfaced in richer failure traces later
pub struct Verdict {
    pub fail_to_pass_ok: bool,
    pub pass_to_pass_ok: bool,
    pub solved: bool,
    pub f2p_exit: i32,
    pub p2p_exit: Option<i32>,
    pub f2p_output: String,
    pub p2p_output: Option<String>,
}

pub fn evaluate(rt: &dyn WorkspaceRuntime, task: &TaskSpec) -> Result<Verdict> {
    let f2p_cmd = format!("{} {}", task.oracle.command, task.oracle.fail_to_pass.join(" "));
    let f2p = rt.run(&f2p_cmd)?;
    let fail_to_pass_ok = f2p.exit_code == 0;

    let (pass_to_pass_ok, p2p_exit, p2p_output) = if task.oracle.pass_to_pass.is_empty() {
        (true, None, None)
    } else {
        let cmd = format!("{} {}", task.oracle.command, task.oracle.pass_to_pass.join(" "));
        let r = rt.run(&cmd)?;
        (
            r.exit_code == 0,
            Some(r.exit_code),
            Some(merge(&r.stdout, &r.stderr)),
        )
    };

    Ok(Verdict {
        fail_to_pass_ok,
        pass_to_pass_ok,
        solved: fail_to_pass_ok && pass_to_pass_ok,
        f2p_exit: f2p.exit_code,
        p2p_exit,
        f2p_output: merge(&f2p.stdout, &f2p.stderr),
        p2p_output,
    })
}

fn merge(stdout: &str, stderr: &str) -> String {
    format!("{}\n{}", stdout.trim_end(), stderr.trim_end())
        .trim()
        .to_string()
}

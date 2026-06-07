//! The oracle — two-sided, behavior-based verdict.
//!   FAIL_TO_PASS  the targeted tests must now pass
//!   PASS_TO_PASS  the previously-passing tests must stay passing
//! Verdict is by exit code per group: deterministic, parser-free.

use anyhow::Result;

use crate::ports::WorkspaceRuntime;
use crate::task::TaskSpec;

pub struct Verdict {
    pub fail_to_pass_ok: bool,
    pub pass_to_pass_ok: bool,
    pub solved: bool,
    pub f2p_exit: i32,
    pub p2p_exit: Option<i32>,
    pub f2p_output: String,
}

pub fn evaluate(rt: &dyn WorkspaceRuntime, task: &TaskSpec) -> Result<Verdict> {
    let f2p = rt.run(&task.oracle.fail_to_pass)?;
    let fail_to_pass_ok = f2p.exit_code == 0;

    let (pass_to_pass_ok, p2p_exit) = match &task.oracle.pass_to_pass {
        None => (true, None),
        Some(cmd) => {
            let r = rt.run(cmd)?;
            (r.exit_code == 0, Some(r.exit_code))
        }
    };

    Ok(Verdict {
        fail_to_pass_ok,
        pass_to_pass_ok,
        solved: fail_to_pass_ok && pass_to_pass_ok,
        f2p_exit: f2p.exit_code,
        p2p_exit,
        f2p_output: merge(&f2p.stdout, &f2p.stderr),
    })
}

fn merge(stdout: &str, stderr: &str) -> String {
    format!("{}\n{}", stdout.trim_end(), stderr.trim_end())
        .trim()
        .to_string()
}

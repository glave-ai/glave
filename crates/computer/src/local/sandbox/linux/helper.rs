//! glave as the sandbox's helper, for Landlock: started as `glave __glave-sandbox <plan> --
//! <command>…`, it starts a session of its own, restricts itself with Landlock and the
//! seccomp filter, then becomes the command (`exec`), which keeps every restriction.

use std::ffi::OsString;

use super::landlocked::{self, Plan};
use super::seccomp;

/// Runs the helper on its arguments (after `__glave-sandbox`). Returns only on failure.
pub(in crate::local::sandbox) fn helper(args: &[OsString]) -> String {
    let [plan, separator, program, rest @ ..] = args else {
        return "usage: glave __glave-sandbox <plan> -- <command>…".into();
    };
    if separator != "--" {
        return "usage: glave __glave-sandbox <plan> -- <command>…".into();
    }
    let plan: Plan = match serde_json::from_str(&plan.to_string_lossy()) {
        Ok(plan) => plan,
        Err(error) => return format!("could not read the sandbox's plan: {error}"),
    };
    // A session of its own, so a command cannot push keystrokes into the person's terminal;
    // one that already leads its own session has that.
    if let Err(error) = rustix::process::setsid()
        && rustix::process::getsid(None) != Ok(rustix::process::getpid())
    {
        return format!("could not start a session: {error}");
    }
    if let Err(error) = landlocked::restrict(&plan) {
        return error;
    }
    if let Err(error) = seccomp::apply(plan.network) {
        return error;
    }
    glave_process::Program::new(program)
        .args(rest)
        .exec()
        .to_string()
}

//! `cargo xtask spawns`: every operating-system process glave starts goes through glave-process
//! (`crates/process`), so none starts without an owner (#361). Outside that crate, no
//! non-test source under `crates/` names `std::process::Command`, `tokio::process::Command`,
//! `CommandExt` or rmcp's `TokioChildProcess`. Tests (`sources.rs` says which files are) and
//! the test doubles of `glave-ports` start what they like.

use crate::sources;

/// Where processes may be started directly.
const OWNER: &str = "crates/process/";

/// What names a way to start a process other than glave-process.
const RAW: &[&str] = &["process::Command", "CommandExt", "TokioChildProcess"];

/// Fails on a source that starts a process another way.
pub(crate) fn check() -> Result<(), String> {
    let read = sources::read(&["crates"])?;
    let mut problems = Vec::new();
    let mut scanned = 0_usize;
    for (path, text) in read.iter().filter(|(path, _)| !path.starts_with(OWNER)) {
        scanned += 1;
        problems.extend(
            bypasses(text)
                .into_iter()
                .map(|(n, what)| format!("{path}:{n}: {what}")),
        );
    }
    if problems.is_empty() {
        println!("xtask: {scanned} sources start processes only through glave-process");
        Ok(())
    } else {
        Err(format!(
            "{} process(es) started outside glave-process; start them with \
             glave_process::Processes::spawn, or Processes::output for one waited for:\n  {}",
            problems.len(),
            problems.join("\n  ")
        ))
    }
}

/// Each line of `text`, by number from 1, that names a raw way to start a process, up to its
/// inline tests; comments aside.
fn bypasses(text: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut grouped = false;
    for (n, code) in sources::code(text) {
        // `use std::process::{Child, Command}`, across lines.
        if code.contains("process::{") {
            grouped = true;
        }
        let named = RAW
            .iter()
            .find(|raw| code.contains(*raw))
            .copied()
            .or_else(|| {
                (grouped && words(code).any(|word| word == "Command")).then_some("process::Command")
            });
        if grouped && code.contains('}') {
            grouped = false;
        }
        if let Some(named) = named {
            found.push((n, format!("starts a process with `{named}`")));
        }
    }
    found
}

fn words(code: &str) -> impl Iterator<Item = &str> {
    code.split(|c: char| !c.is_alphanumeric() && c != '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_raw_spawn_fails_the_check() {
        let raw = "fn go() {\n    let _ = std::process::Command::new(\"ls\").spawn();\n}\n";
        assert_eq!(
            bypasses(raw),
            [(2, "starts a process with `process::Command`".to_owned())]
        );
        let grouped = "use tokio::process::{\n    Child,\n    Command,\n};\n";
        assert_eq!(bypasses(grouped).len(), 1);
        assert_eq!(
            bypasses("use std::os::unix::process::CommandExt as _;\n").len(),
            1
        );
        assert_eq!(bypasses("let t = TokioChildProcess::new(c);\n").len(), 1);
    }

    #[test]
    fn glave_s_own_commands_comments_and_tests_pass() {
        let fine = "use glave_ports::computer::Command;\nuse tokio::process::{Child, ChildStdout};\n\
            // std::process::Command is not used here\nlet c = Command::new(\"ls\");\n\
            #[cfg(test)]\nmod tests {\n    use std::process::Command;\n}\n";
        assert!(bypasses(fine).is_empty(), "{:?}", bypasses(fine));
    }
}

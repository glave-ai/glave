//! `cargo xtask spawns`: every operating-system process glave starts goes through glave-process
//! (`crates/process`), so none starts without an owner (#361). Outside that crate, no
//! non-test source under `crates/` names `std::process::Command`, `tokio::process::Command`,
//! `CommandExt` or rmcp's `TokioChildProcess`. Tests (`tests/`, `benches/`, `tests.rs`,
//! `*_tests.rs`, and an inline `mod tests` to the end of its file) and the test doubles of
//! `glave-ports` start what they like.

use crate::{cmd, workspace};

/// Where processes may be started directly.
const OWNER: &str = "crates/process/";

/// What names a way to start a process other than glave-process.
const RAW: &[&str] = &["process::Command", "CommandExt", "TokioChildProcess"];

/// Fails on a source that starts a process another way.
pub fn check() -> Result<(), String> {
    let root = workspace::root()?;
    let listed = cmd::output("git", &["ls-files", "-z", "crates"])?;
    let mut problems = Vec::new();
    let mut scanned = 0_usize;
    for path in listed.split('\0').filter(|path| checked(path)) {
        let text = std::fs::read_to_string(root.join(path))
            .map_err(|err| format!("could not read {path}: {err}"))?;
        scanned += 1;
        problems.extend(
            bypasses(&text)
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
             glave_process::Processes::spawn, or Program::output for one waited for:\n  {}",
            problems.len(),
            problems.join("\n  ")
        ))
    }
}

/// Whether `path` is a non-test Rust source outside glave-process.
fn checked(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    std::path::Path::new(path)
        .extension()
        .is_some_and(|extension| extension == "rs")
        && !path.starts_with(OWNER)
        && !path.starts_with("crates/ports/src/testing")
        && !path.contains("/tests/")
        && !path.contains("/benches/")
        && name != "tests.rs"
        && !name.ends_with("_tests.rs")
}

/// Each line of `text`, by number from 1, that names a raw way to start a process, up to its
/// inline tests; comments aside.
fn bypasses(text: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut lines = text.lines().enumerate().peekable();
    let mut grouped = false;
    while let Some((n, line)) = lines.next() {
        let code = line.trim_start();
        let tests_follow = lines
            .peek()
            .is_some_and(|(_, next)| next.trim_start().starts_with("mod tests"));
        if code.starts_with("#[cfg(test)]") && tests_follow {
            break;
        }
        if code.starts_with("//") {
            continue;
        }
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
            found.push((n + 1, format!("starts a process with `{named}`")));
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
        assert!(checked("crates/cli/src/daemon.rs"));
        assert!(!checked("crates/process/src/program.rs"));
        assert!(!checked("crates/cli/tests/serve.rs"));
        assert!(!checked("crates/computer/src/local/tests.rs"));
        assert!(!checked("crates/computer/src/local/confinement_tests.rs"));
        assert!(!checked("crates/cli/benches/budgets.rs"));
        assert!(!checked("web/app/main.ts"));
    }
}

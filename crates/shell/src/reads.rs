//! Commands that only read, which `ask` mode lets run without asking. Git is not among them:
//! a repository's own settings can make even `git status` run programs, as Codex found.

use crate::parse::{Command, Word};
use crate::paths::basename;

/// Programs that only read, whatever their arguments.
const READERS: &[&str] = &[
    "ls", "cat", "head", "tail", "wc", "pwd", "echo", "printf", "grep", "egrep", "fgrep", "stat",
    "file", "du", "df", "which", "whereis", "uname", "whoami", "id", "date", "tree", "uniq", "cut",
    "tr", "diff", "cmp", "basename", "dirname", "realpath", "readlink", "true", "false", "seq",
    "nl", "tac", "rev", "column", "jq", "cd", "test", "[",
];

/// Whether `command` only reads.
pub(crate) fn reads_only(command: &Command) -> bool {
    if !command.writes.is_empty() {
        return false;
    }
    let Some(name) = command.words.first().and_then(Word::literal).map(basename) else {
        return false;
    };
    let args: Vec<&str> = command.words.iter().skip(1).map(Word::text).collect();
    match name {
        // Sorting can write its result to a file.
        "sort" => !args
            .iter()
            .any(|a| a.starts_with("-o") || a.starts_with("--output")),
        // Searching may run another program on each file.
        "rg" => !args
            .iter()
            .any(|a| a.starts_with("--pre") || a.starts_with("--hostname-bin")),
        "find" => !args.iter().any(|a| {
            matches!(
                *a,
                "-exec"
                    | "-execdir"
                    | "-ok"
                    | "-okdir"
                    | "-delete"
                    | "-fls"
                    | "-fprint"
                    | "-fprint0"
                    | "-fprintf"
            )
        }),
        name => READERS.contains(&name),
    }
}

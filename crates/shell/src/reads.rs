//! Commands that only read, which `ask` mode lets run without asking. Git is not among them:
//! a repository's own settings can make even `git status` run programs, as Codex found.

use crate::parse::{Command, Word};
use crate::paths::basename;

/// Programs that only read, whatever their arguments.
const READERS: &[&str] = &[
    "ls", "cat", "head", "tail", "wc", "pwd", "echo", "printf", "grep", "egrep", "fgrep", "stat",
    "file", "du", "df", "which", "whereis", "uname", "whoami", "id", "date", "cut", "tr", "diff",
    "cmp", "basename", "dirname", "realpath", "readlink", "true", "false", "seq", "nl", "tac",
    "rev", "column", "jq", "cd", "test", "[",
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
        // Sorting can write its result to a file (`-o`, in a cluster too: `-uo`), and run a
        // program on what it keeps aside.
        "sort" => {
            let given = given(&args, &['k', 'o', 'S', 't', 'T']);
            !given.short.contains(&'o')
                && !given
                    .long
                    .iter()
                    .any(|long| abbreviates(long, &["output", "compress-program"]))
        }
        // A second operand is the file it writes.
        "uniq" => given(&args, &['f', 's', 'w']).operands.len() < 2,
        // Each letter of a cluster is an option of its own: `-o` writes the listing to a file,
        // and `-R` one into every folder it lists.
        "tree" => !args.iter().any(|a| {
            a.strip_prefix('-')
                .is_some_and(|cluster| !cluster.starts_with('-') && cluster.contains(['o', 'R']))
        }),
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

/// What a program's arguments give, as its options are read: each short option in order
/// (`-uo` gives `u` and `o`), each long option's name (`--output=f` gives `output`), and the
/// operands. A short option in `valued` takes a value: the rest of its argument, or the next
/// one. A lone `--` ends the options.
struct Given<'a> {
    short: Vec<char>,
    long: Vec<&'a str>,
    operands: Vec<&'a str>,
}

fn given<'a>(args: &[&'a str], valued: &[char]) -> Given<'a> {
    let mut given = Given {
        short: Vec::new(),
        long: Vec::new(),
        operands: Vec::new(),
    };
    let mut args = args.iter().copied();
    while let Some(arg) = args.next() {
        if arg == "--" {
            given.operands.extend(args);
            break;
        }
        if let Some(long) = arg.strip_prefix("--") {
            given
                .long
                .push(long.split_once('=').map_or(long, |(name, _)| name));
            continue;
        }
        let Some(cluster) = arg.strip_prefix('-').filter(|cluster| !cluster.is_empty()) else {
            given.operands.push(arg);
            continue;
        };
        for (at, option) in cluster.char_indices() {
            given.short.push(option);
            if valued.contains(&option) {
                if at.saturating_add(option.len_utf8()) == cluster.len() {
                    args.next();
                }
                break;
            }
        }
    }
    given
}

/// Whether `name`, a long option as given, names one of `options`, whole or by a prefix, as
/// programs that read options with GNU's getopt take one.
fn abbreviates(name: &str, options: &[&str]) -> bool {
    !name.is_empty() && options.iter().any(|option| option.starts_with(name))
}

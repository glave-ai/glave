//! What `find` does to what it finds: `-delete` removes it, and `-exec`, `-execdir`, `-ok` and
//! `-okdir` run a command on it, read as a script of its own with `{}` standing for what is
//! found under each folder searched: the folder itself, when nothing before the action tests
//! what is found, else something inside it.

use std::path::Path;

use glave_event::Risk;

use crate::judge::{Caution, Context, Kind};
use crate::parse::Word;
use crate::paths::{basename, critical, inside};
use crate::rules::{found, target};

/// Options that hold for the whole search, which test nothing that is found.
const SEARCH_OPTIONS: &[&str] = &[
    "-depth",
    "-d",
    "-xdev",
    "-x",
    "-mount",
    "-follow",
    "-noleaf",
    "-ignore_readdir_race",
    "-noignore_readdir_race",
    "-daystart",
    "-warn",
    "-nowarn",
    "(",
    ")",
];

/// Those that take a value.
const VALUED_SEARCH_OPTIONS: &[&str] = &["-maxdepth", "-mindepth", "-regextype"];

/// The folders `find`'s words search, and where its expression starts. With none named, it
/// searches the folder it runs in.
fn roots(words: &[Word]) -> (Vec<Word>, usize) {
    let mut at = 1;
    // Options before the folders: `-H`, `-L`, `-P`, `-O3`, `-D` with a value, and `--`.
    while let Some(word) = words.get(at).and_then(Word::literal) {
        match word {
            "-D" => at = at.saturating_add(2),
            "-H" | "-L" | "-P" => at = at.saturating_add(1),
            "--" => {
                at = at.saturating_add(1);
                break;
            }
            _ if word.starts_with("-O") => at = at.saturating_add(1),
            _ => break,
        }
    }
    let start = at;
    while words.get(at).is_some_and(|word| {
        !word
            .literal()
            .is_some_and(|text| text.starts_with('-') || matches!(text, "(" | "!" | ","))
    }) {
        at = at.saturating_add(1);
    }
    let named = words.get(start..at).unwrap_or_default();
    let roots = if named.is_empty() {
        vec![Word::Literal(".".to_owned())]
    } else {
        named.to_vec()
    };
    (roots, at)
}

/// Whether nothing in `expression` tests what is found, so an action after it takes every
/// thing, the folder searched first of all.
fn untested(expression: &[Word]) -> bool {
    let mut words = expression.iter();
    while let Some(word) = words.next() {
        match word.literal() {
            Some(option) if VALUED_SEARCH_OPTIONS.contains(&option) => {
                words.next();
            }
            Some(option) if SEARCH_OPTIONS.contains(&option) => {}
            _ => return false,
        }
    }
    true
}

/// What `find` does, in order: where each action is among its words, and for a command, its
/// words. A command with no `;` or `+` to end it is left out: `find` refuses to run it.
fn actions(words: &[Word], expression: usize) -> Vec<(usize, Option<&[Word]>)> {
    let mut actions = Vec::new();
    let mut at = expression;
    while let Some(word) = words.get(at) {
        match word.literal() {
            Some("-delete") => actions.push((at, None)),
            Some("-exec" | "-execdir" | "-ok" | "-okdir") => {
                let start = at.saturating_add(1);
                let rest = words.get(start..).unwrap_or_default();
                let Some(end) = rest
                    .iter()
                    .position(|word| matches!(word.literal(), Some(";" | "+")))
                else {
                    break;
                };
                actions.push((at, rest.get(..end)));
                at = start.saturating_add(end);
            }
            _ => {}
        }
        at = at.saturating_add(1);
    }
    actions
}

/// `find … -delete` of a folder no project command should remove, with nothing tested first,
/// or of what lies inside a folder no project command should empty, outside the project, or
/// inside a folder it cannot name.
pub(crate) fn deletes(
    words: &[Word],
    cwd: Option<&Path>,
    context: &Context<'_>,
) -> Option<Caution> {
    let (roots, expression) = roots(words);
    actions(words, expression)
        .into_iter()
        .filter(|(_, command)| command.is_none())
        .find_map(|(at, _)| {
            let itself = untested(words.get(expression..at).unwrap_or_default());
            roots
                .iter()
                .find_map(|root| deleted(root, itself, cwd, context))
        })
}

/// What deleting what is found under `root` (`itself` too) needs the person for, if anything.
fn deleted(
    root: &Word,
    itself: bool,
    cwd: Option<&Path>,
    context: &Context<'_>,
) -> Option<Caution> {
    let Some(folder) = target(root, cwd, context) else {
        let what = root.text().to_owned();
        return Some(found(Kind::Deletes, Risk::DeletesUnknown { what }));
    };
    let within = if itself {
        folder.clone()
    } else {
        folder.join("{}")
    };
    let path = folder.display().to_string();
    if critical(&within, context.project, context.home) {
        Some(found(Kind::Deletes, Risk::Deletes { path }))
    } else if !inside(&within, context.project) && !context.temporary(&within) {
        Some(found(Kind::Deletes, Risk::DeletesOutside { path }))
    } else {
        None
    }
}

/// The commands `find`'s words run on what they find, each as a script, once for each folder
/// searched (one for every folder only known when it runs): `{}` is the folder itself when
/// nothing before the command tests what is found, else something inside it. `None` when the
/// scripts would be longer than `most` bytes in all.
pub(crate) fn scripts(words: &[Word], most: usize) -> Option<Vec<String>> {
    if words.first().and_then(Word::literal).map(basename) != Some("find") {
        return Some(Vec::new());
    }
    let (roots, expression) = roots(words);
    let roots = distinct(&roots);
    let mut scripts = Vec::new();
    let mut left = most;
    for (at, command) in actions(words, expression) {
        let Some(command) = command else {
            continue;
        };
        let itself = untested(words.get(expression..at).unwrap_or_default());
        for root in &roots {
            let mut script = String::new();
            for word in command {
                let made = found_in(word, root, itself, left.saturating_sub(script.len()))?;
                script.push_str(&made);
                script.push(' ');
                if script.len() > left {
                    return None;
                }
            }
            left = left.saturating_sub(script.len());
            scripts.push(script);
        }
    }
    Some(scripts)
}

/// Each folder named once, and one of those only known when it runs for them all: what is
/// found in any of them is as unknown.
fn distinct(roots: &[Word]) -> Vec<&Word> {
    let mut kept: Vec<&Word> = Vec::new();
    for root in roots {
        let same = |seen: &&Word| match (root, *seen) {
            (Word::Dynamic(_), Word::Dynamic(_)) => true,
            (root, seen) => root == seen,
        };
        if !kept.iter().any(same) {
            kept.push(root);
        }
    }
    kept
}

/// `word` as a script writes it, each `{}` in it what is found under `root`: the folder itself
/// when `itself`, else something inside it. `None` when that would be longer than `most`.
fn found_in(word: &Word, root: &Word, itself: bool, most: usize) -> Option<String> {
    let Word::Literal(text) = word else {
        return Some(word.quoted());
    };
    let times = text.matches("{}").count();
    if times == 0 {
        return Some(word.quoted());
    }
    let folder = root.text();
    let longest = times
        .saturating_mul(folder.len().saturating_add(5))
        .saturating_add(text.len().saturating_mul(4))
        .saturating_add(2);
    if longest > most {
        return None;
    }
    let found = match (root, itself) {
        (Word::Literal(_), true) => folder.to_owned(),
        (Word::Literal(_), false) => format!("{}/{{}}", folder.trim_end_matches('/')),
        // A folder only known when it runs: what is found in it can't be known either.
        (Word::Dynamic(_), true) => {
            return Some(join(text, folder));
        }
        (Word::Dynamic(_), false) => {
            return Some(join(text, &format!("{folder}/'{{}}'")));
        }
    };
    Some(Word::Literal(text.replace("{}", &found)).quoted())
}

/// `text` quoted, each `{}` in it `source` as a script writes it.
fn join(text: &str, source: &str) -> String {
    text.split("{}")
        .map(|part| Word::Literal(part.to_owned()).quoted())
        .collect::<Vec<_>>()
        .join(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn literal(words: &[&str]) -> Vec<Word> {
        words
            .iter()
            .map(|w| Word::Literal((*w).to_owned()))
            .collect()
    }

    #[test]
    fn the_folders_searched_come_before_the_expression() {
        let (named, at) = roots(&literal(&["find", "-L", "src", "/tmp", "-name", "x"]));
        assert_eq!(named, literal(&["src", "/tmp"]));
        assert_eq!(at, 4);
        let (named, _) = roots(&literal(&["find", "-delete"]));
        assert_eq!(named, literal(&["."]));
        let (named, _) = roots(&literal(&["find", "-P", "--", "-odd", "-delete"]));
        assert_eq!(named, literal(&["."]), "`--` ends the options");
    }

    #[test]
    fn each_command_runs_on_what_is_found_in_each_folder() {
        let words = literal(&[
            "find", "/usr", "src", "/usr", "-name", "x", "-exec", "rm", "-rf", "{}", ";", "-ok",
            "echo", "{}.bak", "+",
        ]);
        assert_eq!(
            scripts(&words, 1024).unwrap(),
            vec![
                "'rm' '-rf' '/usr/{}' ",
                "'rm' '-rf' 'src/{}' ",
                "'echo' '/usr/{}.bak' ",
                "'echo' 'src/{}.bak' ",
            ]
        );
        // With nothing tested first, the folder itself is found first.
        let words = literal(&["find", ".", "-maxdepth", "1", "-exec", "rm", "{}", "+"]);
        assert_eq!(scripts(&words, 1024).unwrap(), vec!["'rm' '.' "]);
        let dynamic = vec![
            Word::Literal("find".to_owned()),
            Word::Dynamic("\"$D\"".to_owned()),
            Word::Dynamic("$E".to_owned()),
            Word::Literal("-type".to_owned()),
            Word::Literal("f".to_owned()),
            Word::Literal("-exec".to_owned()),
            Word::Literal("rm".to_owned()),
            Word::Literal("{}".to_owned()),
            Word::Literal(";".to_owned()),
        ];
        assert_eq!(
            scripts(&dynamic, 1024).unwrap(),
            vec!["'rm' ''\"$D\"/'{}''' "]
        );
        // A command find would refuse, with nothing to end it, runs nothing.
        let unended = literal(&["find", ".", "-exec", "rm", "-rf", "{}"]);
        assert_eq!(scripts(&unended, 1024).unwrap(), Vec::<String>::new());
        assert_eq!(scripts(&words, 4), None, "longer than allowed");
    }
}

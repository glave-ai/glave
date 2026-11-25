//! What `find` does to what it finds: `-delete` removes it, and `-exec`, `-execdir`, `-ok` and
//! `-okdir` run a command on it, read as a script of its own with `{}` standing for something
//! found inside each folder searched.

use std::path::Path;

use glave_event::Risk;

use crate::judge::{Caution, Context, Kind};
use crate::parse::Word;
use crate::paths::{basename, critical, inside};
use crate::rules::{found, target};

/// The folders `find`'s words search, and where its expression starts. With none named, it
/// searches the folder it runs in.
fn roots(words: &[Word]) -> (Vec<Word>, usize) {
    let mut at = 1;
    // Options before the folders: `-H`, `-L`, `-P`, `-O3`, and `-D` with a value.
    while let Some(word) = words.get(at).and_then(Word::literal) {
        match word {
            "-D" => at = at.saturating_add(2),
            "-H" | "-L" | "-P" => at = at.saturating_add(1),
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

/// `find … -delete` of what lies inside a folder no project command should empty, or outside the
/// project, or inside a folder it cannot name.
pub(crate) fn deletes(
    words: &[Word],
    cwd: Option<&Path>,
    context: &Context<'_>,
) -> Option<Caution> {
    let (roots, expression) = roots(words);
    let deleting = words
        .iter()
        .skip(expression)
        .any(|word| word.literal() == Some("-delete"));
    if !deleting {
        return None;
    }
    roots.iter().find_map(|root| {
        let Some(folder) = target(root, cwd, context) else {
            let what = root.text().to_owned();
            return Some(found(Kind::Deletes, Risk::DeletesUnknown { what }));
        };
        // What it finds lies inside the folder: the folder itself goes only when it matches.
        let within = folder.join("{}");
        let path = folder.display().to_string();
        if critical(&within, context.project, context.home) {
            Some(found(Kind::Deletes, Risk::Deletes { path }))
        } else if !inside(&within, context.project) && !context.temporary(&within) {
            Some(found(Kind::Deletes, Risk::DeletesOutside { path }))
        } else {
            None
        }
    })
}

/// The commands `find`'s words run on what they find, each as a script, once for each folder
/// searched: `{}` is something found inside it.
pub(crate) fn scripts(words: &[Word]) -> Vec<String> {
    if words
        .first()
        .and_then(Word::literal)
        .map(basename)
        != Some("find")
    {
        return Vec::new();
    }
    let (roots, expression) = roots(words);
    let mut scripts = Vec::new();
    let mut rest = words.get(expression..).unwrap_or_default().iter();
    while let Some(word) = rest.next() {
        if !matches!(
            word.literal(),
            Some("-exec" | "-execdir" | "-ok" | "-okdir")
        ) {
            continue;
        }
        let command: Vec<&Word> = rest
            .by_ref()
            .take_while(|word| !matches!(word.literal(), Some(";" | "+")))
            .collect();
        for root in &roots {
            let words: Vec<String> = command.iter().map(|word| found_in(word, root)).collect();
            scripts.push(words.join(" "));
        }
    }
    scripts
}

/// `word` as a script writes it, each `{}` in it something found inside `root`.
fn found_in(word: &Word, root: &Word) -> String {
    match (word, root) {
        (Word::Literal(text), Word::Literal(folder)) if text.contains("{}") => {
            let inside = format!("{}/{{}}", folder.trim_end_matches('/'));
            Word::Literal(text.replace("{}", &inside)).quoted()
        }
        // A folder only known when it runs: what is found in it can't be known either.
        (Word::Literal(text), Word::Dynamic(folder)) if text.contains("{}") => text
            .split("{}")
            .map(|part| Word::Literal(part.to_owned()).quoted())
            .collect::<Vec<_>>()
            .join(&format!("{folder}/'{{}}'")),
        (word, _) => word.quoted(),
    }
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
        let (roots, at) = roots(&literal(&["find", "-L", "src", "/tmp", "-name", "x"]));
        assert_eq!(roots, literal(&["src", "/tmp"]));
        assert_eq!(at, 4);
        let (roots, _) = roots(&literal(&["find", "-delete"]));
        assert_eq!(roots, literal(&["."]));
    }

    #[test]
    fn each_command_runs_on_what_is_found_in_each_folder() {
        let words = literal(&[
            "find", "/usr", "src", "-exec", "rm", "-rf", "{}", ";", "-ok", "echo", "{}.bak", "+",
        ]);
        assert_eq!(
            scripts(&words),
            vec![
                "'rm' '-rf' '/usr/{}'",
                "'rm' '-rf' 'src/{}'",
                "'echo' '/usr/{}.bak'",
                "'echo' 'src/{}.bak'",
            ]
        );
        let dynamic = vec![
            Word::Literal("find".to_owned()),
            Word::Dynamic("\"$D\"".to_owned()),
            Word::Literal("-exec".to_owned()),
            Word::Literal("rm".to_owned()),
            Word::Literal("{}".to_owned()),
            Word::Literal(";".to_owned()),
        ];
        assert_eq!(scripts(&dynamic), vec!["'rm' ''\"$D\"/'{}'''"]);
    }
}

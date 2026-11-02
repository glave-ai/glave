//! What the person is shown of a skill `install` adds, under the question: its frontmatter, its
//! text and the files it brings, each cut to a size the event can carry, and what was cut said
//! in a sentence. Text only, which each client draws; control characters other than a line's end
//! and a tab are left out, so nothing written can move a terminal's cursor or colour.

use std::collections::BTreeMap;
use std::path::{Component, Path};

use glave_event::{AddedFile, ServiceAdds};

use crate::skills::SKILL_MD;
use crate::skills::frontmatter;

/// The most of a SKILL.md's frontmatter shown.
const MOST_FRONT: usize = 4 * 1024;
/// The most of a SKILL.md's text shown.
const MOST_TEXT: usize = 16 * 1024;
/// The most of one file shown.
const MOST_FILE: usize = 8 * 1024;
/// The most of all the files together shown; past it, a file shows its path alone.
const MOST_FILES: usize = 48 * 1024;

/// The most files a skill brings.
pub(super) const MOST_COUNT: usize = 64;
/// The longest path a file it brings may have.
const MOST_PATH: usize = 200;

/// What the person is shown of the skill `text` and its `files`.
pub(super) fn adds(text: &str, files: &BTreeMap<String, String>) -> ServiceAdds {
    let (front, body) = match frontmatter::parse(text) {
        Ok(document) => {
            let head = text
                .strip_suffix(document.body.as_str())
                .unwrap_or_default();
            let lines = head.lines().filter(|line| line.trim_end() != "---");
            (lines.collect::<Vec<_>>().join("\n"), document.body)
        }
        Err(_) => (String::new(), text.to_owned()),
    };
    let (front, front_cut) = start(&front, MOST_FRONT);
    let (text, body_cut) = start(body.trim_start_matches(['\r', '\n']), MOST_TEXT);
    let mut left = MOST_FILES;
    let files: Vec<AddedFile> = files
        .iter()
        .map(|(path, text)| {
            let (shown, cut) = start(text, MOST_FILE.min(left));
            left = left.saturating_sub(shown.len());
            AddedFile {
                path: path.clone(),
                text: shown,
                runs_code: runs_code(path, text),
                cut,
            }
        })
        .collect();
    let mut adds = ServiceAdds {
        front,
        text,
        text_cut: front_cut || body_cut,
        files,
        cut: None,
    };
    adds.cut = match adds.parts_cut() {
        0 => None,
        parts => Some(cut_sentence(parts)),
    };
    adds
}

/// What was cut, in glave's words: as the session's words say it (`adds-cut`).
pub(crate) fn cut_sentence(parts: u64) -> String {
    let parts = match parts {
        1 => "1 part".to_owned(),
        parts => format!("{parts} parts"),
    };
    format!("Too long to show in full here: {parts} cut short. All of it is added as written.")
}

/// Whether the file at `path` is a program the skill runs: in `scripts/`, as the Agent Skills
/// format keeps them, or starting with `#!`.
fn runs_code(path: &str, text: &str) -> bool {
    path.starts_with("scripts/") || text.starts_with("#!")
}

/// As much of `text` as `most` bytes hold, ending at a line's end where one fits, without
/// control characters but line ends and tabs; and whether any was left out.
fn start(text: &str, most: usize) -> (String, bool) {
    let shown = if text.len() <= most {
        text
    } else {
        let mut end = most;
        while !text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        let head = text.get(..end).unwrap_or_default();
        head.rfind('\n')
            .map_or(head, |at| head.get(..at).unwrap_or(head))
    };
    let kept = |c: &char| !c.is_control() || matches!(c, '\n' | '\t');
    (
        shown.chars().filter(kept).collect(),
        shown.len() < text.len(),
    )
}

/// Why `path` can't be a file a skill brings: one within its folder, plain parts only, not its
/// SKILL.md.
pub(super) fn check_path(path: &str) -> Result<(), String> {
    let plain = !path.is_empty()
        && path.len() <= MOST_PATH
        && !path.contains('\\')
        && !path.ends_with('/')
        && !path.chars().any(char::is_control)
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)));
    if !plain {
        return Err(format!(
            "The file {path:?} must be a path within the skill's folder, such as \
             scripts/run.sh, of at most {MOST_PATH} characters."
        ));
    }
    if path == SKILL_MD {
        return Err("Give the SKILL.md as skill, not among its files.".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(each: &[(&str, &str)]) -> BTreeMap<String, String> {
        each.iter()
            .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
            .collect()
    }

    #[test]
    fn the_frontmatter_the_text_and_each_file_are_shown_apart() {
        let skill = "---\nname: tidy\ndescription: Tidies.\n---\n\n# Tidy\n\nRun it.\n";
        let shown = adds(
            skill,
            &files(&[
                ("scripts/tidy.py", "print(1)\n"),
                ("bin/go", "#!/bin/sh\necho\n"),
                ("references/style.md", "# Style\n"),
            ]),
        );
        assert_eq!(shown.front, "name: tidy\ndescription: Tidies.");
        assert_eq!(shown.text, "# Tidy\n\nRun it.\n");
        assert!(!shown.text_cut);
        assert_eq!(shown.cut, None);
        let runs: Vec<(&str, bool)> = shown
            .files
            .iter()
            .map(|file| (file.path.as_str(), file.runs_code))
            .collect();
        assert_eq!(
            runs,
            [
                ("bin/go", true),
                ("references/style.md", false),
                ("scripts/tidy.py", true)
            ]
        );
    }

    #[test]
    fn what_is_too_long_is_cut_at_a_line_and_said() {
        let long = "a line of the skill\n".repeat(2000);
        let skill = format!("---\nname: big\ndescription: Big.\n---\n{long}");
        let big = "x".repeat(MOST_FILE * 2);
        let many: Vec<(String, String)> = (0..8)
            .map(|at| (format!("references/{at}.md"), big.clone()))
            .collect();
        let many: BTreeMap<String, String> = many.into_iter().collect();
        let shown = adds(&skill, &many);
        assert!(shown.text_cut);
        assert!(shown.text.len() <= MOST_TEXT);
        assert!(shown.text.ends_with("skill"), "cut at a line's end");
        let total: usize = shown.files.iter().map(|file| file.text.len()).sum();
        assert!(total <= MOST_FILES);
        assert!(shown.files.iter().all(|file| file.cut));
        assert_eq!(shown.files.len(), 8, "every file is named");
        assert_eq!(
            shown.cut.as_deref(),
            Some(
                "Too long to show in full here: 9 parts cut short. All of it is added as written."
            )
        );
    }

    #[test]
    fn control_characters_are_left_out_of_what_is_shown() {
        let shown = adds(
            "---\nname: a\ndescription: A.\n---\nred \u{1b}[31mtext\tok\n",
            &BTreeMap::new(),
        );
        assert_eq!(shown.text, "red [31mtext\tok\n");
    }

    #[test]
    fn a_file_stays_within_the_skills_folder() {
        for path in ["scripts/run.sh", "a/b/c.md", "notes"] {
            assert_eq!(check_path(path), Ok(()), "{path}");
        }
        for path in [
            "",
            "/etc/passwd",
            "../up",
            "a/../b",
            "./a",
            "a\\b",
            "SKILL.md",
            "a\nb",
        ] {
            assert!(check_path(path).is_err(), "{path:?}");
        }
    }
}

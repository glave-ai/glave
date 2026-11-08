//! git's answer to `diff --numstat --patch -z`, read as changes. Its counts come first, a
//! file to each record ending in a zero byte (`added`, `removed` and the path, between tabs,
//! `-` for each count of a binary file), and an empty record ends them; then the patch, a
//! section for each file in the same order, each beginning `diff --git`, its stretches each
//! beginning `@@`. An answer cut short loses its last section, whose lines are not whole.

use std::path::Path;

use glave_ports::computer::{ChangeKind, ChangedFile, Changes, DiffLine, FileChange, LineKind};
use glave_ports::secrets::is_secret_file;

/// The most files listed: past them, the rest go unnamed.
const MOST_FILES: usize = 1_000;

/// `printed`, git's answer, as changes; `cut` when it was cut short.
pub(super) fn changes(printed: &[u8], cut: bool) -> Changes {
    let (counted, patch, whole) = counts(printed);
    let mut shortened = cut || !whole || counted.len() > MOST_FILES;
    let mut sections = sections(patch, cut).into_iter();
    let mut in_step = true;
    let mut files = Vec::with_capacity(counted.len().min(MOST_FILES));
    for counted in counted.into_iter().take(MOST_FILES) {
        let section = sections.next().filter(|_| in_step);
        // In step while each section is of its file: a header that names another stops the
        // lines, which could be another file's.
        in_step = section
            .as_ref()
            .is_some_and(|section| section.of(&counted.path));
        let shown =
            in_step && counted.counts.is_some() && !is_secret_file(Path::new(&counted.path));
        shortened |= !shown && counted.counts.is_none_or(|counts| counts != (0, 0));
        let (added, removed) = counted.counts.unwrap_or_default();
        files.push(ChangedFile {
            changes: match section {
                Some(section) if shown => section.changes(),
                _ => Vec::new(),
            },
            path: counted.path,
            added,
            removed,
        });
    }
    Changes { files, shortened }
}

/// A file as the counts name it: its path, and the lines added and removed, unless binary.
struct Counted {
    path: String,
    counts: Option<(u64, u64)>,
}

/// The files `printed` counts, what follows them, and whether their records ended as they
/// should, with an empty one.
fn counts(printed: &[u8]) -> (Vec<Counted>, &[u8], bool) {
    let mut counted = Vec::new();
    let mut rest = printed;
    while let Some(end) = rest.iter().position(|byte| *byte == 0) {
        let record = rest.get(..end).unwrap_or_default();
        rest = rest.get(end.saturating_add(1)..).unwrap_or_default();
        if record.is_empty() {
            return (counted, rest, true);
        }
        let mut fields = record.splitn(3, |byte| *byte == b'\t');
        let (Some(added), Some(removed), Some(path)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let number = |field: &[u8]| std::str::from_utf8(field).ok()?.parse::<u64>().ok();
        counted.push(Counted {
            path: String::from_utf8_lossy(path).into_owned(),
            counts: number(added).zip(number(removed)),
        });
    }
    (counted, &[], rest.is_empty())
}

/// One file's section of the patch: its header, and the lines after it.
struct Section<'a> {
    header: &'a [u8],
    lines: Vec<&'a [u8]>,
}

/// The sections of `patch`, the last left out when it was `cut` short.
fn sections(patch: &[u8], cut: bool) -> Vec<Section<'_>> {
    let patch = patch.strip_suffix(b"\n").unwrap_or(patch);
    let mut sections: Vec<Section<'_>> = Vec::new();
    for line in patch.split(|byte| *byte == b'\n') {
        if line.starts_with(b"diff --git ") {
            sections.push(Section {
                header: line,
                lines: Vec::new(),
            });
        } else if let Some(section) = sections.last_mut() {
            section.lines.push(line);
        }
    }
    if cut {
        sections.pop();
    }
    sections
}

impl Section<'_> {
    /// Whether this is the section of the file at `path`: its header names it, or names it in
    /// quotes, as git writes a path with unusual characters.
    fn of(&self, path: &str) -> bool {
        self.header.starts_with(b"diff --git \"")
            || self.header == format!("diff --git a/{path} b/{path}").as_bytes()
    }

    /// Its changes: the whole of a new file, else each stretch of an old one.
    fn changes(&self) -> Vec<FileChange> {
        let mut new = false;
        let mut changes: Vec<FileChange> = Vec::new();
        for line in &self.lines {
            if line.starts_with(b"@@") {
                changes.push(FileChange {
                    kind: if new {
                        ChangeKind::Wrote
                    } else {
                        ChangeKind::Edited
                    },
                    lines: Vec::new(),
                });
                continue;
            }
            let Some(change) = changes.last_mut() else {
                new |= line.starts_with(b"new file mode");
                continue;
            };
            let (kind, text) = match line.split_first() {
                Some((b' ', text)) => (LineKind::Same, text),
                Some((b'-', text)) => (LineKind::Removed, text),
                Some((b'+', text)) => (LineKind::Added, text),
                // An empty line kept, as git writes one with `diff.suppressBlankEmpty`.
                None => (LineKind::Same, &[][..]),
                // `\ No newline at end of file`.
                Some(_) => continue,
            };
            let text = text.strip_suffix(b"\r").unwrap_or(text);
            change.lines.push(DiffLine {
                kind,
                text: String::from_utf8_lossy(text).into_owned(),
            });
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// git's answer for: a binary file, a deleted one, one edited, and a new one, as
    /// `diff --numstat --patch -z` prints it.
    fn answer() -> Vec<u8> {
        let counts =
            "-\t-\tbin.dat\x000\t1\tgone.txt\x002\t1\tmain.rs\x002\t0\tnew file.txt\x00\x00";
        let patch = "diff --git a/bin.dat b/bin.dat\n\
            new file mode 100644\n\
            index 0000000..88768ef\n\
            Binary files /dev/null and b/bin.dat differ\n\
            diff --git a/gone.txt b/gone.txt\n\
            deleted file mode 100644\n\
            index 587be6b..0000000\n\
            --- a/gone.txt\n\
            +++ /dev/null\n\
            @@ -1 +0,0 @@\n\
            -x\n\
            diff --git a/main.rs b/main.rs\n\
            index 5626abf..814f4a4 100644\n\
            --- a/main.rs\n\
            +++ b/main.rs\n\
            @@ -1,2 +1,2 @@ fn main() {\n\
            \x20fn main() {\n\
            -    hi();\r\n\
            +    hello();\n\
            @@ -9 +9,2 @@\n\
            \n\
            +--- not a header\n\
            \\ No newline at end of file\n\
            diff --git a/new file.txt b/new file.txt\n\
            new file mode 100644\n\
            index 0000000..07f33c4\n\
            --- /dev/null\n\
            +++ b/new file.txt\t\n\
            @@ -0,0 +1,2 @@\n\
            +new\n\
            +file\n";
        [counts.as_bytes(), patch.as_bytes()].concat()
    }

    /// Each change of `file` as its kind and its lines, each with its sign.
    fn shown(file: &ChangedFile) -> Vec<(ChangeKind, Vec<String>)> {
        file.changes
            .iter()
            .map(|change| {
                let lines = change.lines.iter().map(|line| {
                    let sign = match line.kind {
                        LineKind::Same => ' ',
                        LineKind::Removed => '-',
                        LineKind::Added => '+',
                    };
                    format!("{sign}{}", line.text)
                });
                (change.kind, lines.collect())
            })
            .collect()
    }

    #[test]
    fn each_file_has_its_counts_and_its_stretches_and_a_binary_one_none() {
        let changes = changes(&answer(), false);
        let counted: Vec<_> = changes
            .files
            .iter()
            .map(|file| (file.path.as_str(), file.added, file.removed))
            .collect();
        assert_eq!(
            counted,
            [
                ("bin.dat", 0, 0),
                ("gone.txt", 0, 1),
                ("main.rs", 2, 1),
                ("new file.txt", 2, 0)
            ]
        );
        let [binary, gone, main, new] = &changes.files[..] else {
            panic!("four files");
        };
        assert!(binary.changes.is_empty(), "named, never dumped");
        assert!(changes.shortened, "a binary file's lines are left out");
        assert_eq!(shown(gone), [(ChangeKind::Edited, vec!["-x".to_owned()])]);
        assert_eq!(
            shown(main),
            [
                (
                    ChangeKind::Edited,
                    vec![
                        " fn main() {".to_owned(),
                        "-    hi();".to_owned(),
                        "+    hello();".to_owned()
                    ]
                ),
                (
                    ChangeKind::Edited,
                    vec![" ".to_owned(), "+--- not a header".to_owned()]
                )
            ],
            "each stretch its own change; a blank kept line, a line like a header and no note \
             of a last newline"
        );
        assert_eq!(
            shown(new),
            [(
                ChangeKind::Wrote,
                vec!["+new".to_owned(), "+file".to_owned()]
            )]
        );
    }

    #[test]
    fn an_answer_cut_short_names_every_file_counted_and_shows_only_whole_ones() {
        let answer = answer();
        let at = answer
            .windows(4)
            .position(|window| window == b"+new")
            .unwrap();
        let changes = changes(&answer[..at], true);
        assert_eq!(changes.files.len(), 4);
        assert!(changes.shortened);
        assert_eq!(changes.files[2].changes.len(), 2, "whole before the cut");
        assert!(changes.files[3].changes.is_empty(), "cut in its lines");
        let in_counts = super::changes(&answer[..20], true);
        let paths: Vec<_> = in_counts.files.iter().map(|file| &file.path).collect();
        assert_eq!(paths, ["bin.dat"], "a record cut short is not named");
    }

    #[test]
    fn a_secret_s_lines_are_left_out_and_nothing_changed_is_nothing() {
        let answer = "1\t0\t.env\x00\x00diff --git a/.env b/.env\nnew file mode 100644\n\
            @@ -0,0 +1 @@\n+TOKEN=x\n";
        let changes = changes(answer.as_bytes(), false);
        assert_eq!((changes.files[0].added, changes.shortened), (1, true));
        assert!(
            changes.files[0].changes.is_empty(),
            "named, its lines kept back"
        );
        let none = super::changes(b"", false);
        assert!(none.files.is_empty() && !none.shortened);
    }

    #[test]
    fn a_section_of_another_file_stops_the_lines_where_they_would_go_wrong() {
        let answer = "1\t0\ta.txt\x001\t0\tb.txt\x00\x00diff --git a/b.txt b/b.txt\n\
            @@ -0,0 +1 @@\n+b\n";
        let changes = changes(answer.as_bytes(), false);
        assert!(changes.files.iter().all(|file| file.changes.is_empty()));
        assert!(changes.shortened);
    }
}

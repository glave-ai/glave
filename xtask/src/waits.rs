//! `cargo xtask waits`: every wait has a limit, sized to what it waits for (#710,
//! `specs/invariants.md`, 20). In the non-test sources under `crates/` and `xtask/`, a call
//! that waits for something to end or arrive (a process, a thread, a task set, a channel, an
//! async lock, a notification, a watched value, a semaphore's permit) has its limit on the
//! same line (`timeout(` in its code, not a comment or a string), or is listed in
//! [`bounded::BOUNDED`] with what bounds it instead: a limit set around it elsewhere, a stop
//! or a close that always comes, the work it guards, or a person, the limit of a wait for
//! them, who can always cancel it. Each entry says how many lines of its file it stands for,
//! so a new wait fails the check until it has a limit or a reason, and an entry that matches
//! fewer lines than it says fails it too, so the list stays true.
//!
//! What a line cannot show is not seen: a task's or a oneshot's handle awaited by name, and
//! sockets, pipes and calls across the network. Their limits are kept by the tests the
//! invariant names.

mod bounded;

use crate::sources;
use bounded::BOUNDED;

/// Where waits are looked for.
const FOLDERS: &[&str] = &["crates", "xtask"];

/// What waits, as code calls it, and what it waits for.
const WAITS: &[(&str, &str)] = &[
    (".wait()", "a process's or a thread's end"),
    (".wait_with_output()", "a process's end"),
    (".join()", "a thread's end"),
    (".join_next().await", "a task's end"),
    (".recv()", "a channel's next message"),
    (".lock().await", "an async lock"),
    (".notified()", "a notification"),
    (".changed()", "a watched value's change"),
    (".wait_for(", "a watched value's change"),
    (".acquire().await", "a semaphore's permit"),
    (".acquire_owned().await", "a semaphore's permit"),
];

/// What waits in xtask, which may run a program without glave-process: a process's end. Not
/// looked for in `crates/`, where an HTTP response's `status()` shares the name and `cargo
/// xtask spawns` already sends every process through glave-process.
const XTASK_WAITS: &[(&str, &str)] = &[
    (".status()", "a process's end"),
    (".output()", "a process's end"),
];

/// What says a wait's limit is on its line.
const LIMITED: &str = "timeout(";

/// Fails on a wait with no limit on its line that [`BOUNDED`] does not list, and on an entry
/// of it that stands for more lines than are there.
pub(crate) fn check() -> Result<(), String> {
    let read = sources::read(FOLDERS)?;
    let mut matched = vec![0_usize; BOUNDED.len()];
    let mut problems = Vec::new();
    let mut waits = 0_usize;
    for (path, text) in &read {
        for (n, found) in unbounded(path, text) {
            waits += 1;
            match listed(path, &found.line) {
                Some(entry) => {
                    if let Some(count) = matched.get_mut(entry) {
                        *count += 1;
                    }
                }
                None => problems.push(format!(
                    "{path}:{n}: waits for {} with no limit: `{}`",
                    found.waits_for, found.line
                )),
            }
        }
    }
    for ((path, line, lines, _), matched) in BOUNDED.iter().zip(&matched) {
        match matched.cmp(lines) {
            std::cmp::Ordering::Equal => {}
            std::cmp::Ordering::Less => problems.push(format!(
                "{path}: {matched} wait(s) say `{line}`, and its entry stands for {lines}; \
                 correct it, or remove it from xtask/src/waits/bounded.rs"
            )),
            std::cmp::Ordering::Greater => problems.push(format!(
                "{path}: {matched} waits say `{line}`, and its entry stands for {lines}: give \
                 the new one a limit, or count it there if the same reason bounds it"
            )),
        }
    }
    if problems.is_empty() {
        println!(
            "xtask: every wait in {} sources has a limit ({waits} bounded elsewhere, each said why)",
            read.len()
        );
        Ok(())
    } else {
        Err(format!(
            "{} wait(s) with no limit: give each one, sized to what it waits for, on its line \
             (tokio::time::timeout), or list it in xtask/src/waits/bounded.rs with what bounds \
             it (specs/invariants.md, 20):\n  {}",
            problems.len(),
            problems.join("\n  ")
        ))
    }
}

/// A wait found on one line.
struct Found {
    waits_for: &'static str,
    line: String,
}

/// Each wait in `text`, a source at `path`, with no limit on its line, by line number. What
/// a string or a comment says is not code.
fn unbounded(path: &str, text: &str) -> Vec<(usize, Found)> {
    let xtask = path.starts_with("xtask/");
    sources::code(text)
        .into_iter()
        .filter_map(|(n, code)| {
            let called = unquoted(code);
            if called.contains(LIMITED) {
                return None;
            }
            let waits_for = WAITS
                .iter()
                .chain(XTASK_WAITS.iter().filter(|_| xtask))
                .find(|(call, _)| called.contains(call))
                .map(|(_, waits_for)| *waits_for)?;
            Some((
                n,
                Found {
                    waits_for,
                    line: code.trim_end().to_owned(),
                },
            ))
        })
        .collect()
}

/// `code` with what its string literals say left out, their quotes kept, and without a comment
/// at its end.
fn unquoted(code: &str) -> String {
    let mut kept = String::with_capacity(code.len());
    let (mut quoted, mut escaped) = (false, false);
    let mut chars = code.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, escaped, c) {
            (true, false, '\\') => escaped = true,
            (true, true, _) => escaped = false,
            // A quote as a char, `'"'`, which opens no string.
            (false, _, '\'') if chars.peek() == Some(&'"') => {
                kept.push(c);
                chars.next();
            }
            (_, _, '"') => {
                quoted = !quoted;
                kept.push(c);
            }
            (false, _, '/') if chars.peek() == Some(&'/') => break,
            (false, _, c) => kept.push(c),
            (true, false, _) => {}
        }
    }
    kept
}

/// The entry of [`BOUNDED`] that lists `line` of `path`, if one does.
fn listed(path: &str, line: &str) -> Option<usize> {
    BOUNDED
        .iter()
        .position(|(file, said, _, _)| *file == path && line.contains(said))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wait_with_no_limit_on_its_line_is_found() {
        let text = "async fn go() {\n    let status = child.wait().await?;\n    \
            let held = lock.lock().await;\n    let _ = rx.recv().await;\n    \
            let _ = done.wait_for(|d| *d).await;\n    while set.join_next().await.is_some() {}\n}\n";
        let found: Vec<usize> = unbounded("crates/a/src/b.rs", text)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(found, [2, 3, 4, 5, 6]);
    }

    #[test]
    fn a_limit_on_the_line_strings_comments_and_tests_pass() {
        let text = "let _ = tokio::time::timeout(LIMIT, child.wait()).await;\n\
            // child.wait() is not called here\nlet said = \"rx.recv() \\\" child.wait()\";\n\
            #[cfg(test)]\nmod tests {\n    fn t() { child.wait(); }\n}\n";
        assert!(unbounded("crates/a/src/b.rs", text).is_empty());
        assert_eq!(unquoted(r#"a("b.wait()", c.recv())"#), r#"a("", c.recv())"#);
        assert!(unbounded("crates/a/src/b.rs", "let x = words.join(\" \");\n").is_empty());
    }

    #[test]
    fn a_limit_only_a_comment_or_a_string_names_is_none() {
        let commented = "let _ = rx.recv().await; // no timeout( here\n";
        assert_eq!(unbounded("crates/a/src/b.rs", commented).len(), 1);
        let quoted = "let _ = rx.recv().await.context(\"timeout(\");\n";
        assert_eq!(unbounded("crates/a/src/b.rs", quoted).len(), 1);
        assert_eq!(unquoted("a.wait(); // b // c"), "a.wait(); ");
        let after_a_quote_char = "if c == '\"' { let _ = rx.recv().await; }\n";
        assert_eq!(unbounded("crates/a/src/b.rs", after_a_quote_char).len(), 1);
        assert_eq!(
            unquoted(r#"x("http://a") // c"#),
            r#"x("")"#.to_owned() + " "
        );
    }

    #[test]
    fn a_process_waited_for_in_xtask_is_found_and_an_http_status_elsewhere_is_not() {
        let text = "let ran = Command::new(\"ls\").status();\nlet code = response.status();\n";
        assert_eq!(unbounded("xtask/src/a.rs", text).len(), 2);
        assert!(unbounded("crates/a/src/b.rs", text).is_empty());
    }

    #[test]
    fn every_entry_names_a_source_counts_its_lines_and_says_why() {
        for (path, line, lines, why) in BOUNDED {
            assert!(
                sources::non_test(path),
                "{path} is not a source the check reads"
            );
            assert!(!line.trim().is_empty(), "{path}: an entry with no line");
            assert!(*lines > 0, "{path}: `{line}` stands for no line");
            assert!(why.len() > 20, "{path}: `{line}` needs its reason");
        }
    }
}

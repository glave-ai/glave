use super::*;

/// A function `name` of ten lines: six lines of code, with enough tokens to count, between its
/// first line and `tail`.
fn body(name: &str, tail: &str) -> String {
    format!(
        "fn {name}() {{\n    let first = compute_first(alpha, beta, gamma, delta);\n    \
         let second = compute_second(first, alpha, beta, gamma);\n    \
         let third = combine(first, second, alpha, beta);\n    \
         if third.is_ready(alpha, beta) {{\n        send_it(third, delta, gamma, beta);\n    }}\n    \
         record(first, second, third, alpha);\n    {tail}\n}}\n"
    )
}

fn sources(texts: &[(&str, String)]) -> Vec<(String, String)> {
    texts
        .iter()
        .map(|(path, text)| ((*path).to_owned(), text.clone()))
        .collect()
}

#[test]
fn a_copy_in_two_files_is_found_with_both_places() {
    let found = clones::find(
        &sources(&[
            ("a.rs", body("one", "done(1);")),
            (
                "b.rs",
                format!("// a comment\n\n{}", body("two", "done(2);")),
            ),
        ]),
        WINDOW,
        LEAST_TOKENS,
    );
    assert_eq!(found.len(), 1, "{found:?}");
    let copy = &found[0];
    assert_eq!(copy.lines, 6);
    assert_eq!(
        copy.places,
        [("a.rs".to_owned(), 2), ("b.rs".to_owned(), 4)]
    );
    assert!(copy.entry().ends_with(" 6 a.rs b.rs"), "{}", copy.entry());
    assert_eq!(copy.places(), "a.rs:2, b.rs:4");
}

#[test]
fn a_copy_runs_as_far_as_both_places_go_on_alike() {
    let found = clones::find(
        &sources(&[
            ("a.rs", body("one", "done(1);")),
            ("b.rs", body("two", "done(1);")),
            ("c.rs", body("three", "done(1);")),
        ]),
        WINDOW,
        LEAST_TOKENS,
    );
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].lines, 7);
    assert_eq!(found[0].places.len(), 3);
}

#[test]
fn small_copies_imports_comments_and_brackets_are_not_copies() {
    let imports = "use std::collections::{\n    BTreeMap,\n    BTreeSet,\n    HashMap,\n    \
                   HashSet,\n    VecDeque,\n    BinaryHeap,\n};\n";
    let small = "let a = 1;\nlet b = 2;\nlet c = 3;\nlet d = 4;\nlet e = 5;\nlet f = 6;\n";
    let found = clones::find(
        &sources(&[
            ("a.rs", format!("{imports}{small}")),
            ("b.rs", format!("{imports}{small}")),
        ]),
        WINDOW,
        LEAST_TOKENS,
    );
    assert_eq!(found, []);
}

#[test]
fn a_copy_in_one_file_does_not_overlap_itself() {
    let twice = format!("{}{}", body("one", "done(1);"), body("two", "done(2);"));
    let found = clones::find(&sources(&[("a.rs", twice)]), WINDOW, LEAST_TOKENS);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].places,
        [("a.rs".to_owned(), 2), ("a.rs".to_owned(), 12)]
    );
}

#[test]
fn the_fingerprint_is_the_texts_alone() {
    let find = |a: &str, b: &str| {
        clones::find(
            &sources(&[("x.rs", a.to_owned()), ("y.rs", b.to_owned())]),
            WINDOW,
            LEAST_TOKENS,
        )
    };
    let here = find(&body("one", "done(1);"), &body("two", "done(2);"));
    let moved = find(
        &format!("\n\n    {}", body("three", "x();")),
        &format!("// moved\n{}", body("four", "y();")),
    );
    assert_eq!(here[0].fingerprint, moved[0].fingerprint);
}

#[test]
fn a_copy_not_listed_fails_and_so_does_an_entry_no_longer_found() {
    let found = clones::find(
        &sources(&[
            ("a.rs", body("one", "done(1);")),
            ("b.rs", body("two", "done(2);")),
        ]),
        WINDOW,
        LEAST_TOKENS,
    );
    let entry = found[0].entry();
    let listed = format!("# what was there\n\n{entry}\n");
    assert_eq!(against(&found, &listed), Vec::<String>::new());

    let new = against(&found, "# nothing yet\n");
    assert_eq!(new.len(), 1, "{new:?}");
    assert!(
        new[0].starts_with("a.rs:2, b.rs:2: 6 lines of code copied"),
        "{new:?}"
    );
    assert!(new[0].contains(&format!("`{entry}`")), "{new:?}");

    let gone = against(&[], &listed);
    assert_eq!(gone.len(), 1, "{gone:?}");
    assert!(gone[0].contains("is no longer found"), "{gone:?}");

    let third = clones::find(
        &sources(&[
            ("a.rs", body("one", "done(1);")),
            ("b.rs", body("two", "done(2);")),
            ("c.rs", body("three", "done(3);")),
        ]),
        WINDOW,
        LEAST_TOKENS,
    );
    assert_eq!(
        against(&third, &listed).len(),
        2,
        "a third copy is a new one"
    );
}

#[test]
fn only_rust_and_the_webs_typescript_are_read_and_never_generated_code() {
    assert!(scanned("crates/ports/src/lib.rs"));
    assert!(scanned("xtask/src/main.rs"));
    assert!(scanned("web/app/src/main.ts"));
    assert!(!scanned("web/client/src/generated/api.ts"));
    assert!(!scanned("web/app/src/index.html"));
    assert!(!scanned("crates/cli/Cargo.toml"));
}

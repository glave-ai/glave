use settings::{load, Settings};

#[test]
fn an_empty_file_gives_the_defaults() {
    assert_eq!(load(""), Settings::default());
}

#[test]
fn keys_ignore_case() {
    assert_eq!(load("Timeout = 30").timeout_secs, 30);
    assert_eq!(load("RETRIES=7").retries, 7);
}

#[test]
fn comments_and_blank_lines_are_skipped() {
    let settings = load("# verbose = yes\n\nverbose = on\n");
    assert!(settings.verbose);
}

#[test]
fn bad_values_keep_the_default() {
    assert_eq!(load("timeout = soon").timeout_secs, 10);
}

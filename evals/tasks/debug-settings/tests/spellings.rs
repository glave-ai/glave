use settings::load;

#[test]
fn the_reported_file_works() {
    let settings = load("# app settings\nTimeout = 30\nmax_retries = 5\n");
    assert_eq!((settings.timeout_secs, settings.retries), (30, 5));
}

#[test]
fn every_spelling_of_two_word_keys() {
    for text in ["max_retries = 6", "max-retries = 6", "Max_Retries = 6", "MAX-RETRIES=6"] {
        assert_eq!(load(text).retries, 6, "{text:?}");
    }
    for text in ["timeout_secs = 4", "Timeout-Secs = 4", "TIMEOUT_SECS=4"] {
        assert_eq!(load(text).timeout_secs, 4, "{text:?}");
    }
}

#[test]
fn unknown_keys_change_nothing() {
    assert_eq!(load("retry = 9").retries, 3);
}

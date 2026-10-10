use textkit::{acronym, caesar, palindrome, runs, vowels};

#[test]
fn caesar_wraps_and_keeps_case() {
    assert_eq!(caesar::shift("xyz XYZ", 3), "abc ABC");
    assert_eq!(caesar::shift("Hello, World", 26), "Hello, World");
    assert_eq!(caesar::shift("é 1", 5), "é 1");
}

#[test]
fn vowels_in_either_case() {
    assert_eq!(vowels::count_vowels(""), 0);
    assert_eq!(vowels::count_vowels("AEIOU aeiou y"), 10);
}

#[test]
fn palindromes() {
    assert!(palindrome::is_palindrome(""));
    assert!(palindrome::is_palindrome("No 'x' in Nixon"));
    assert!(palindrome::is_palindrome("12321"));
    assert!(!palindrome::is_palindrome("123"));
}

#[test]
fn acronyms() {
    assert_eq!(acronym::acronym("  as  soon as possible "), "ASAP");
    assert_eq!(acronym::acronym("complementary metal-oxide semiconductor"), "CMOS");
    assert_eq!(acronym::acronym(""), "");
}

#[test]
fn runs() {
    assert_eq!(runs::encode(""), "");
    assert_eq!(runs::encode("abc"), "abc");
    assert_eq!(runs::encode("zzzzzzzzzzzz"), "12z");
}

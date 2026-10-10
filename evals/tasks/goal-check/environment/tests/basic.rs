use textkit::{acronym, caesar, palindrome, runs, vowels};

#[test]
fn caesar_shifts_letters() {
    assert_eq!(caesar::shift("Zebra!", 1), "Afcsb!");
}

#[test]
fn vowels_are_counted() {
    assert_eq!(vowels::count_vowels("Education"), 5);
}

#[test]
fn palindromes_ignore_punctuation() {
    assert!(palindrome::is_palindrome("A man, a plan, a canal: Panama"));
    assert!(!palindrome::is_palindrome("glave"));
}

#[test]
fn acronyms_split_on_hyphens() {
    assert_eq!(acronym::acronym("portable network-graphics"), "PNG");
}

#[test]
fn runs_are_encoded() {
    assert_eq!(runs::encode("aaabcc"), "3ab2c");
}

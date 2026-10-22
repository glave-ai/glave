//! The check every merged text passes before tidy-up writes it (`specs/memory.md`, "Tidy-up"):
//! it joins its sources without inventing a fact or losing one.

use std::collections::BTreeSet;

use crate::similar::words;

/// Words a merged text may add that no source has: joining words, never facts.
const MOST_NEW_WORDS: usize = 2;
/// The share of each source's words, in percent, a merged text must keep.
const KEEP_PERCENT: usize = 50;
/// Memories one merge may take in.
const MOST_MERGED: usize = 8;
/// What can close a sentence, after any closing quote or bracket.
const SENTENCE_ENDS: [char; 5] = ['.', '!', '?', ':', ';'];
/// Quotes and brackets that can follow a sentence's last mark.
const CLOSERS: [char; 6] = ['"', '\'', ')', ']', '\u{201d}', '\u{2019}'];

/// Whether `text` merges `sources` without inventing or losing a fact: it adds at most a couple
/// of joining words (none with a digit), keeps at least half of each source's words, and keeps
/// every one of its names, numbers and identifiers ([`facts`]).
pub(super) fn keeps_facts(text: &str, sources: &[String]) -> Result<(), String> {
    if sources.len() > MOST_MERGED {
        return Err(format!("a merge takes at most {MOST_MERGED} memories"));
    }
    let merged = words(text);
    let all: BTreeSet<String> = sources.iter().flat_map(|source| words(source)).collect();
    let new: Vec<&String> = merged.difference(&all).collect();
    if new.len() > MOST_NEW_WORDS {
        return Err(format!("the merged text adds words no memory has: {new:?}"));
    }
    let kept_numbers = numbers(text);
    let known: BTreeSet<String> = sources.iter().flat_map(|source| numbers(source)).collect();
    if let Some(number) = kept_numbers.difference(&known).next() {
        return Err(format!(
            "the merged text adds {number:?}, which no memory has"
        ));
    }
    let kept_facts = singular(&merged);
    for source in sources {
        let theirs = words(source);
        let kept = theirs.intersection(&merged).count();
        if kept.saturating_mul(100) < theirs.len().saturating_mul(KEEP_PERCENT) {
            return Err(format!("the merged text loses most of {source:?}"));
        }
        let lost: Vec<String> = singular(&facts(source))
            .difference(&kept_facts)
            .cloned()
            .chain(numbers(source).difference(&kept_numbers).cloned())
            .collect();
        if !lost.is_empty() {
            return Err(format!("the merged text loses {lost:?} from {source:?}"));
        }
    }
    Ok(())
}

/// The words of `text` that carry its facts, normalised as [`words`] does: names (a capital
/// letter anywhere but at the start of a sentence), and every word of a token that holds a
/// digit or reads as a channel, path, file, address or type (`#`, `/`, `_`, `@` or `+` in it,
/// or a `.` followed by more).
fn facts(text: &str) -> BTreeSet<String> {
    let mut found = String::new();
    // The next word starts a sentence.
    let mut opens = true;
    for token in text.split_whitespace() {
        let identifier = token.contains(|c: char| c.is_ascii_digit() || "#/_@+".contains(c))
            || token
                .split('.')
                .skip(1)
                .any(|after| after.starts_with(char::is_alphanumeric));
        let mut first = opens;
        for word in token.split(|c: char| !c.is_alphanumeric()) {
            if word.is_empty() {
                continue;
            }
            let named = word
                .chars()
                .enumerate()
                .any(|(at, c)| c.is_uppercase() && (at > 0 || !first));
            if identifier || named {
                found.push_str(word);
                found.push(' ');
            }
            first = false;
            opens = false;
        }
        if token.trim_end_matches(CLOSERS).ends_with(SENTENCE_ENDS) {
            opens = true;
        }
    }
    words(&found)
}

/// `words` with any plural's `s` dropped, however short the word: names such as "PRs" and
/// "APIs" are the same facts as "PR" and "API".
fn singular(words: &BTreeSet<String>) -> BTreeSet<String> {
    words
        .iter()
        .map(|word| word.strip_suffix('s').unwrap_or(word).to_owned())
        .collect()
}

/// Every run of letters and digits in `text` that holds a digit: numbers, versions, names.
fn numbers(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().any(|c| c.is_ascii_digit()))
        .map(str::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_merge_may_join_but_never_invent_or_lose() {
        let sources = [
            "Use tabs, not spaces, in this project's Python".to_owned(),
            "This project's Python code uses tabs for indentation".to_owned(),
        ];
        assert_eq!(
            keeps_facts(
                "Use tabs, not spaces, for indentation in this project's Python code",
                &sources
            ),
            Ok(())
        );
        assert!(keeps_facts("Use tabs in Python and deploy to Render", &sources).is_err());
        assert!(
            keeps_facts(
                "Use tabs, not spaces, width 8, in this project's Python code",
                &sources
            )
            .is_err(),
            "adds a number"
        );
        assert!(
            keeps_facts("Python", &sources).is_err(),
            "loses nearly everything"
        );
    }

    /// DeepSeek's third memory-month attempt (#555): a merge kept most of "deploys to Railway"
    /// but not where.
    #[test]
    fn a_merge_keeps_every_name_number_and_identifier() {
        let sources = [
            "Atlas deploys to Railway; merging to main ships it".to_owned(),
            "Atlas config lives in config/atlas.toml, and on-call is posted in #ops-tern"
                .to_owned(),
        ];
        assert_eq!(
            keeps_facts(
                "Atlas deploys to Railway when merging to main; config lives in \
                 config/atlas.toml, on-call in #ops-tern",
                &sources
            ),
            Ok(())
        );
        let lost = keeps_facts(
            "Atlas ships on merging to main; config lives in config/atlas.toml, on-call in \
             #ops-tern",
            &sources,
        );
        assert!(
            lost.as_ref().is_err_and(|why| why.contains("railway")),
            "{lost:?}"
        );
        assert!(
            keeps_facts(
                "Atlas deploys to Railway when merging to main; config lives in its own \
                 folder, on-call in #ops-tern",
                &sources
            )
            .is_err(),
            "loses a path"
        );
        let numbered = ["Releases go out every 2nd Tuesday".to_owned()];
        assert!(
            keeps_facts("Releases go out every Tuesday", &numbered).is_err(),
            "loses a number"
        );
    }

    #[test]
    fn names_are_words_capitalised_inside_a_sentence_plurals_or_not() {
        assert_eq!(
            facts("Keep answers short. Use Helix: \"Releases\" on Thursday.) Store dates"),
            ["helix", "thursday"].map(String::from).into(),
            "a sentence's first word is no name, after a closing quote or bracket too"
        );
        let reviews = [
            "PRs need a linked issue".to_owned(),
            "Each PR needs a linked issue and a review".to_owned(),
        ];
        assert_eq!(
            keeps_facts("PRs need a linked issue and a review", &reviews),
            Ok(())
        );
        assert!(
            facts("Errors are problem+json").contains("problem"),
            "a type is an identifier"
        );
    }
}

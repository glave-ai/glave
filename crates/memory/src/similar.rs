//! The cheap near-duplicate check `remember` runs before saving (`specs/memory.md`, "Duplicate
//! check"): words normalised, overlap scored. Full text only; semantic search comes later.

use std::collections::BTreeSet;

/// Words too common to tell two memories apart.
const STOP: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "do", "for", "from", "has", "have", "in",
    "is", "it", "its", "of", "on", "or", "our", "so", "that", "the", "their", "this", "to", "uses",
    "use", "was", "we", "with", "you", "your",
];

/// How much two memories' words must overlap to count as near duplicates: the Dice coefficient
/// of their word sets.
const NEAR: f64 = 0.6;

/// The normalised words of `text`: lowercase, letters and digits only, a plural's `s` dropped,
/// single letters and common words left out.
pub(crate) fn words(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|word| word.chars().count() > 1 && !STOP.contains(&word.as_str()))
        .map(|word| match word.strip_suffix('s') {
            Some(stem) if stem.chars().count() > 3 => stem.to_owned(),
            _ => word,
        })
        .collect()
}

/// How alike two texts are, from 0 (no word shared) to 1 (the same words).
pub(crate) fn overlap(one: &str, other: &str) -> f64 {
    let (one, other) = (words(one), words(other));
    let total = one.len().saturating_add(other.len());
    if total == 0 {
        return 0.0;
    }
    let shared = one.intersection(&other).count();
    let (shared, total) = (
        f64::from(u32::try_from(shared).unwrap_or(u32::MAX)),
        f64::from(u32::try_from(total).unwrap_or(u32::MAX)),
    );
    2.0 * shared / total
}

/// Whether `text` nearly repeats `existing`.
pub(crate) fn near(text: &str, existing: &str) -> bool {
    overlap(text, existing) >= NEAR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewordings_are_near_and_different_facts_are_not() {
        assert!(near(
            "Use tabs, not spaces, in this project's Python",
            "This project's Python uses tabs instead of spaces"
        ));
        assert!(near(
            "Tests need `make db` first",
            "the tests need make db first"
        ));
        assert!(!near("We deploy with Fly", "Ada prefers short answers"));
        assert!(!near("We deploy with Fly", "We moved deploys to Render"));
        assert_eq!(overlap("", ""), 0.0);
    }

    proptest::proptest! {
        #[test]
        fn overlap_is_symmetric_and_bounded(a in ".{0,80}", b in ".{0,80}") {
            let ab = overlap(&a, &b);
            proptest::prop_assert!((0.0..=1.0).contains(&ab));
            proptest::prop_assert!((ab - overlap(&b, &a)).abs() < 1e-9);
        }
    }
}

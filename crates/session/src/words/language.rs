//! The languages the server writes its words in, and which one a tag or a client asks for: by
//! the whole tag, else by its language alone (`en-GB` reads English), as the web app chooses.
//! Besides English there are only the pseudo-locales today, had by their whole tag alone, as a
//! browser set to one asks for it; a real language joins once a speaker has read it.

/// A language the server writes its words in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Language {
    English,
    /// `en-XA`: English accented and longer, to show a word that skips the catalog.
    Accented,
    /// `ar-XB`: English written right to left.
    RightToLeft,
}

/// Each language by its tag.
const TAGS: [(&str, Language); 3] = [
    ("en", Language::English),
    ("en-XA", Language::Accented),
    ("ar-XB", Language::RightToLeft),
];

impl Language {
    /// The language `tag` (BCP 47, `en-GB`) asks for, when the server writes in it: by the
    /// whole tag, else by its language alone. `None` for one it does not, or a tag that is not
    /// one.
    pub(crate) fn of(tag: &str) -> Option<Self> {
        let tag = tag.trim();
        if !well_formed(tag) {
            return None;
        }
        let whole = TAGS.iter().find(|(each, _)| each.eq_ignore_ascii_case(tag));
        let alone = || {
            let (language, _) = tag.split_once('-')?;
            TAGS.iter()
                .find(|(each, _)| !each.contains('-') && each.eq_ignore_ascii_case(language))
        };
        whole.or_else(alone).map(|(_, language)| *language)
    }

    /// The first language of an `Accept-Language` header the server writes in, by the weight
    /// each is given (`q`), in the header's order where they weigh the same. A language given
    /// no weight (`q=0`), `*` and what is not a tag are passed over.
    pub(crate) fn accepted(header: &str) -> Option<Self> {
        let mut asked: Vec<(u16, &str)> = header
            .split(',')
            .filter_map(|item| {
                let mut parts = item.split(';');
                let tag = parts.next()?.trim();
                let weight = match parts.find_map(|param| param.trim().strip_prefix("q=")) {
                    Some(weight) => thousandths(weight.trim())?,
                    None => 1000,
                };
                (weight > 0).then_some((weight, tag))
            })
            .collect();
        // Stable: the header's order among those that weigh the same.
        asked.sort_by(|a, b| b.0.cmp(&a.0));
        asked.into_iter().find_map(|(_, tag)| Self::of(tag))
    }
}

/// Whether `tag` is shaped as a language tag: subtags of 1 to 8 letters and digits between
/// hyphens, the first letters alone.
fn well_formed(tag: &str) -> bool {
    let mut subtags = tag.split('-');
    let first = subtags.next().unwrap_or_default();
    let fits = |subtag: &str| (1..=8).contains(&subtag.len());
    fits(first)
        && first.bytes().all(|b| b.is_ascii_alphabetic())
        && subtags.all(|subtag| fits(subtag) && subtag.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// A weight (`0.8`) in thousandths, when it is one: 0 to 1, with at most three decimals.
fn thousandths(weight: &str) -> Option<u16> {
    let (whole, fraction) = weight.split_once('.').unwrap_or((weight, ""));
    if fraction.len() > 3 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let whole: u16 = match whole {
        "0" => 0,
        "1" => 1000,
        _ => return None,
    };
    let fraction: u16 = format!("{fraction:0<3}").parse().ok()?;
    let weight = whole + fraction;
    (weight <= 1000).then_some(weight)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn a_tag_asks_for_its_whole_language_or_its_language_alone() {
        assert_eq!(Language::of("en"), Some(Language::English));
        assert_eq!(Language::of("en-GB"), Some(Language::English));
        assert_eq!(Language::of("EN-us"), Some(Language::English));
        assert_eq!(Language::of("en-XA"), Some(Language::Accented));
        assert_eq!(Language::of("en-xa"), Some(Language::Accented));
        assert_eq!(Language::of("ar-XB"), Some(Language::RightToLeft));
        // Arabic itself is not one the server writes in yet; its pseudo-locale is had only by
        // its whole tag.
        assert_eq!(Language::of("ar"), None);
        assert_eq!(Language::of("ar-EG"), None);
        assert_eq!(Language::of("de-DE"), None);
        for broken in ["", "-", "en-", "e n", "en_GB", "1en", "en-toolongsubtag"] {
            assert_eq!(Language::of(broken), None, "{broken:?}");
        }
    }

    #[test]
    fn a_client_gets_the_first_language_it_accepts_that_the_server_has() {
        assert_eq!(Language::accepted("en-XA"), Some(Language::Accented));
        assert_eq!(
            Language::accepted("de-DE, de;q=0.9, en;q=0.8"),
            Some(Language::English)
        );
        assert_eq!(
            Language::accepted("en;q=0.5, ar-XB;q=0.9"),
            Some(Language::RightToLeft)
        );
        // The same weight keeps the header's order.
        assert_eq!(
            Language::accepted("ar-XB;q=0.7, en-XA;q=0.7"),
            Some(Language::RightToLeft)
        );
        assert_eq!(
            Language::accepted("en-XA;q=0, en;q=0.1"),
            Some(Language::English)
        );
        assert_eq!(Language::accepted("*, fr"), None);
        assert_eq!(Language::accepted("en;q=2, en-XA;q=abc"), None);
        assert_eq!(Language::accepted(""), None);
    }

    proptest! {
        #[test]
        fn any_header_is_read_without_failing(header in ".{0,200}") {
            let _ = Language::accepted(&header);
        }

        #[test]
        fn a_header_naming_one_of_its_languages_alone_gets_it(
            at in 0usize..3,
            weight in 1u16..=1000,
        ) {
            let (tag, language) = TAGS[at];
            let header = format!("{tag};q={}.{:03}", weight / 1000, weight % 1000);
            prop_assert_eq!(Language::accepted(&header), Some(language));
        }
    }
}

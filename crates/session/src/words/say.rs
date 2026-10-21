//! A sentence written in a language, through Fluent. English is each sentence's own pattern;
//! the pseudo-locales are made from it as it is written, never by hand, as the web app's are
//! (`web/tools/i18n.mjs`): `en-XA` with every letter accented, a third longer and bracketed,
//! so a word that skips the catalog or a cut shows at once, and `ar-XB` with each word between
//! the marks that write it right to left. The values a sentence holds stay as they are.

use std::borrow::Cow;

use fluent_bundle::{FluentBundle, FluentResource};

use super::{Language, Sentence};

/// Each letter and its accented look-alike, for `en-XA`.
const PLAIN: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
const ACCENTED: &str = "åƀçðéƒĝĥîĵķļɱñöþǫŕšţûṽŵẋýžÅƁÇÐÉƑĜĤÎĴĶĻṀÑÖÞǪŔŠŢÛṼŴẊÝŽ";
/// What makes `en-XA` longer, a word at a time.
const NUMBERS: [&str; 10] = [
    "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
];

/// `sentence` written in `language`: its message alone, read and written by Fluent, as plain
/// text with no marks around the values it holds. A pattern Fluent cannot read writes its id,
/// which the catalog's test rules out.
pub(crate) fn say(language: Language, sentence: &Sentence) -> String {
    let source = format!("{} = {}\n", sentence.id(), sentence.english());
    let resource = FluentResource::try_new(source).unwrap_or_else(|(read, _)| read);
    let mut bundle = FluentBundle::new(Vec::new());
    bundle.set_use_isolating(false);
    bundle.set_transform(match language {
        Language::English => None,
        Language::Accented => Some(accented),
        Language::RightToLeft => Some(right_to_left),
    });
    // The message is the bundle's only one: nothing it could clash with.
    let _ = bundle.add_resource(resource);
    let Some(pattern) = bundle
        .get_message(sentence.id())
        .and_then(|message| message.value())
    else {
        return sentence.id().to_owned();
    };
    let args = sentence.args();
    let mut errors = Vec::new();
    let said = bundle.format_pattern(pattern, Some(&args), &mut errors);
    match language {
        Language::Accented => longer(&said),
        Language::English | Language::RightToLeft => said.into_owned(),
    }
}

/// `text` with every letter accented, for `en-XA`.
fn accented(text: &str) -> Cow<'_, str> {
    let plain: Vec<char> = PLAIN.chars().collect();
    let accented: Vec<char> = ACCENTED.chars().collect();
    text.chars()
        .map(|each| {
            plain
                .iter()
                .position(|letter| *letter == each)
                .and_then(|at| accented.get(at).copied())
                .unwrap_or(each)
        })
        .collect::<String>()
        .into()
}

/// `[Šéţţîñĝš one two]`: `text` a third longer and bracketed, to show where it is cut.
fn longer(text: &str) -> String {
    let letters = text.chars().filter(|each| !each.is_whitespace()).count();
    let mut padding: Vec<&str> = Vec::new();
    let mut added = 0;
    for word in NUMBERS.iter().cycle() {
        if !padding.is_empty() && added * 10 >= letters * 3 {
            break;
        }
        padding.push(word);
        added += word.len() + 1;
    }
    format!("[{text} {}]", padding.join(" "))
}

/// `text` with each word between the marks that write it right to left, for `ar-XB`, as
/// Android's is.
fn right_to_left(text: &str) -> Cow<'_, str> {
    let mut written = String::with_capacity(text.len() * 2);
    let mut word = String::new();
    for each in text.chars() {
        if each.is_whitespace() {
            flush(&mut written, &mut word);
            written.push(each);
        } else {
            word.push(each);
        }
    }
    flush(&mut written, &mut word);
    written.into()
}

/// Moves `word`, if there is one, onto `written` between the marks that write it right to
/// left.
fn flush(written: &mut String, word: &mut String) {
    if !word.is_empty() {
        written.push_str("\u{200f}\u{202e}");
        written.push_str(word);
        written.push_str("\u{202c}\u{200f}");
        word.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_is_each_sentences_own_words_with_its_values() {
        let said = say(
            Language::English,
            &Sentence::NoThread {
                thread: "t1".into(),
            },
        );
        assert_eq!(said, "no thread t1 in this project");
    }

    #[test]
    fn en_xa_is_accented_longer_and_bracketed_its_values_left_as_they_are() {
        let said = say(
            Language::Accented,
            &Sentence::NoThread {
                thread: "t1".into(),
            },
        );
        assert_eq!(said, "[ñö ţĥŕéåð t1 îñ ţĥîš þŕöĵéçţ one two]");
    }

    #[test]
    fn ar_xb_writes_each_word_right_to_left() {
        let said = say(
            Language::RightToLeft,
            &Sentence::NoThread {
                thread: "t1".into(),
            },
        );
        let word = |word: &str| format!("\u{200f}\u{202e}{word}\u{202c}\u{200f}");
        assert_eq!(
            said,
            format!(
                "{} {} t1 {} {} {}",
                word("no"),
                word("thread"),
                word("in"),
                word("this"),
                word("project")
            )
        );
    }
}

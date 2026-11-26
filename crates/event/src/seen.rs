//! Which letters a person sees: what a client shows of outside text leaves out, or marks, the
//! rest, so nothing written can rewrite a terminal or hide letters from the eye (bidirectional
//! overrides, zero-width marks, variation selectors, fillers that draw as nothing).

/// Whether `letter` is one a person sees: not a control or format character.
pub fn seen(letter: char) -> bool {
    !letter.is_control()
        && !matches!(
            letter,
            '\u{ad}'
                | '\u{34f}'
                | '\u{600}'..='\u{605}'
                | '\u{61c}'
                | '\u{6dd}'
                | '\u{70f}'
                | '\u{890}'..='\u{891}'
                | '\u{8e2}'
                | '\u{115f}'..='\u{1160}'
                | '\u{180b}'..='\u{180f}'
                | '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206f}'
                | '\u{3164}'
                | '\u{fe00}'..='\u{fe0f}'
                | '\u{feff}'
                | '\u{ffa0}'
                | '\u{fff9}'..='\u{fffb}'
                | '\u{1d173}'..='\u{1d17a}'
                | '\u{e0001}'
                | '\u{e0020}'..='\u{e007f}'
                | '\u{e0100}'..='\u{e01ef}'
        )
}

/// `text` on one line as a person sees it: every space, tab and line end one space, runs of
/// them one, none at either end, and only the letters [`seen`] keeps.
pub fn one_line(text: &str) -> String {
    let shown: String = text
        .chars()
        .filter(|letter| letter.is_whitespace() || seen(*letter))
        .collect();
    shown.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::{one_line, seen};

    #[test]
    fn one_line_keeps_only_what_is_seen_with_single_spaces() {
        assert_eq!(one_line("a\u{1b}[2Kb\nc\u{200b}d\u{202e}e"), "a[2Kb cde");
        assert_eq!(one_line("  two\r\n\tlines\u{2028}here "), "two lines here");
        assert_eq!(one_line("a\u{b}b\u{c}c\u{85}d"), "a b c d");
    }

    #[test]
    fn marks_that_change_letters_or_draw_as_nothing_are_not_seen() {
        let unseen = [
            '\u{200b}',
            '\u{202e}',
            '\u{34f}',
            '\u{115f}',
            '\u{1160}',
            '\u{180b}',
            '\u{180f}',
            '\u{3164}',
            '\u{fe00}',
            '\u{fe0f}',
            '\u{ffa0}',
            '\u{e0100}',
            '\u{e01ef}',
        ];
        for letter in unseen {
            assert!(!seen(letter), "{}", letter.escape_unicode());
        }
        for letter in [
            'a',
            '\u{e9}',
            '\u{4e2d}',
            '\u{1f600}',
            ' ',
            '\u{1100}',
            '\u{fe10}',
        ] {
            assert!(seen(letter), "{}", letter.escape_unicode());
        }
    }
}

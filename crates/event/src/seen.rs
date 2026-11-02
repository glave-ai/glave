//! Which letters a person sees: what a client shows of outside text leaves out, or marks, the
//! rest, so nothing written can rewrite a terminal or hide letters from the eye (bidirectional
//! overrides, zero-width marks).

/// Whether `letter` is one a person sees: not a control or format character.
pub fn seen(letter: char) -> bool {
    !letter.is_control()
        && !matches!(
            letter,
            '\u{ad}'
                | '\u{600}'..='\u{605}'
                | '\u{61c}'
                | '\u{6dd}'
                | '\u{70f}'
                | '\u{890}'..='\u{891}'
                | '\u{8e2}'
                | '\u{180e}'
                | '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206f}'
                | '\u{feff}'
                | '\u{fff9}'..='\u{fffb}'
                | '\u{1d173}'..='\u{1d17a}'
                | '\u{e0001}'
                | '\u{e0020}'..='\u{e007f}'
        )
}

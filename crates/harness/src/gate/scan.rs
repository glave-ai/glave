//! Finding what does not belong in the history: runs that look like base64, and text too long
//! for one piece. A pure pass over text; [`super`] keeps what it pulls out.

use std::ops::Range;

use base64::Engine as _;
use base64::alphabet::{STANDARD, URL_SAFE};
use base64::engine::DecodePaddingMode;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use glave_ports::computer::looks_binary;
use glave_ports::saved_line;

use crate::files::sniffed;

/// The longest run that looks like base64 the history holds, in bytes.
pub(crate) const MOST_RUN: usize = 8 * 1024;
/// The longest piece of text the history holds, in bytes.
pub(crate) const MOST_TEXT: usize = 64 * 1024;
/// What stays of text too long for one piece: as much as a command's output starts with.
const PREVIEW: usize = 8 * 1024;
/// The widths base64 is wrapped at: PEM's 64, MIME's 76, and those tools add around them.
const WRAPS: [usize; 5] = [60, 64, 72, 76, 80];

/// `text`, kept within `room` bytes: each run that looks like base64 and whatever is too long
/// becomes a short line, its bytes added to `saved`. Text with neither stays as it is.
pub(crate) fn sweep(text: &mut String, room: usize, saved: &mut Vec<Vec<u8>>) {
    let found = runs(text);
    if !found.is_empty() {
        let mut swept = String::new();
        let mut from = 0;
        for run in found {
            swept.push_str(text.get(from..run.start).unwrap_or_default());
            let encoded = text.get(run.clone()).unwrap_or_default();
            let bytes = decoded(encoded).unwrap_or_else(|| encoded.as_bytes().to_vec());
            swept.push_str(&saved_line(word_for(media_type(&bytes)), &bytes));
            saved.push(bytes);
            from = run.end;
        }
        swept.push_str(text.get(from..).unwrap_or_default());
        *text = swept;
    }
    if text.len() > room {
        let mut end = PREVIEW.min(room);
        while !text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        let line = saved_line("text", text.as_bytes());
        let preview = text.get(..end).unwrap_or_default();
        let shown = format!("{preview}\n{line}");
        saved.push(std::mem::replace(text, shown).into_bytes());
    }
}

/// What `bytes` hold, told from their first bytes alone.
pub(crate) fn media_type(bytes: &[u8]) -> &'static str {
    if let Some(sniffed) = sniffed(bytes) {
        return sniffed;
    }
    let brand = bytes.get(8..12);
    match bytes {
        [b'R', b'I', b'F', b'F', ..] if brand == Some(b"WAVE") => "audio/wav",
        [b'I', b'D', b'3', ..] => "audio/mpeg",
        [0xFF, second, ..] if second & 0xE0 == 0xE0 => "audio/mpeg",
        [b'O', b'g', b'g', b'S', ..] => "audio/ogg",
        [b'f', b'L', b'a', b'C', ..] => "audio/flac",
        [_, _, _, _, b'f', b't', b'y', b'p', b'M', b'4', b'A', ..] => "audio/mp4",
        [_, _, _, _, b'f', b't', b'y', b'p', ..] => "video/mp4",
        [b'P', b'K', 3, 4, ..] => "application/zip",
        [0x1F, 0x8B, ..] => "application/gzip",
        _ if looks_binary(bytes) => "application/octet-stream",
        _ => "text/plain",
    }
}

/// What the line calls what is of `media_type`.
fn word_for(media_type: &str) -> &'static str {
    match media_type.split('/').next() {
        Some("image") => "image",
        Some("audio") => "audio",
        Some("text") => "text",
        _ => "binary data",
    }
}

/// The kinds of letter a stretch holds, as bits. Encoded bytes hold all of them; words, paths
/// and hashes rarely do.
const UPPER: u8 = 1;
const LOWER: u8 = 2;
const DIGIT: u8 = 4;
const BEYOND_HEX: u8 = 8;
const ENCODED: u8 = UPPER | LOWER | DIGIT | BEYOND_HEX;

/// The kinds `byte`, a letter of base64, is.
fn kind(byte: u8) -> u8 {
    let case = match byte {
        b'A'..=b'Z' => UPPER,
        b'a'..=b'z' => LOWER,
        b'0'..=b'9' => DIGIT,
        _ => 0,
    };
    case | if byte.is_ascii_hexdigit() {
        0
    } else {
        BEYOND_HEX
    }
}

/// Lines in a row, each as wide as the first but for a shorter last, as base64 is wrapped.
struct Chain {
    start: usize,
    end: usize,
    letters: usize,
    lines: usize,
    width: usize,
    kinds: u8,
    /// A shorter line or padding ended it.
    ended: bool,
    /// Where its next line would start.
    next: Option<usize>,
}

impl Chain {
    /// Whether this is what the history must not hold: one unbroken run over the limit, or
    /// wrapped lines over it that read as encoded bytes.
    fn over(&self) -> bool {
        self.letters > MOST_RUN && (self.lines == 1 || self.kinds == ENCODED)
    }
}

/// Where in `text` the runs that look like base64 are, in order.
fn runs(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut chain: Option<Chain> = None;
    let mut at = 0;
    while at < bytes.len() {
        let Some(token) = token(bytes, at) else {
            // An escape in a JSON string is no letter of a run after it.
            let escape = bytes.get(at) == Some(&b'\\')
                && bytes
                    .get(at.saturating_add(1))
                    .is_some_and(u8::is_ascii_alphabetic);
            at = at.saturating_add(if escape { 2 } else { 1 });
            continue;
        };
        let next = next_line(bytes, token.end);
        let joins = chain.as_ref().is_some_and(|chain| {
            !chain.ended
                && chain.next == Some(at)
                && WRAPS.contains(&chain.width)
                && (token.letters == chain.width
                    || token.letters < chain.width && !starts_a_sentence(bytes, token.end))
        });
        if let Some(chain) = chain.as_mut().filter(|_| joins) {
            chain.end = token.end;
            chain.letters = chain.letters.saturating_add(token.letters);
            chain.lines = chain.lines.saturating_add(1);
            // A shorter last line may be a word after the run: only whole lines say what the
            // run is.
            if token.letters == chain.width {
                chain.kinds |= token.kinds;
            }
            chain.ended = token.padded || token.letters < chain.width;
            chain.next = next;
        } else {
            if let Some(done) = chain.take().filter(Chain::over) {
                found.push(done.start..done.end);
            }
            chain = Some(Chain {
                start: at,
                end: token.end,
                letters: token.letters,
                lines: 1,
                width: token.letters,
                kinds: token.kinds,
                ended: token.padded,
                next,
            });
        }
        at = next.unwrap_or(token.end);
    }
    if let Some(done) = chain.filter(Chain::over) {
        found.push(done.start..done.end);
    }
    found
}

/// A stretch of base64's letters and the padding after it, ending at `end`.
struct Token {
    end: usize,
    letters: usize,
    kinds: u8,
    padded: bool,
}

/// The token starting at `at`, if a letter of base64 is there. A slash a JSON writer escaped
/// (`\/`) is a letter.
fn token(bytes: &[u8], at: usize) -> Option<Token> {
    let mut end = at;
    let mut letters = 0usize;
    let mut kinds = 0;
    loop {
        match (bytes.get(end), bytes.get(end.saturating_add(1))) {
            (Some(byte), _) if is_letter(*byte) => {
                kinds |= kind(*byte);
                end = end.saturating_add(1);
            }
            (Some(b'\\'), Some(b'/')) => {
                kinds |= kind(b'/');
                end = end.saturating_add(2);
            }
            _ => break,
        }
        letters = letters.saturating_add(1);
    }
    if letters == 0 {
        return None;
    }
    let mut padding = 0;
    while padding < 2 && bytes.get(end) == Some(&b'=') {
        end = end.saturating_add(1);
        padding += 1;
    }
    Some(Token {
        end,
        letters,
        kinds,
        padded: padding > 0,
    })
}

/// Whether what follows a shorter line says it is a word, not the end of a run wrapped in lines.
fn starts_a_sentence(bytes: &[u8], after: usize) -> bool {
    matches!(bytes.get(after), Some(b' ' | b'.' | b',' | b':' | b';'))
}

fn is_letter(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'-' | b'_')
}

/// Where the line after a token ending at `at` starts, when only a line break, as written or
/// as a JSON string escapes it, and an indent come between.
fn next_line(bytes: &[u8], at: usize) -> Option<usize> {
    let rest = bytes.get(at..)?;
    let broken = [&b"\r\n"[..], b"\n", b"\\r\\n", b"\\n"]
        .into_iter()
        .find(|line_break| rest.starts_with(line_break))?;
    let mut next = at.saturating_add(broken.len());
    while matches!(bytes.get(next), Some(b' ' | b'\t')) {
        next = next.saturating_add(1);
    }
    Some(next)
}

/// The bytes a run encodes, when it reads as base64: `None` for a run that only looks like it,
/// which is kept as the text it is.
fn decoded(run: &str) -> Option<Vec<u8>> {
    let mut plain = Vec::with_capacity(run.len());
    let mut letters = run.bytes();
    let mut kinds = 0;
    while let Some(byte) = letters.next() {
        match byte {
            b'\\' => {
                if letters.next() == Some(b'/') {
                    kinds |= kind(b'/');
                    plain.push(b'/');
                }
            }
            b'=' | b'\r' | b'\n' | b' ' | b'\t' => {}
            _ => {
                kinds |= kind(byte);
                plain.push(byte);
            }
        }
    }
    if kinds != ENCODED {
        return None;
    }
    let url_safe = plain.iter().any(|byte| matches!(byte, b'-' | b'_'));
    if url_safe && plain.iter().any(|byte| matches!(byte, b'+' | b'/')) {
        return None;
    }
    let config = GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireNone);
    let alphabet = if url_safe { &URL_SAFE } else { &STANDARD };
    GeneralPurpose::new(alphabet, config).decode(plain).ok()
}

#[cfg(test)]
mod tests;

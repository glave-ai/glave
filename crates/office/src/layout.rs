//! A part's XML laid out to edit, and drawn back together to pack. Word writes a document's
//! whole body on one line, too long to read or edit a piece at a time, so each element that
//! follows another tag starts a line of its own, indented by its depth. Packing takes those
//! line breaks out again, so a part no one edited goes back as it came.

use quick_xml::events::Event;
use quick_xml::reader::Reader;

/// What a piece of XML is, for laying it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Piece {
    /// A start tag.
    Open,
    /// An empty element, a comment or a processing instruction: one tag that holds nothing.
    Lone,
    /// An end tag.
    Close,
    /// Text, a reference or CDATA.
    Text,
    /// The declaration, or a document type.
    Prolog,
}

/// A piece of XML: what it is, where it ends, and how many elements hold it.
#[derive(Debug, Clone, Copy)]
struct Placed {
    piece: Piece,
    end: usize,
    depth: usize,
}

/// `text` cut into its pieces; `None` when it is not XML quick-xml reads through.
fn pieces(text: &str) -> Option<Vec<Placed>> {
    let mut reader = Reader::from_str(text);
    let mut pieces = Vec::new();
    let mut depth = 0usize;
    loop {
        let piece = match reader.read_event().ok()? {
            Event::Eof => return Some(pieces),
            Event::Start(_) => Piece::Open,
            Event::Empty(_) | Event::Comment(_) | Event::PI(_) => Piece::Lone,
            Event::End(_) => Piece::Close,
            Event::Text(_) | Event::GeneralRef(_) | Event::CData(_) => Piece::Text,
            Event::Decl(_) | Event::DocType(_) => Piece::Prolog,
        };
        if piece == Piece::Close {
            depth = depth.saturating_sub(1);
        }
        let end = usize::try_from(reader.buffer_position()).ok()?;
        pieces.push(Placed { piece, end, depth });
        if piece == Piece::Open {
            depth += 1;
        }
    }
}

/// `text` with each element inside the root that follows another tag on a line of its own,
/// indented a space for each element holding it past the root. Text is never touched, so
/// nothing a reader sees changes; XML quick-xml can't read through is left as it is.
pub(crate) fn spread(text: &str) -> String {
    let Some(pieces) = pieces(text) else {
        return text.to_owned();
    };
    let mut spread = String::with_capacity(text.len() + text.len() / 4);
    let (mut start, mut before) = (0, Piece::Text);
    for Placed { piece, end, depth } in pieces {
        if matches!(piece, Piece::Open | Piece::Lone) && depth > 0 && before != Piece::Text {
            spread.push('\n');
            spread.extend(std::iter::repeat_n(' ', depth - 1));
        }
        spread.push_str(text.get(start..end).unwrap_or_default());
        (start, before) = (end, piece);
    }
    spread
}

/// `text` drawn back together: every line break [`spread`] makes taken out again, with its
/// indent, and those an edit leaves between tags. That is blank text inside the root, holding
/// a line break, between two tags; text an element holds alone, as `<w:t> </w:t>` does,
/// always stays.
pub(crate) fn condensed(text: &str) -> String {
    let Some(pieces) = pieces(text) else {
        return text.to_owned();
    };
    let mut condensed = String::with_capacity(text.len());
    let mut start = 0;
    for (index, placed) in pieces.iter().enumerate() {
        let piece = text.get(start..placed.end).unwrap_or_default();
        let before = index
            .checked_sub(1)
            .and_then(|before| pieces.get(before))
            .map(|before| before.piece);
        let after = pieces.get(index + 1).map(|after| after.piece);
        let between_tags = before.is_some_and(|before| before != Piece::Text)
            && matches!(after, Some(Piece::Open | Piece::Lone | Piece::Close));
        let alone = before == Some(Piece::Open) && after == Some(Piece::Close);
        let laid_out = placed.piece == Piece::Text
            && placed.depth > 0
            && piece.contains('\n')
            && piece
                .bytes()
                .all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
            && between_tags
            && !alone;
        if !laid_out {
            condensed.push_str(piece);
        }
        start = placed.end;
    }
    condensed
}

#[cfg(test)]
mod tests;

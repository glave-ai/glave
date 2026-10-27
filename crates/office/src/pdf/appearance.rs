//! A filled field's look drawn again, so a viewer that shows what the file holds, rather than
//! redrawing forms itself, shows the new value: the text from the left in Helvetica, at the
//! field's own size and colour, kept inside its box. Viewers that redraw forms are asked to as
//! well (`NeedAppearances`), and lay it out as the form says.

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Encoding, Object, ObjectId, Stream, StringFormat};

/// The font's name in each look glave draws.
const FONT: &str = "Helv";

/// The size of text when the field leaves it to the viewer (`0 Tf`) and has several lines.
const SEVERAL_LINES: f32 = 10.0;

/// The widget `id`'s normal look made to show `text`, in the look `look` gives (its `DA`),
/// one line or several. `font` is the Helvetica every look shares, made at the first.
pub(super) fn draw(
    document: &mut Document,
    id: ObjectId,
    look: Option<&[u8]>,
    text: &str,
    several: bool,
    font: &mut Option<ObjectId>,
) {
    let Some((width, height)) = size(document, id) else {
        return;
    };
    let (size, colour) = styled(look);
    let size = match size {
        Some(size) if size > 0.0 => size,
        _ if several => SEVERAL_LINES,
        _ => ((height - 4.0) * 0.7).clamp(4.0, 12.0),
    };
    let lines: Vec<&str> = if several {
        text.lines().collect()
    } else {
        vec![text]
    };
    let font = *font.get_or_insert_with(|| document.add_object(helvetica()));
    let mut operations = vec![
        Operation::new("BMC", vec!["Tx".into()]),
        Operation::new("q", vec![]),
        Operation::new(
            "re",
            vec![
                1.into(),
                1.into(),
                (width - 2.0).into(),
                (height - 2.0).into(),
            ],
        ),
        Operation::new("W", vec![]),
        Operation::new("n", vec![]),
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![FONT.into(), size.into()]),
        colour,
    ];
    let first = if several {
        height - 2.0 - size
    } else {
        (height - size) / 2.0 + size * 0.22
    };
    let encoding = Encoding::SimpleEncoding(b"WinAnsiEncoding");
    for (index, line) in lines.iter().enumerate() {
        let (x, y) = if index == 0 {
            (2.0, first)
        } else {
            (0.0, -size * 1.15)
        };
        operations.push(Operation::new("Td", vec![x.into(), y.into()]));
        let shown = Object::String(encoding.string_to_bytes(line), StringFormat::Literal);
        operations.push(Operation::new("Tj", vec![shown]));
    }
    operations.extend(["ET", "Q", "EMC"].map(|operator| Operation::new(operator, vec![])));
    let Ok(content) = (Content { operations }).encode() else {
        return;
    };
    let mut fonts = Dictionary::new();
    fonts.set(FONT, font);
    let mut resources = Dictionary::new();
    resources.set("Font", fonts);
    let mut form = Dictionary::new();
    form.set("Type", "XObject");
    form.set("Subtype", "Form");
    form.set(
        "BBox",
        vec![0.into(), 0.into(), width.into(), height.into()],
    );
    form.set("Resources", resources);
    let drawn = document.add_object(Stream::new(form, content));
    if let Ok(widget) = document.get_dictionary_mut(id) {
        let mut looks = Dictionary::new();
        looks.set("N", drawn);
        widget.set("AP", looks);
    }
}

/// The width and height of the widget `id`'s box, when it has one big enough to draw in.
fn size(document: &Document, id: ObjectId) -> Option<(f32, f32)> {
    let corners = document.get_dictionary(id).ok()?.get(b"Rect").ok()?;
    let corners = super::resolved(document, corners)?.as_array().ok()?;
    let [x1, y1, x2, y2] = corners.as_slice() else {
        return None;
    };
    let at = |corner: &Object| corner.as_float().ok();
    let (width, height) = ((at(x2)? - at(x1)?).abs(), (at(y2)? - at(y1)?).abs());
    (width > 2.0 && height > 2.0).then_some((width, height))
}

/// The text size a field's look (`DA`) gives, if any, and the colour to draw in: its own, or
/// black.
fn styled(look: Option<&[u8]>) -> (Option<f32>, Operation) {
    let operations = look
        .and_then(|look| Content::decode(look).ok())
        .map(|content| content.operations)
        .unwrap_or_default();
    let size = operations
        .iter()
        .find(|operation| operation.operator == "Tf")
        .and_then(|operation| operation.operands.get(1))
        .and_then(|size| size.as_float().ok());
    let colour = operations
        .into_iter()
        .find(|operation| ["g", "rg", "k"].contains(&operation.operator.as_str()))
        .unwrap_or_else(|| Operation::new("g", vec![0.into()]));
    (size, colour)
}

/// The font each look is drawn in: Helvetica, which every viewer has, in Windows' encoding.
fn helvetica() -> Dictionary {
    let mut font = Dictionary::new();
    font.set("Type", "Font");
    font.set("Subtype", "Type1");
    font.set("BaseFont", "Helvetica");
    font.set("Encoding", "WinAnsiEncoding");
    font
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_look_gives_its_size_and_colour_and_black_is_the_default() {
        let (size, colour) = styled(Some(b"/TiRo 9 Tf 0.2 0.3 0.4 rg"));
        assert_eq!(size, Some(9.0));
        assert_eq!(colour.operator, "rg");
        assert_eq!(colour.operands.len(), 3);
        let (size, colour) = styled(Some(b"/Helv 0 Tf"));
        assert_eq!(size, Some(0.0));
        assert_eq!(
            (colour.operator.as_str(), colour.operands),
            ("g", vec![0.into()])
        );
        assert_eq!(styled(None).0, None);
    }
}

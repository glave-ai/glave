//! Images made fit to send a model, once, as they come into a thread (`specs/session.md`,
//! "Attachments"): upright, at most [`LONG_EDGE`] on the long side and [`MAX_IMAGE`] in bytes.
//! An image already so is sent as it is; any other, and every GIF (OpenAI reads no animated
//! ones), is scaled down and encoded as a JPEG, what was transparent made white and its
//! metadata (where it was taken, with what) left out. The original is never changed.

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder as _, ImageFormat, ImageReader, Limits, Rgb, RgbImage};

/// The long edge an image is sent at, at most, in pixels: what Claude reads at its standard
/// tier without scaling it down itself (its newest models read up to 2,576); OpenAI scales
/// to 2,048, and refuses no image this size.
pub(crate) const LONG_EDGE: u32 = 1_568;

/// The largest image sent as it is, in bytes: 5 MB once in base64, what Amazon Bedrock takes
/// for Claude (Anthropic's own API takes 10 MB, OpenAI's far more).
pub(crate) const MAX_IMAGE: u64 = 3_750_000;

/// The JPEG qualities a copy is tried at, best first: at 85 text stays sharp and a phone
/// photo comes to a few hundred kilobytes; the lower ones are for what compresses badly, such
/// as noise, so the copy stays within [`MAX_IMAGE`].
const QUALITIES: [u8; 3] = [85, 60, 40];

/// The most memory decoding one image may take: a 48-megapixel photo fits, a small file that
/// would unpack to gigabytes does not.
const MAX_DECODED: u64 = 256 * 1024 * 1024;

/// The widest and tallest image decoded at all, in pixels.
const MAX_SIDE: u32 = 20_000;

/// An image, made fit to send.
#[derive(Debug)]
pub(crate) struct Fitted {
    /// Its width as given, upright, in pixels.
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// The JPEG the model is sent in its place; `None` when it is sent as it is.
    pub(crate) copy: Option<Resized>,
}

/// An image's copy, fit to send.
#[derive(Debug)]
pub(crate) struct Resized {
    pub(crate) bytes: Vec<u8>,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// `bytes`, an image, made fit to send: `None` when they are not one glave reads (PNG, JPEG,
/// GIF or WebP, whole), or would take too much memory to decode.
pub(crate) fn fit(bytes: &[u8]) -> Option<Fitted> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let format = reader.format()?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODED);
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).ok()?;
    image.apply_orientation(orientation);
    let (width, height) = (image.width(), image.height());
    let small = width.max(height) <= LONG_EDGE;
    let light = u64::try_from(bytes.len()).is_ok_and(|len| len <= MAX_IMAGE);
    if small && light && orientation == Orientation::NoTransforms && format != ImageFormat::Gif {
        return Some(Fitted {
            width,
            height,
            copy: None,
        });
    }
    if !small {
        image = image.resize(LONG_EDGE, LONG_EDGE, FilterType::Lanczos3);
    }
    let rgb = over_white(image);
    let bytes = QUALITIES.iter().find_map(|quality| {
        let mut out = Vec::new();
        let encoded = JpegEncoder::new_with_quality(&mut out, *quality).encode_image(&rgb);
        let light = u64::try_from(out.len()).is_ok_and(|len| len <= MAX_IMAGE);
        (encoded.is_ok() && light).then_some(out)
    })?;
    Some(Fitted {
        width,
        height,
        copy: Some(Resized {
            bytes,
            width: rgb.width(),
            height: rgb.height(),
        }),
    })
}

/// `image` with nothing transparent: what was shows white, as on a page.
fn over_white(image: DynamicImage) -> RgbImage {
    if !image.color().has_alpha() {
        return image.into_rgb8();
    }
    let rgba = image.into_rgba8();
    let mut rgb = RgbImage::new(rgba.width(), rgba.height());
    for (out, pixel) in rgb.pixels_mut().zip(rgba.pixels()) {
        let [r, g, b, alpha] = pixel.0;
        let alpha = u16::from(alpha);
        // At most 255 * 255, so it fits, and divided back to a byte.
        let over = |channel: u8| {
            let mixed = (u16::from(channel) * alpha + 255 * (255 - alpha)) / 255;
            u8::try_from(mixed).unwrap_or(u8::MAX)
        };
        *out = Rgb([over(r), over(g), over(b)]);
    }
    rgb
}

#[cfg(test)]
mod tests;

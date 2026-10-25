//! Images made fit to send a model, once, as they come into a thread (`specs/session.md`,
//! "Attachments"): upright, at most [`LONG_EDGE`] on the long side and [`MAX_IMAGE`] in bytes,
//! with no metadata. An image already so is sent as it is; any other, and every GIF (OpenAI
//! reads no animated ones), gets a copy: scaled down, what was transparent made white, its
//! metadata (where it was taken, with what) left out, encoded as a JPEG. The original is never
//! changed.

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{
    DynamicImage, ImageDecoder as _, ImageError, ImageFormat, ImageReader, Limits, Rgb, RgbImage,
};

use super::FileError;

/// The long edge an image is sent at, at most, in pixels: what Claude reads at its standard
/// tier without scaling it down itself (its newest models read up to 2,576); OpenAI scales
/// to 2,048, and refuses no image this size.
const LONG_EDGE: u32 = 1_568;

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
pub const MAX_SIDE: u32 = 20_000;

/// The copy of `bytes`, an image, the model is sent in its place: a JPEG fit to send, or
/// `None` when the image is sent as it is. Refused when they are not a whole image glave reads
/// (PNG, JPEG, GIF or WebP), or one too large to open.
pub(crate) fn fit(bytes: &[u8]) -> Result<Option<Vec<u8>>, FileError> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| FileError::Unreadable)?;
    let format = reader.format().ok_or(FileError::Unreadable)?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODED);
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|error| refused(&error))?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let metadata = decoder.exif_metadata().ok().flatten().is_some();
    let mut image = DynamicImage::from_decoder(decoder).map_err(|error| refused(&error))?;
    image.apply_orientation(orientation);
    let small = image.width().max(image.height()) <= LONG_EDGE;
    let light = u64::try_from(bytes.len()).is_ok_and(|len| len <= MAX_IMAGE);
    let upright = orientation == Orientation::NoTransforms;
    if small && light && upright && !metadata && format != ImageFormat::Gif {
        return Ok(None);
    }
    if !small {
        image = image.resize(LONG_EDGE, LONG_EDGE, FilterType::Lanczos3);
    }
    let rgb = over_white(image);
    let copy = QUALITIES.iter().find_map(|quality| {
        let mut out = Vec::new();
        let encoded = JpegEncoder::new_with_quality(&mut out, *quality).encode_image(&rgb);
        let fits = u64::try_from(out.len()).is_ok_and(|len| len <= MAX_IMAGE);
        (encoded.is_ok() && fits).then_some(out)
    });
    copy.map(Some).ok_or(FileError::Unreadable)
}

/// Why an image could not be opened: too large to, or not a whole image.
fn refused(error: &ImageError) -> FileError {
    match error {
        ImageError::Limits(_) => FileError::TooLarge,
        _ => FileError::Unreadable,
    }
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

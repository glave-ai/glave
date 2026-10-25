use image::{ImageEncoder as _, RgbaImage};

use super::*;

/// `image` encoded as `format`.
fn encoded(image: &DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, format).unwrap();
    out.into_inner()
}

/// A stand-in for a photo: gradients with grain, so it compresses as one does.
fn grainy(width: u32, height: u32) -> DynamicImage {
    let mut seed: u32 = 0x9e37_79b9;
    DynamicImage::ImageRgb8(RgbImage::from_fn(width, height, |x, y| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let grain = u8::try_from(seed % 24).unwrap();
        let shade = |value: u32, of: u32| u8::try_from(value * 200 / of).unwrap() + grain;
        Rgb([
            shade(x, width),
            shade(y, height),
            shade(x + y, width + height),
        ])
    }))
}

fn decoded(bytes: &[u8]) -> DynamicImage {
    image::load_from_memory(bytes).unwrap()
}

#[test]
fn a_phone_photo_is_scaled_to_the_long_edge_as_a_jpeg_of_a_few_hundred_kilobytes() {
    let photo = encoded(&grainy(4032, 3024), ImageFormat::Jpeg);
    assert!(photo.len() > 1_000_000, "a photo of {} bytes", photo.len());
    let fitted = fit(&photo).unwrap();
    assert_eq!((fitted.width, fitted.height), (4032, 3024));
    let copy = fitted.copy.unwrap();
    assert_eq!((copy.width, copy.height), (1568, 1176));
    assert!(copy.bytes.len() < 600_000, "{} bytes", copy.bytes.len());
    assert_eq!(image::guess_format(&copy.bytes).unwrap(), ImageFormat::Jpeg);
    let sent = decoded(&copy.bytes);
    assert_eq!((sent.width(), sent.height()), (1568, 1176));
}

#[test]
fn a_small_light_upright_image_is_sent_as_it_is() {
    for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
        let fitted = fit(&encoded(&grainy(1568, 900), format)).unwrap();
        assert_eq!((fitted.width, fitted.height), (1568, 900));
        assert!(fitted.copy.is_none(), "{format:?}");
    }
}

#[test]
fn a_photo_turned_by_its_metadata_is_made_upright() {
    // EXIF orientation 6: shown turned a quarter clockwise.
    let exif = b"MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0".to_vec();
    let mut photo = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut photo, 90);
    encoder.set_exif_metadata(exif).unwrap();
    let image = grainy(400, 300).into_rgb8();
    encoder
        .write_image(&image, 400, 300, image::ExtendedColorType::Rgb8)
        .unwrap();
    let fitted = fit(&photo).unwrap();
    assert_eq!((fitted.width, fitted.height), (300, 400));
    let copy = fitted.copy.unwrap();
    assert_eq!((copy.width, copy.height), (300, 400));
}

#[test]
fn what_was_transparent_shows_white() {
    let clear = DynamicImage::ImageRgba8(RgbaImage::new(2000, 1000));
    let copy = fit(&encoded(&clear, ImageFormat::Png))
        .unwrap()
        .copy
        .unwrap();
    assert_eq!((copy.width, copy.height), (1568, 784));
    let sent = decoded(&copy.bytes).into_rgb8();
    assert!(
        sent.pixels().all(|pixel| pixel.0.iter().all(|c| *c > 250)),
        "white"
    );
}

#[test]
fn a_gif_is_sent_as_a_jpeg_of_its_first_frame() {
    let fitted = fit(&encoded(&grainy(64, 48), ImageFormat::Gif)).unwrap();
    let copy = fitted.copy.unwrap();
    assert_eq!((copy.width, copy.height), (64, 48));
    assert_eq!(image::guess_format(&copy.bytes).unwrap(), ImageFormat::Jpeg);
}

#[test]
fn even_noise_at_the_long_edge_comes_within_the_size_sent() {
    let mut seed: u32 = 7;
    let noise = RgbImage::from_fn(LONG_EDGE, LONG_EDGE, |_, _| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let [r, g, b, _] = seed.to_le_bytes();
        Rgb([r, g, b])
    });
    let png = encoded(&DynamicImage::ImageRgb8(noise), ImageFormat::Png);
    assert!(png.len() as u64 > MAX_IMAGE, "too heavy to send as it is");
    let copy = fit(&png).unwrap().copy.unwrap();
    assert_eq!((copy.width, copy.height), (LONG_EDGE, LONG_EDGE));
    assert!(copy.bytes.len() as u64 <= MAX_IMAGE, "{}", copy.bytes.len());
}

#[test]
fn what_is_not_a_whole_image_glave_reads_is_refused() {
    let png = encoded(&grainy(32, 32), ImageFormat::Png);
    assert!(
        fit(png.get(..png.len() / 2).unwrap()).is_none(),
        "cut short"
    );
    assert!(fit(b"%PDF-1.7\n").is_none());
    assert!(fit(b"plain text").is_none());
    let wide = encoded(&grainy(MAX_SIDE + 1, 1), ImageFormat::Png);
    assert!(fit(&wide).is_none(), "too wide to decode");
}

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

/// A JPEG of `image` carrying `exif`.
fn with_exif(image: &DynamicImage, exif: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut out, 90);
    encoder.set_exif_metadata(exif.to_vec()).unwrap();
    let rgb = image.to_rgb8();
    encoder
        .write_image(
            &rgb,
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    out
}

/// The copy `bytes` are sent as, decoded, checked to be a JPEG.
fn copied(bytes: &[u8]) -> DynamicImage {
    let copy = fit(bytes).unwrap().unwrap();
    assert_eq!(image::guess_format(&copy).unwrap(), ImageFormat::Jpeg);
    image::load_from_memory(&copy).unwrap()
}

#[test]
fn a_phone_photo_is_scaled_to_the_long_edge_as_a_jpeg_of_a_few_hundred_kilobytes() {
    let photo = encoded(&grainy(4032, 3024), ImageFormat::Jpeg);
    assert!(photo.len() > 1_000_000, "a photo of {} bytes", photo.len());
    let copy = fit(&photo).unwrap().unwrap();
    assert!(copy.len() < 600_000, "{} bytes", copy.len());
    let sent = image::load_from_memory(&copy).unwrap();
    assert_eq!((sent.width(), sent.height()), (1568, 1176));
}

#[test]
fn a_small_light_upright_image_with_no_metadata_is_sent_as_it_is() {
    for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
        let fitted = fit(&encoded(&grainy(1568, 900), format)).unwrap();
        assert!(fitted.is_none(), "{format:?}");
    }
}

#[test]
fn a_photo_turned_by_its_metadata_is_made_upright() {
    // EXIF orientation 6: shown turned a quarter clockwise.
    let exif = b"MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0";
    let sent = copied(&with_exif(&grainy(400, 300), exif));
    assert_eq!((sent.width(), sent.height()), (300, 400));
}

#[test]
fn metadata_such_as_where_a_photo_was_taken_is_left_out() {
    // Upright (orientation 1), with a GPS block: small and light, but not sent as it is.
    let exif = b"MM\0\x2a\0\0\0\x08\0\x02\x01\x12\0\x03\0\0\0\x01\0\x01\0\0\x88\x25\0\x04\0\0\0\x01\0\0\0\x26\0\0\0\0\0\0\0\0\0\0";
    let photo = with_exif(&grainy(400, 300), exif);
    let copy = fit(&photo).unwrap().unwrap();
    let mut decoder = ImageReader::new(Cursor::new(&copy))
        .with_guessed_format()
        .unwrap()
        .into_decoder()
        .unwrap();
    assert_eq!(
        decoder.exif_metadata().unwrap(),
        None,
        "no metadata in the copy"
    );
}

#[test]
fn what_was_transparent_shows_white() {
    let clear = DynamicImage::ImageRgba8(RgbaImage::new(2000, 1000));
    let sent = copied(&encoded(&clear, ImageFormat::Png)).into_rgb8();
    assert_eq!((sent.width(), sent.height()), (1568, 784));
    assert!(
        sent.pixels().all(|pixel| pixel.0.iter().all(|c| *c > 250)),
        "white"
    );
}

#[test]
fn a_gif_is_sent_as_a_jpeg_of_its_first_frame() {
    let sent = copied(&encoded(&grainy(64, 48), ImageFormat::Gif));
    assert_eq!((sent.width(), sent.height()), (64, 48));
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
    let copy = fit(&png).unwrap().unwrap();
    assert!(copy.len() as u64 <= MAX_IMAGE, "{}", copy.len());
}

#[test]
fn what_is_not_a_whole_image_glave_reads_is_refused_and_a_huge_one_said_so() {
    let png = encoded(&grainy(32, 32), ImageFormat::Png);
    let unreadable = |bytes: &[u8]| matches!(fit(bytes), Err(FileError::Unreadable));
    assert!(unreadable(png.get(..png.len() / 2).unwrap()), "cut short");
    assert!(unreadable(b"%PDF-1.7\n"));
    assert!(unreadable(b"plain text"));
    let wide = encoded(&grainy(MAX_SIDE + 1, 1), ImageFormat::Png);
    assert!(
        matches!(fit(&wide), Err(FileError::TooLarge)),
        "too wide to open"
    );
}

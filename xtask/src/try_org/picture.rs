//! The picture try-org's stand-in makes for every image it is asked for, as OpenRouter's image
//! API answers: a PNG drawn here, a checkerboard in two greys, its rows stored as they are
//! (PNG's deflate allows that), so a person can see an image arrive, drawn and saved, with no
//! account, network or cost.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

/// Its side, in pixels.
const SIDE: u32 = 256;

/// What every image it makes says it cost, in dollars.
const COST: &str = "0.04";

/// The answer to `POST /images`: one picture, in base64, with what it cost.
pub(crate) fn answer() -> Value {
    let cost: Value = serde_json::from_str(COST).unwrap_or(Value::Null);
    json!({
        "created": 1_791_000_000,
        "data": [{"b64_json": STANDARD.encode(png()), "media_type": "image/png"}],
        "usage": {"prompt_tokens": 20, "completion_tokens": 1_024, "total_tokens": 1_044, "cost": cost},
    })
}

/// The picture, as a PNG: 8-bit greys, each row's filter byte then its pixels.
fn png() -> Vec<u8> {
    let mut rows = Vec::new();
    for y in 0..SIDE {
        rows.push(0);
        rows.extend((0..SIDE).map(|x| {
            if (x / 32 + y / 32) % 2 == 0 {
                0xd8
            } else {
                0x70
            }
        }));
    }
    let mut header = Vec::new();
    header.extend(SIDE.to_be_bytes());
    header.extend(SIDE.to_be_bytes());
    // 8 bits a pixel, greys, deflate, the usual filters, not interlaced.
    header.extend([8, 0, 0, 0, 0]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut png, *b"IHDR", &header);
    chunk(&mut png, *b"IDAT", &stored(&rows));
    chunk(&mut png, *b"IEND", &[]);
    png
}

/// `data` as a zlib stream of stored blocks, as deflate allows: no compression at all.
fn stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = data.chunks(usize::from(u16::MAX)).collect();
    for (n, block) in blocks.iter().enumerate() {
        let last = u8::from(n + 1 == blocks.len());
        let length = u16::try_from(block.len()).unwrap_or(u16::MAX);
        out.push(last);
        out.extend(length.to_le_bytes());
        out.extend((!length).to_le_bytes());
        out.extend_from_slice(block);
    }
    let (mut a, mut b) = (1_u32, 0_u32);
    for byte in data {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    out.extend(((b << 16) | a).to_be_bytes());
    out
}

/// Appends a PNG chunk of `kind` holding `data`, with its length and checksum.
fn chunk(png: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    png.extend(u32::try_from(data.len()).unwrap_or(u32::MAX).to_be_bytes());
    let start = png.len();
    png.extend_from_slice(&kind);
    png.extend_from_slice(data);
    let checked = png.get(start..).unwrap_or_default();
    png.extend(crc32(checked).to_be_bytes());
}

/// The CRC-32 PNG checks each chunk with.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_is_a_png_its_checksums_right() {
        let png = png();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(
            crc32(b"IEND"),
            0xae42_6082,
            "IEND's checksum, as every PNG ends"
        );
        assert!(png.ends_with(&[0xae, 0x42, 0x60, 0x82]));
        let answer = answer();
        assert_eq!(answer["usage"]["cost"], 0.04);
        assert_eq!(answer["data"][0]["media_type"], "image/png");
    }
}

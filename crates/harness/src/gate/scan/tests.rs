//! The scanner: what it must find, whatever wraps it, and what it must leave alone.

use base64::engine::general_purpose::{STANDARD as PADDED, URL_SAFE_NO_PAD};
use glave_ports::digest;
use proptest::prelude::*;

use super::*;

fn swept(text: &str) -> (String, Vec<Vec<u8>>) {
    let mut text = text.to_owned();
    let mut saved = Vec::new();
    sweep(&mut text, MOST_TEXT, &mut saved);
    (text, saved)
}

fn unchanged(text: &str) {
    let (after, saved) = swept(text);
    assert_eq!(saved.len(), 0, "pulled out of {text:.80}");
    assert_eq!(after, text);
}

fn wrapped(encoded: &str, width: usize, line_break: &str) -> String {
    encoded
        .as_bytes()
        .chunks(width)
        .map(|line| String::from_utf8_lossy(line).into_owned())
        .collect::<Vec<_>>()
        .join(line_break)
}

/// Bytes that look nothing like text, the same each run.
fn noise(size: usize) -> Vec<u8> {
    let mut state = 7u32;
    (0..size)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            state.to_be_bytes().first().copied().unwrap_or(0)
        })
        .collect()
}

/// A run must be gone, its bytes kept whole, and its line in its place.
fn pulled_out(text: &str, bytes: &[u8]) {
    let (after, saved) = swept(text);
    assert_eq!(saved, [bytes.to_vec()], "in {text:.80}");
    assert!(after.contains(digest(bytes).as_str()), "{after}");
    assert!(after.len() < 400, "{after}");
}

#[test]
fn base64_is_found_however_it_is_written() {
    let bytes = noise(12_000);
    let padded = PADDED.encode(&bytes);
    pulled_out(&format!("before {padded} after"), &bytes);
    pulled_out(&format!("x: {}\n", URL_SAFE_NO_PAD.encode(&bytes)), &bytes);
    pulled_out(&format!("data:image/png;base64,{padded}\""), &bytes);
    for width in [64, 76] {
        pulled_out(
            &format!("start\n{}\nend.", wrapped(&padded, width, "\n")),
            &bytes,
        );
        pulled_out(&format!("{}\r\n", wrapped(&padded, width, "\r\n")), &bytes);
        pulled_out(&format!("  {}\n", wrapped(&padded, width, "\n  ")), &bytes);
        pulled_out(
            &format!(r#"{{"data":"{}"}}"#, wrapped(&padded, width, "\\n")),
            &bytes,
        );
    }
    pulled_out(
        &format!(r#"{{"data":"{}"}}"#, padded.replace('/', "\\/")),
        &bytes,
    );
    pulled_out(&format!(r#"{{"note":"a\n{padded}"}}"#), &bytes);
}

#[test]
fn a_certificate_loses_its_body_and_keeps_its_words() {
    let bytes = noise(12_000);
    let body = wrapped(&PADDED.encode(&bytes), 64, "\n");
    let pem = format!("-----BEGIN CERTIFICATE-----\n{body}\n-----END CERTIFICATE-----\n");
    let (after, saved) = swept(&pem);
    assert_eq!(saved, [bytes]);
    assert!(
        after.starts_with("-----BEGIN CERTIFICATE-----\n["),
        "{after}"
    );
    assert!(after.ends_with("]\n-----END CERTIFICATE-----\n"), "{after}");
}

#[test]
fn what_is_kept_is_told_by_its_first_bytes() {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(noise(12_000));
    let (after, saved) = swept(&PADDED.encode(&png));
    assert!(after.starts_with("[image, "), "{after}");
    assert_eq!(
        saved.first().map(|bytes| media_type(bytes)),
        Some("image/png")
    );
    let mut wave = b"RIFF\0\0\0\0WAVE".to_vec();
    wave.extend(noise(12_000));
    assert_eq!(media_type(&wave), "audio/wav");
    assert!(swept(&PADDED.encode(&wave)).0.starts_with("[audio, "));
    assert_eq!(media_type(&noise(100)), "application/octet-stream");
    assert_eq!(media_type(b"PK\x03\x04rest"), "application/zip");
    assert_eq!(media_type(b"plain words"), "text/plain");
    let (after, saved) = swept(&"x".repeat(MOST_RUN + 1));
    assert!(after.starts_with("[text, "), "{after}");
    assert_eq!(saved.len(), 1);
}

#[test]
fn ordinary_text_under_the_limits_passes_unchanged() {
    // Source code, this very file.
    unchanged(include_str!("../scan.rs"));
    unchanged(include_str!("tests.rs"));
    // Long hex hashes, one a line, as `git log --format=%H` or a lockfile prints them.
    let hashes: Vec<String> = (0u8..=255)
        .flat_map(|n| [n, n.wrapping_mul(3)])
        .map(|n| digest(&[n]).as_str().to_owned())
        .collect();
    let hashes = hashes.join("\n");
    assert!(hashes.len() > MOST_RUN);
    unchanged(&hashes);
    // Minified JSON.
    let items: Vec<String> = (0..1_000)
        .map(|n| format!(r#"{{"id":{n},"name":"item-{n}","ok":true,"tag":"Ab3"}}"#))
        .collect();
    let json = format!("[{}]", items.join(","));
    assert!((MOST_RUN..MOST_TEXT).contains(&json.len()));
    unchanged(&json);
    // A listing of folders, a word a line: every letter base64's, none of them encoded.
    let folders: Vec<String> = (0..1_000)
        .map(|n| format!("crates/harness/src/Part{n}/tools"))
        .collect();
    unchanged(&folders.join("\n"));
    let words: Vec<String> = (0..4_000).map(|n| format!("Word{n}")).collect();
    unchanged(&words.join("\n"));
    // Even lines of every kind of letter, at a width nothing wraps base64 at.
    let even: Vec<String> = (1_000..1_400)
        .map(|n| format!("tests/fixtures/Case{n}/expected/output/x"))
        .collect();
    unchanged(&even.join("\n"));
    // A sequence wrapped as base64 is, of one case and no digits.
    unchanged(&wrapped(&"ACGTTGCA".repeat(2_000), 60, "\n"));
    // A run right at the limit stays; a line more is the gate's.
    let at_limit = PADDED.encode(noise(MOST_RUN / 4 * 3));
    assert_eq!(at_limit.len(), MOST_RUN);
    unchanged(&at_limit);
}

#[test]
fn a_wider_line_before_wrapped_base64_stays_out_of_it() {
    let bytes = noise(12_000);
    let encoded = wrapped(&PADDED.encode(&bytes), 76, "\n");
    let heading = "A".repeat(100);
    let (after, saved) = swept(&format!("{heading}\n{encoded}\n"));
    assert_eq!(saved, [bytes]);
    assert!(after.starts_with(&heading), "{after:.120}");
}

#[test]
fn a_word_after_a_run_is_left_out_of_it() {
    let bytes = noise(12_000);
    let encoded = PADDED.encode(&bytes);
    let (after, saved) = swept(&format!("{encoded}\nDone\n"));
    assert_eq!(saved, [bytes]);
    assert!(after.ends_with("]\nDone\n"), "{after}");
}

#[test]
fn text_too_long_for_one_piece_keeps_a_preview_and_a_line() {
    let long = "All work and no play. ".repeat(4_000);
    let (after, saved) = swept(&long);
    assert_eq!(saved, [long.clone().into_bytes()]);
    assert!(after.starts_with("All work"), "{after:.40}");
    assert!(after.len() < PREVIEW + 200);
    assert!(after.ends_with(&format!("{}]", digest(long.as_bytes()).as_str())));
}

#[test]
fn a_preview_never_cuts_a_character() {
    let (after, saved) = swept(&"é".repeat(MOST_TEXT));
    assert_eq!(saved.len(), 1);
    assert!(after.starts_with('é'));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Whatever its bytes and however it is wrapped, a run over the limit never stays, and is
    /// kept to the byte.
    #[test]
    fn a_run_over_the_limit_is_always_pulled_out(
        bytes in proptest::collection::vec(any::<u8>(), 6_200..16_000),
        width in prop_oneof![Just(0usize), Just(60), Just(64), Just(72), Just(76), Just(80)],
        url_safe in any::<bool>(),
        before in "[ -~]{0,20}",
        after in "[ -~]{0,20}",
    ) {
        let encoded = if url_safe {
            URL_SAFE_NO_PAD.encode(&bytes)
        } else {
            PADDED.encode(&bytes)
        };
        let run = if width == 0 { encoded } else { wrapped(&encoded, width, "\n") };
        let (text, saved) = swept(&format!("{before}\n\n{run}\n\n{after}"));
        prop_assert_eq!(saved, vec![bytes]);
        prop_assert!(runs(&text).is_empty());
        prop_assert!(text.len() < 400);
    }

    /// Any text comes out within the limits, and sweeping it again changes nothing.
    #[test]
    fn what_comes_out_is_within_the_limits_and_settled(
        text in prop_oneof![
            ".{0,2000}",
            "[A-Za-z0-9+/\n =]{0,70000}",
            "[a-f0-9]{64}(\n[a-f0-9]{64}){0,300}",
        ],
    ) {
        let (once, _) = swept(&text);
        prop_assert!(once.len() <= MOST_TEXT);
        prop_assert!(runs(&once).is_empty());
        let (twice, saved) = swept(&once);
        prop_assert_eq!(twice, once);
        prop_assert!(saved.is_empty());
    }

    /// Text with no long run and within the cap stays exactly as it was.
    #[test]
    fn text_with_nothing_to_pull_out_stays_as_it_is(
        words in proptest::collection::vec("[A-Za-z0-9/+_-]{1,30}", 0..4000),
        separator in prop_oneof![Just(" "), Just(", "), Just("."), Just("\n")],
    ) {
        let text = words.join(separator);
        prop_assume!(text.len() <= MOST_TEXT);
        let (after, saved) = swept(&text);
        prop_assert_eq!(after, text);
        prop_assert!(saved.is_empty());
    }
}

//! What the test server's `blocks` returns: results for the history's gate to judge.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rmcp::model::{CallToolResult, ContentBlock, ResourceContents};
use serde_json::json;

use super::said;

/// `size` bytes no one would call words: the same each run, as a test needs.
fn noise(size: usize) -> Vec<u8> {
    let mut state = 0x2545_f491_u32;
    (0..size)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            state.to_be_bytes().first().copied().unwrap_or(0)
        })
        .collect()
}

/// A result for the history's gate to judge: `image`, `audio` or `resource` blocks with bytes,
/// `base64` in text, or `structured` JSON holding it, its label saying nothing, or `flood`,
/// words too long for one piece of text.
pub(crate) fn blocks(what: &str) -> CallToolResult {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(noise(20_000));
    let encoded = STANDARD.encode(&png);
    match what {
        "image" => CallToolResult::success(vec![
            ContentBlock::text("Here is the chart."),
            ContentBlock::image(encoded, "text/plain"),
        ]),
        "audio" => {
            let mut wave = b"RIFF\0\0\0\0WAVE".to_vec();
            wave.extend(noise(20_000));
            CallToolResult::success(vec![ContentBlock::audio(
                STANDARD.encode(&wave),
                "audio/wav",
            )])
        }
        "resource" => {
            let contents = ResourceContents::blob(encoded, "test://chart");
            CallToolResult::success(vec![ContentBlock::resource(contents)])
        }
        "base64" => said(format!("The file: {encoded} and that is all.")),
        "structured" => {
            let mut result = CallToolResult::success(Vec::new());
            result.structured_content = Some(json!({"note": "plain", "data": encoded}));
            result
        }
        _ => said("All work and no play. ".repeat(4_000)),
    }
}

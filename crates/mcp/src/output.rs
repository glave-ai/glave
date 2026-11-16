//! A tool result as the model sees it: text parts, with what the server sent as bytes kept
//! apart and named by a line (`glave_ports::saved_line`). How much text stays is the harness's
//! gate's to say (`specs/invariants.md`, 26).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::Part;
use glave_ports::{ToolOutput, saved_line};
use rmcp::model::{CallToolResult, ContentBlock, ResourceContents};

/// Converts a server's result.
pub fn output(result: &CallToolResult) -> ToolOutput {
    let mut saved = Vec::new();
    let mut texts: Vec<String> = result
        .content
        .iter()
        .map(|block| describe(block, &mut saved))
        .collect();
    if texts.is_empty()
        && let Some(structured) = &result.structured_content
    {
        texts.push(structured.to_string());
    }
    if texts.is_empty() {
        texts.push("(no output)".to_owned());
    }
    ToolOutput {
        is_error: result.is_error.unwrap_or(false),
        parts: vec![Part::Text {
            text: texts.join("\n"),
        }],
        // The connection marks it untrusted for an open-world tool, the server for one the
        // person or a policy marked.
        untrusted: false,
        files: Vec::new(),
        saved,
        structured: None,
    }
}

fn describe(block: &ContentBlock, saved: &mut Vec<Vec<u8>>) -> String {
    match block {
        ContentBlock::Text(text) => text.text.clone(),
        ContentBlock::Image(image) => bytes_of("image", &image.data, saved),
        ContentBlock::Audio(audio) => bytes_of("audio", &audio.data, saved),
        ContentBlock::Resource(embedded) => resource(&embedded.resource, saved),
        ContentBlock::ResourceLink(link) => format!("[resource: {}]", link.uri),
        // A newer kind of content this version does not know.
        _ => "[content glave cannot show yet]".to_owned(),
    }
}

/// A resource's contents: text as it is, bytes added to `saved` and named by a line.
pub(crate) fn resource(content: &ResourceContents, saved: &mut Vec<Vec<u8>>) -> String {
    match content {
        ResourceContents::TextResourceContents { text, .. } => text.clone(),
        ResourceContents::BlobResourceContents { uri, blob, .. } => {
            bytes_of(&format!("resource {uri}"), blob, saved)
        }
        // A newer kind of resource this version does not know.
        _ => "[a resource glave cannot show]".to_owned(),
    }
}

/// The line for `what`, whose bytes `data` encodes, added to `saved`; data that is not base64
/// is kept as it came, and goes no further either.
fn bytes_of(what: &str, data: &str, saved: &mut Vec<Vec<u8>>) -> String {
    let bytes = STANDARD
        .decode(data)
        .unwrap_or_else(|_| data.as_bytes().to_vec());
    let line = saved_line(what, &bytes);
    saved.push(bytes);
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGO: &str = "iVBORw0KGgo=";

    #[test]
    fn text_is_kept_whole_for_the_gate_to_judge() {
        let result = CallToolResult::success(vec![ContentBlock::text("hello")]);
        assert_eq!(output(&result), ToolOutput::text(false, "hello"));
        let flood = "x".repeat(150_000);
        let result = CallToolResult::success(vec![ContentBlock::text(flood.clone())]);
        assert_eq!(output(&result), ToolOutput::text(false, flood));
    }

    #[test]
    fn errors_stay_errors_and_empty_results_say_so() {
        let failed = CallToolResult::error(vec![ContentBlock::text("no such issue")]);
        assert!(output(&failed).is_error);
        let empty = CallToolResult::success(Vec::new());
        assert_eq!(output(&empty), ToolOutput::text(false, "(no output)"));
    }

    #[test]
    fn bytes_become_a_line_in_their_place_whatever_their_label_says() {
        let result = CallToolResult::success(vec![
            ContentBlock::text("before"),
            ContentBlock::image(LOGO, "text/plain"),
            ContentBlock::text("after"),
        ]);
        let said = output(&result);
        let logo = STANDARD.decode(LOGO).unwrap();
        assert_eq!(said.saved, std::slice::from_ref(&logo));
        let line = saved_line("image", &logo);
        assert_eq!(
            said.parts,
            [Part::Text {
                text: format!("before\n{line}\nafter")
            }]
        );
    }

    #[test]
    fn data_that_is_not_base64_is_kept_as_it_came() {
        let result = CallToolResult::success(vec![ContentBlock::image("not base64!", "image/png")]);
        let said = output(&result);
        assert_eq!(said.saved, [b"not base64!".to_vec()]);
        let line = saved_line("image", b"not base64!");
        assert_eq!(said.parts, [Part::Text { text: line }]);
    }
}

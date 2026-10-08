//! A tool result as the model sees it: text parts, with what cannot be shown as text named
//! instead, and the whole cut to a size a model can take, as Claude Code caps MCP output.

use glave_event::Part;
use glave_ports::ToolOutput;
use rmcp::model::{CallToolResult, ContentBlock, ResourceContents};

/// The most text one result gives the model: about 25,000 tokens.
const MAX_CHARS: usize = 100_000;

/// Converts a server's result.
pub fn output(result: &CallToolResult) -> ToolOutput {
    let mut texts: Vec<String> = result.content.iter().map(describe).collect();
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
            text: cut(texts.join("\n")),
        }],
    }
}

/// `text`, cut to what one result may give the model.
pub(crate) fn cut(text: String) -> String {
    let total = text.chars().count();
    if total > MAX_CHARS {
        let kept: String = text.chars().take(MAX_CHARS).collect();
        format!("{kept}\n[cut short: {total} characters, only the first {MAX_CHARS} shown]")
    } else {
        text
    }
}

fn describe(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text(text) => text.text.clone(),
        ContentBlock::Image(image) => format!("[image: {}]", image.mime_type),
        ContentBlock::Audio(audio) => format!("[audio: {}]", audio.mime_type),
        ContentBlock::Resource(embedded) => match &embedded.resource {
            ResourceContents::TextResourceContents { text, .. } => text.clone(),
            ResourceContents::BlobResourceContents { uri, .. } => format!("[resource: {uri}]"),
            // A newer kind of resource this version does not know.
            _ => "[resource]".to_owned(),
        },
        ContentBlock::ResourceLink(link) => format!("[resource: {}]", link.uri),
        // A newer kind of content this version does not know.
        _ => "[content glave cannot show yet]".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_kept_and_a_flood_is_cut_short() {
        let result = CallToolResult::success(vec![ContentBlock::text("hello")]);
        assert_eq!(output(&result), ToolOutput::text(false, "hello"));
        let flood = CallToolResult::success(vec![ContentBlock::text("x".repeat(150_000))]);
        let ToolOutput { parts, .. } = output(&flood);
        let Some(Part::Text { text }) = parts.first() else {
            panic!("expected text");
        };
        assert!(text.ends_with("only the first 100000 shown]"));
        assert!(text.chars().count() < 100_100);
    }

    #[test]
    fn errors_stay_errors_and_empty_results_say_so() {
        let failed = CallToolResult::error(vec![ContentBlock::text("no such issue")]);
        assert!(output(&failed).is_error);
        let empty = CallToolResult::success(Vec::new());
        assert_eq!(output(&empty), ToolOutput::text(false, "(no output)"));
    }
}

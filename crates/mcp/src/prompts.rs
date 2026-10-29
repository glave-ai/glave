//! A server's prompts as commands the person types, as Claude Code and OpenCode offer them:
//! `/mcp__<server>__<prompt> some words` sends the prompt's text, the words filling its
//! arguments in order and the last taking any left over. The prompt is the person's own
//! message, so it runs nothing.

use rmcp::model::{ContentBlock, GetPromptResult, JsonObject, Prompt};

use crate::resources::contents_text;

/// `words` as `prompt`'s arguments, in the order it declares them; the last takes what is
/// left, so no word is lost.
pub(crate) fn arguments(prompt: &Prompt, words: &str) -> JsonObject {
    let declared = prompt.arguments.as_deref().unwrap_or_default();
    let mut rest = words.trim();
    let mut given = JsonObject::new();
    for (at, argument) in declared.iter().enumerate() {
        if rest.is_empty() {
            break;
        }
        let word = if at.saturating_add(1) == declared.len() {
            std::mem::take(&mut rest)
        } else {
            let (word, after) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            rest = after.trim_start();
            word
        };
        given.insert(argument.name.clone(), word.into());
    }
    given
}

/// What `result` sends: each message's text, a blank line apart. What is not text is named.
pub(crate) fn text(result: &GetPromptResult) -> String {
    let parts: Vec<String> = result
        .messages
        .iter()
        .map(|message| match &message.content {
            ContentBlock::Text(text) => text.text.clone(),
            ContentBlock::Resource(embedded) => {
                contents_text(std::slice::from_ref(&embedded.resource))
            }
            ContentBlock::ResourceLink(resource) => format!("[resource: {}]", resource.uri),
            ContentBlock::Image(_) => "[an image glave cannot send in a prompt]".to_owned(),
            _ => "[content glave cannot send in a prompt]".to_owned(),
        })
        .collect();
    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use rmcp::model::{PromptArgument, PromptMessage, Role};

    use super::*;

    fn prompt(arguments: &[&str]) -> Prompt {
        let declared = arguments
            .iter()
            .map(|name| PromptArgument::new(*name))
            .collect();
        Prompt::new("review", Some("Review a change."), Some(declared))
    }

    #[test]
    fn words_fill_the_arguments_in_order_and_the_last_takes_the_rest() {
        let given = arguments(&prompt(&["pr", "focus"]), "  12  the error  paths ");
        assert_eq!(given.get("pr"), Some(&"12".into()));
        assert_eq!(given.get("focus"), Some(&"the error  paths".into()));
        let short = arguments(&prompt(&["pr", "focus"]), "12");
        assert_eq!(short.get("focus"), None);
        assert!(arguments(&prompt(&[]), "ignored").is_empty());
    }

    #[test]
    fn a_prompts_messages_are_sent_as_text() {
        let result = GetPromptResult::new(vec![
            PromptMessage::new_text(Role::User, "Review PR 12."),
            PromptMessage::new_text(Role::Assistant, "Looking at it."),
        ]);
        assert_eq!(text(&result), "Review PR 12.\n\nLooking at it.");
    }
}

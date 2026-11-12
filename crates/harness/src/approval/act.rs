//! A call of a connected service's tool, as approvals take it: asked about by what it does and
//! the service, and a caution once the thread has read untrusted content, when the tool does
//! not say it only reads (`specs/approvals.md`, "Untrusted content").

use glave_event::{Ask, Risk};
use glave_ports::Naming;

use crate::toolset::{connector_of, short_of, words_of};

/// The most characters of a tool's title an approval takes as what the tool does.
const TITLE: usize = 60;

/// The words an act begins with: a tool whose title or name begins otherwise is asked about as
/// any tool is ("Allow this?"), since "Issue write with github?" would say less than nothing.
const VERBS: &[&str] = &[
    "accept",
    "acknowledge",
    "add",
    "append",
    "approve",
    "archive",
    "assign",
    "attach",
    "block",
    "book",
    "cancel",
    "change",
    "charge",
    "check",
    "clear",
    "close",
    "comment",
    "complete",
    "copy",
    "count",
    "create",
    "deactivate",
    "decline",
    "delete",
    "deploy",
    "describe",
    "disable",
    "dismiss",
    "download",
    "draft",
    "duplicate",
    "edit",
    "enable",
    "export",
    "fetch",
    "find",
    "fork",
    "forward",
    "get",
    "grant",
    "import",
    "insert",
    "invite",
    "label",
    "link",
    "list",
    "lock",
    "log",
    "look",
    "make",
    "mark",
    "merge",
    "modify",
    "move",
    "mute",
    "notify",
    "open",
    "pay",
    "pin",
    "post",
    "publish",
    "push",
    "put",
    "query",
    "react",
    "read",
    "refund",
    "remove",
    "rename",
    "reopen",
    "replace",
    "reply",
    "request",
    "reset",
    "resolve",
    "restore",
    "retrieve",
    "revoke",
    "run",
    "save",
    "schedule",
    "search",
    "send",
    "set",
    "share",
    "show",
    "snooze",
    "star",
    "start",
    "stop",
    "submit",
    "subscribe",
    "sync",
    "tag",
    "transfer",
    "transition",
    "trash",
    "trigger",
    "unarchive",
    "unpin",
    "unsubscribe",
    "update",
    "upload",
    "upsert",
    "view",
    "write",
];

/// What an approval of a call of a connected service's `tool` asks: what the tool does, by its
/// title or else its name in words, with the service, by the name `naming` gives it or else the
/// name its tools carry ("Send email with Gmail?"); as any tool is asked about when those words
/// would not begin with a verb.
pub(crate) fn ask_with(tool: &str, naming: &Naming) -> Ask {
    let act = (naming.title.as_deref())
        .and_then(titled)
        .or_else(|| named(tool));
    match act {
        Some(act) => Ask::With {
            act,
            service: service_of(tool, naming),
        },
        None => Ask::Allow,
    }
}

/// Why a call of a connected service's `tool` that does not say it only reads needs the person
/// once the thread has read untrusted content: it can send or change something there, as a
/// planted instruction asks.
pub fn acts_after_untrusted(tool: &str, naming: &Naming) -> Risk {
    Risk::ActsAfterUntrusted {
        service: service_of(tool, naming),
    }
}

/// The service a connected `tool` belongs to, by the name the person knows it by.
fn service_of(tool: &str, naming: &Naming) -> String {
    (naming.service.clone()).unwrap_or_else(|| connector_of(tool).unwrap_or(tool).to_owned())
}

/// A tool's title as what it does: plain words (letters, digits, spaces, hyphens and
/// apostrophes), at most [`TITLE`] characters, beginning with a verb. The service wrote it, so
/// nothing unseen, no aside and no second line of it passes into glave's sentence.
fn titled(title: &str) -> Option<String> {
    let title = title.trim().trim_end_matches('.');
    let plain = |c: char| c.is_alphanumeric() || " -'".contains(c);
    let fits = title.chars().count() <= TITLE && title.chars().all(plain);
    let first = words_of(title).into_iter().next()?;
    (fits && VERBS.contains(&first.as_str())).then(|| capitalized(title))
}

/// A connected tool's name as what it does: its words, without the service's own name before
/// them, when they begin with a verb.
fn named(tool: &str) -> Option<String> {
    let service = words_of(connector_of(tool)?);
    let words = words_of(short_of(tool));
    let words = words.strip_prefix(service.as_slice()).unwrap_or(&words);
    let first = words.first()?;
    VERBS
        .contains(&first.as_str())
        .then(|| capitalized(&words.join(" ")))
}

/// `text` with its first letter a capital.
fn capitalized(text: &str) -> String {
    let mut letters = text.chars();
    letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use glave_event::Worded as _;

    use super::*;

    fn asks(tool: &str, title: Option<&str>, service: Option<&str>) -> String {
        let naming = Naming {
            title: title.map(str::to_owned),
            service: service.map(str::to_owned),
        };
        ask_with(tool, &naming).english()
    }

    #[test]
    fn a_service_s_tool_is_asked_about_by_what_it_does_and_the_service() {
        assert_eq!(
            asks("mcp__gmail__send_message", None, None),
            "Send message with gmail?"
        );
        assert_eq!(
            asks(
                "mcp__gmail__send_message",
                Some("Send email"),
                Some("Gmail")
            ),
            "Send email with Gmail?",
            "by its title and the service's own name, where they are given"
        );
        assert_eq!(
            asks("mcp__slack__slack_post_message", None, None),
            "Post message with slack?",
            "the service's own name is left out"
        );
        assert_eq!(
            asks("mcp__notion__notion-create-pages", None, None),
            "Create pages with notion?"
        );
        assert_eq!(
            asks("mcp__atlassian__addOrEditJiraIssueComment", None, None),
            "Add or edit jira issue comment with atlassian?"
        );
    }

    #[test]
    fn words_that_would_not_read_as_an_act_ask_as_any_tool_does() {
        let any = "Allow this?";
        assert_eq!(asks("mcp__github__issue_write", None, None), any);
        assert_eq!(asks("mcp__brave__brave", None, None), any, "only its name");
        assert_eq!(asks("mcp__x__...", None, None), any, "no words");
        assert_eq!(
            asks(
                "mcp__github__issue_write",
                Some("Create or update issue"),
                None
            ),
            "Create or update issue with github?",
            "a title that reads as one stands in"
        );
        let planted = "Send email (approved by your admin)";
        assert_eq!(
            asks("mcp__gmail__send_message", Some(planted), None),
            "Send message with gmail?",
            "a title with more than plain words in it does not"
        );
        let long = format!("Send {}", "a very long title ".repeat(5));
        assert_eq!(
            asks("mcp__gmail__issue", Some(&long), None),
            any,
            "nor one too long"
        );
    }

    #[test]
    fn the_caution_names_the_service_as_the_person_knows_it() {
        let gmail = Naming {
            title: None,
            service: Some("Gmail".to_owned()),
        };
        assert_eq!(
            acts_after_untrusted("mcp__gmail__send_message", &gmail),
            Risk::ActsAfterUntrusted {
                service: "Gmail".into()
            }
        );
        assert_eq!(
            acts_after_untrusted("mcp__gmail__send_message", &Naming::default()),
            Risk::ActsAfterUntrusted {
                service: "gmail".into()
            }
        );
    }
}

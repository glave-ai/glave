//! A call of a service's tool, as approvals take it: named by what it does and the service, and
//! a caution once the thread has read untrusted content, when it does not say it only reads
//! (`specs/approvals.md`, "Untrusted content").

use glave_event::Risk;
use glave_ports::Effect;

use crate::toolset::{connector_of, short_of, words_of};

/// A service's tool as an approval of a call of it names it: what it does, in words, and the
/// service, by the name its tools carry, as a service that asks the person is named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Act {
    /// The tool's name in words, without the service's own name before it: `Send message`.
    pub act: String,
    /// `gmail`.
    pub service: String,
}

impl Act {
    /// What the approval asks, in English: "Send message with gmail?".
    pub fn asked(&self) -> String {
        format!("{} with {}?", self.act, self.service)
    }
}

/// What a call of `tool` does and with which service, when it is a service's tool
/// (`mcp__<service>__<tool>`): `Send message` with `gmail` of `mcp__gmail__send_message`, and of
/// `mcp__slack__slack_post_message`, `Post message` with `slack`.
pub fn act_of(tool: &str) -> Option<Act> {
    let service = connector_of(tool)?;
    let words = words_of(short_of(tool));
    let own = words_of(service);
    let act = match words.strip_prefix(own.as_slice()) {
        Some(rest) if !rest.is_empty() => rest,
        _ => words.as_slice(),
    };
    let act = act.join(" ");
    let mut letters = act.chars();
    let first = letters.next()?;
    Some(Act {
        act: first.to_uppercase().chain(letters).collect(),
        service: service.to_owned(),
    })
}

/// Why a call needs the person because the thread has read untrusted content and the call is of
/// a connected service's tool that does not say it only reads, which can send or change
/// something there as a planted instruction asks. None for any other call.
pub(crate) fn acts_after_untrusted(
    untrusted: bool,
    connected: bool,
    effect: Effect,
    tool: &str,
) -> Option<Risk> {
    (untrusted && connected && effect != Effect::Reads).then(|| Risk::ActsAfterUntrusted {
        service: connector_of(tool).unwrap_or(tool).to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_service_s_tool_is_asked_about_by_what_it_does_and_the_service() {
        let asks = |tool: &str| act_of(tool).map(|act| act.asked());
        assert_eq!(
            asks("mcp__gmail__send_message").as_deref(),
            Some("Send message with gmail?")
        );
        assert_eq!(
            asks("mcp__slack__slack_post_message").as_deref(),
            Some("Post message with slack?"),
            "the service's own name is left out"
        );
        assert_eq!(
            asks("mcp__notion__notion-create-pages").as_deref(),
            Some("Create pages with notion?")
        );
        assert_eq!(
            asks("mcp__atlassian__addOrEditJiraIssueComment").as_deref(),
            Some("Add or edit jira issue comment with atlassian?")
        );
        assert_eq!(
            asks("mcp__brave__brave").as_deref(),
            Some("Brave with brave?"),
            "a tool named only as its service keeps its name"
        );
        assert_eq!(asks("mcp__github"), None, "a connector's own tool");
        assert_eq!(asks("mcp__x__..."), None, "no words");
        assert_eq!(asks("web_fetch"), None);
    }

    #[test]
    fn only_a_connected_service_s_write_after_untrusted_content_is_a_caution() {
        let tool = "mcp__gmail__send_message";
        let gmail = Risk::ActsAfterUntrusted {
            service: "gmail".into(),
        };
        assert_eq!(
            acts_after_untrusted(true, true, Effect::Runs, tool),
            Some(gmail)
        );
        let reads = acts_after_untrusted(true, true, Effect::Reads, tool);
        assert_eq!(reads, None, "a read");
        let before = acts_after_untrusted(false, true, Effect::Runs, tool);
        assert_eq!(before, None, "before untrusted content");
        let own = acts_after_untrusted(true, false, Effect::Runs, "web_fetch");
        assert_eq!(own, None, "glave's own tools, and the web search");
    }
}

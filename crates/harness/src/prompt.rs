//! The default system prompt: short and plain, so it fits small models and leaves them room.
//! And what is added to any prompt about the accounts the person connected.

use glave_ports::{CredentialHost, ForgeTool};

const RULES: &str = "\
Rules:
- To find code, use search (file contents) and glob (file names), not bash.
- Read a file before you change it. Use edit for changes to existing files, write for new files.
- Make the smallest change that does what was asked. Do not change anything else.
- When you can check your work (run the tests, build, run the program), do it.
- Bash runs each command fresh in the project root: cd does not carry over.
- If a tool fails, read the error and try a different approach. Do not repeat the same call.
- If something is unclear or risky, ask the person instead of guessing.
- Finish with a short, plain summary: what you changed, how you checked it, and anything left \
undone.";

/// The default instructions for the coding agent. glave runs any model, so the prompt names
/// the one actually answering and never claims to be another.
pub fn system_prompt(model: &str, provider: &str) -> String {
    format!(
        "You are the AI model {model}, served by {provider}, working through glave in the \
         person's project folder. If asked which model you are, say so plainly. You can search \
         the project, find files, read, write and edit files, and run bash commands.\n\n{RULES}"
    )
}

/// What the model is told of the git hosts the person connected (`specs/computers.md`, rule
/// 8): that git signs in to them by itself and which forge tools run signed in, so it pushes
/// and uses them without looking for a token. Never a token; `None` with no host connected,
/// so nothing the model sees changes.
pub(crate) fn signed_in(hosts: &[CredentialHost]) -> Option<String> {
    if hosts.is_empty() {
        return None;
    }
    let names: Vec<String> = hosts.iter().map(|host| bare(&host.url)).collect();
    let mut text = format!(
        "Accounts the person connected: git over HTTPS signs in to {} by itself",
        listed(&names)
    );
    let tools: Vec<String> = ForgeTool::signed_in(hosts)
        .into_iter()
        .map(|(tool, host)| format!("`{}` is signed in for {}", tool.name(), bare(&host.url)))
        .collect();
    if !tools.is_empty() {
        text.push_str(", and ");
        text.push_str(&listed(&tools));
    }
    text.push_str(
        ". Use them as they are: the tokens are hidden from you, so never look for or print one.",
    );
    Some(text)
}

/// A host's URL without its scheme: `github.com`.
fn bare(url: &str) -> String {
    url.split_once("://")
        .map_or(url, |(_, rest)| rest)
        .to_owned()
}

/// `items` as a sentence lists them: `a`, `a and b`, `a, b and c`.
fn listed(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use glave_ports::Forge;

    use super::*;

    #[test]
    fn the_model_is_told_what_is_signed_in_and_never_a_token() {
        assert_eq!(signed_in(&[]), None);
        let host = |url: &str, forge| CredentialHost {
            url: url.into(),
            forge,
        };
        let told = signed_in(&[
            host("https://github.com", Forge::Github),
            host("https://git.example.com", Forge::Other),
            host("https://gitlab.example.com", Forge::Gitlab),
        ])
        .unwrap();
        assert_eq!(
            told,
            "Accounts the person connected: git over HTTPS signs in to github.com, \
             git.example.com and gitlab.example.com by itself, and `gh` is signed in for \
             github.com and `glab` is signed in for gitlab.example.com. Use them as they are: \
             the tokens are hidden from you, so never look for or print one."
        );
    }

    #[test]
    fn the_prompt_names_the_real_model() {
        let prompt = system_prompt("anthropic/claude-haiku-5.5", "openrouter");
        assert!(
            prompt.starts_with(
                "You are the AI model anthropic/claude-haiku-5.5, served by openrouter,"
            )
        );
        assert!(!prompt.to_lowercase().contains("hope"));
    }
}

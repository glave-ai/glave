//! MCP servers as configured: the `.mcp.json` format Claude Code made the common one, read
//! from `{"mcpServers": {"name": {...}}}`. `${VAR}` and `${VAR:-default}` in a server's command,
//! arguments, environment, URL and headers are filled from the environment.

use std::collections::BTreeMap;

use serde::Deserialize;
use thiserror::Error;

/// Why a `.mcp.json` file's text cannot be read as servers.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("MCP settings: {0}")]
pub struct McpSettingsError(pub String);

/// How to reach one server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerConfig {
    /// A program glave starts, speaking MCP on its standard input and output.
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    },
    /// A server at a URL, over Streamable HTTP.
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
}

#[derive(Deserialize)]
struct File {
    #[serde(rename = "mcpServers", default)]
    servers: BTreeMap<String, Raw>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    #[serde(rename = "type")]
    kind: Option<String>,
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    url: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    /// glave's own key: whether what the server's tools return is untrusted, as web content
    /// is. Unset, whoever reads the file decides (see `ServerConfig::is_remote`).
    untrusted: Option<bool>,
}

/// The servers in a `.mcp.json` file's text that say `"untrusted"`, with what they say:
/// `true` when what their tools return comes from outside and no one vouches for it, as with
/// a web page; `false` when the file's author vouches for it.
pub fn marks(text: &str) -> Result<BTreeMap<String, bool>, McpSettingsError> {
    let settings: File =
        serde_json::from_str(text).map_err(|err| McpSettingsError(err.to_string()))?;
    Ok(settings
        .servers
        .into_iter()
        .filter_map(|(name, raw)| Some((name, raw.untrusted?)))
        .collect())
}

impl ServerConfig {
    /// Whether the server is reached across a network, rather than started on this computer.
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Http { .. })
    }
}

/// Reads the servers in a `.mcp.json` file's text, filling `${VAR}` from `environment`.
pub fn parse(
    text: &str,
    environment: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, ServerConfig>, McpSettingsError> {
    let settings: File =
        serde_json::from_str(text).map_err(|err| McpSettingsError(err.to_string()))?;
    let fill = |text: &str| expand(text, environment);
    let mut servers = BTreeMap::new();
    for (name, raw) in settings.servers {
        let bad = |message: &str| McpSettingsError(format!("server {name:?}: {message}"));
        let config = match (raw.kind.as_deref(), raw.command, raw.url) {
            (None | Some("stdio"), Some(command), None) => ServerConfig::Stdio {
                command: fill(&command),
                args: raw.args.iter().map(|arg| fill(arg)).collect(),
                env: raw.env.iter().map(|(k, v)| (k.clone(), fill(v))).collect(),
            },
            (None | Some("http" | "streamable-http"), None, Some(url)) => ServerConfig::Http {
                url: fill(&url),
                headers: raw
                    .headers
                    .iter()
                    .map(|(k, v)| (k.clone(), fill(v)))
                    .collect(),
            },
            (Some("sse"), ..) => {
                return Err(bad(
                    "the old SSE transport is retired; use \"type\": \"http\"",
                ));
            }
            (_, Some(_), Some(_)) => return Err(bad("give either a command or a url, not both")),
            (_, None, None) => return Err(bad("give a command to run or a url")),
            (Some(other), ..) => return Err(bad(&format!("unknown type {other:?}"))),
        };
        servers.insert(name, config);
    }
    Ok(servers)
}

/// `text` with every `${VAR}` and `${VAR:-default}` filled in; an unset variable without a
/// default becomes empty.
fn expand(text: &str, environment: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(rest.get(..start).unwrap_or_default());
        let after = rest.get(start + 2..).unwrap_or_default();
        let Some(end) = after.find('}') else {
            out.push_str(rest.get(start..).unwrap_or_default());
            return out;
        };
        let inner = after.get(..end).unwrap_or_default();
        let (name, default) = inner.split_once(":-").unwrap_or((inner, ""));
        match environment.get(name).filter(|value| !value.is_empty()) {
            Some(value) => out.push_str(value),
            None => out.push_str(default),
        }
        rest = after.get(end + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> BTreeMap<String, String> {
        BTreeMap::from([("TOKEN".to_owned(), "t0k".to_owned())])
    }

    #[test]
    fn stdio_and_http_servers_read_with_variables_filled() {
        let servers = parse(
            r#"{"mcpServers": {
                "docs": {"command": "docs-mcp", "args": ["--root", "${ROOT:-.}"], "env": {"KEY": "${TOKEN}"}},
                "tracker": {"type": "http", "url": "https://mcp.example.com/mcp", "headers": {"Authorization": "Bearer ${TOKEN}"}}
            }}"#,
            &env(),
        )
        .unwrap();
        assert_eq!(
            servers["docs"],
            ServerConfig::Stdio {
                command: "docs-mcp".into(),
                args: vec!["--root".into(), ".".into()],
                env: BTreeMap::from([("KEY".into(), "t0k".into())]),
            }
        );
        assert!(
            matches!(&servers["tracker"], ServerConfig::Http { headers, .. }
            if headers["Authorization"] == "Bearer t0k")
        );
    }

    #[test]
    fn mistakes_are_named() {
        let error = |text: &str| parse(text, &env()).unwrap_err().to_string();
        assert!(error(r#"{"mcpServers": {"a": {}}}"#).contains("give a command"));
        assert!(
            error(r#"{"mcpServers": {"a": {"command": "x", "url": "y"}}}"#).contains("not both")
        );
        assert!(error(r#"{"mcpServers": {"a": {"type": "sse", "url": "y"}}}"#).contains("retired"));
        assert!(
            error(r#"{"mcpServers": {"a": {"command": "x", "cwd": "/"}}}"#)
                .contains("unknown field")
        );
    }

    #[test]
    fn variables_expand_and_unknown_ones_go_empty() {
        let env = env();
        assert_eq!(expand("a ${TOKEN} b", &env), "a t0k b");
        assert_eq!(expand("${NOPE:-fallback}", &env), "fallback");
        assert_eq!(expand("${NOPE}", &env), "");
        assert_eq!(expand("unclosed ${TOKEN", &env), "unclosed ${TOKEN");
    }

    #[test]
    fn servers_can_be_marked_untrusted_or_trusted() {
        let text = r#"{"mcpServers": {
            "search": {"command": "brave-mcp", "untrusted": true},
            "docs": {"command": "docs-mcp", "untrusted": false},
            "files": {"command": "files-mcp"}
        }}"#;
        assert_eq!(parse(text, &env()).unwrap().len(), 3);
        assert_eq!(
            marks(text).unwrap(),
            BTreeMap::from([("docs".to_owned(), false), ("search".to_owned(), true)])
        );
        let servers = parse(
            r#"{"mcpServers": {"tracker": {"url": "https://x"}}}"#,
            &env(),
        );
        assert!(servers.unwrap()["tracker"].is_remote());
        assert!(!parse(text, &env()).unwrap()["docs"].is_remote());
    }
}

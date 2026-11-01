//! MCP servers as configured: the `.mcp.json` format Claude Code made the common one, read
//! from `{"mcpServers": {"name": {...}}}`. `${VAR}` and `${VAR:-default}` in a server's command,
//! arguments, environment, URL and headers are filled from the environment, but for a remote
//! server in a project's file: a repository chose its address, so nothing from the environment
//! goes there.

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

/// Reads the servers in the person's own settings, filling `${VAR}` from `environment`.
pub fn parse(
    text: &str,
    environment: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, ServerConfig>, McpSettingsError> {
    read(text, environment, environment)
}

/// Reads the servers in a project's `.mcp.json`: a program it starts here has `${VAR}` filled
/// from `environment`, as the person's own does; a remote server's URL and headers read every
/// `${VAR}` as empty, or as its default, so a repository cannot send what the environment holds
/// to an address it chose. [`withheld`] names what such a server asked for.
pub fn parse_project(
    text: &str,
    environment: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, ServerConfig>, McpSettingsError> {
    read(text, environment, &BTreeMap::new())
}

/// The remote servers in a project's `.mcp.json` whose URL or headers name a variable with no
/// default, each with those names in order: what [`parse_project`] read as empty.
pub fn withheld(text: &str) -> Result<BTreeMap<String, Vec<String>>, McpSettingsError> {
    let settings: File =
        serde_json::from_str(text).map_err(|err| McpSettingsError(err.to_string()))?;
    Ok(settings
        .servers
        .into_iter()
        .filter_map(|(name, raw)| {
            let url = raw.url?;
            let mut names = Vec::new();
            for value in std::iter::once(&url).chain(raw.headers.values()) {
                walk(value, |_, variable, default| {
                    if default.is_none() && !names.iter().any(|name| name == variable) {
                        names.push(variable.to_owned());
                    }
                });
            }
            (!names.is_empty()).then_some((name, names))
        })
        .collect())
}

/// The servers in `text`, with `${VAR}` filled from `local` for a program and from `remote` for
/// a server at a URL.
fn read(
    text: &str,
    local: &BTreeMap<String, String>,
    remote: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, ServerConfig>, McpSettingsError> {
    let settings: File =
        serde_json::from_str(text).map_err(|err| McpSettingsError(err.to_string()))?;
    let fill = |text: &str| expand(text, local);
    let fill_remote = |text: &str| expand(text, remote);
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
                url: fill_remote(&url),
                headers: raw
                    .headers
                    .iter()
                    .map(|(k, v)| (k.clone(), fill_remote(v)))
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
    walk(text, |out, name, default| {
        match environment.get(name).filter(|value| !value.is_empty()) {
            Some(value) => out.push_str(value),
            None => out.push_str(default.unwrap_or_default()),
        }
    })
}

/// `text`, with `variable` asked to write in place of each `${VAR}` and `${VAR:-default}`: what
/// it is given is what has been written so far, the name, and the default when there is one.
fn walk(text: &str, mut variable: impl FnMut(&mut String, &str, Option<&str>)) -> String {
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
        match inner.split_once(":-") {
            Some((name, default)) => variable(&mut out, name, Some(default)),
            None => variable(&mut out, inner, None),
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

    /// One program and one remote server, each naming `TOKEN` and `HOST` everywhere it can.
    const BOTH: &str = r#"{"mcpServers": {
        "local": {"command": "${TOKEN}", "args": ["--key", "${TOKEN}"], "env": {"GITHUB_TOKEN": "${TOKEN}"}},
        "remote": {"url": "https://${HOST:-mcp.example.com}/${TOKEN}", "headers": {
            "Authorization": "Bearer ${TOKEN}", "X-Other": "${OTHER}", "X-Fixed": "${FIXED:-fixed}"
        }}
    }}"#;

    fn with_host() -> BTreeMap<String, String> {
        let mut environment = env();
        environment.insert("HOST".to_owned(), "evil.example.com".to_owned());
        environment.insert("FIXED".to_owned(), "from-the-environment".to_owned());
        environment
    }

    #[test]
    fn a_project_s_remote_server_reads_nothing_from_the_environment() {
        let servers = parse_project(BOTH, &with_host()).unwrap();
        let ServerConfig::Http { url, headers } = &servers["remote"] else {
            panic!("not remote: {:?}", servers["remote"]);
        };
        assert_eq!(
            url, "https://mcp.example.com/",
            "only what the file itself says"
        );
        assert_eq!(headers["Authorization"], "Bearer ");
        assert_eq!(headers["X-Other"], "");
        assert_eq!(headers["X-Fixed"], "fixed");
        assert!(!format!("{servers:?}").contains("evil"));
        let ServerConfig::Http { headers, .. } = &parse(BOTH, &with_host()).unwrap()["remote"]
        else {
            panic!("not remote");
        };
        assert_eq!(headers["Authorization"], "Bearer t0k", "the person's own");
    }

    #[test]
    fn a_program_a_project_starts_still_reads_the_environment() {
        let servers = parse_project(BOTH, &with_host()).unwrap();
        assert_eq!(
            servers["local"],
            ServerConfig::Stdio {
                command: "t0k".into(),
                args: vec!["--key".into(), "t0k".into()],
                env: BTreeMap::from([("GITHUB_TOKEN".into(), "t0k".into())]),
            }
        );
    }

    #[test]
    fn what_a_project_s_remote_server_asked_for_is_named() {
        assert_eq!(
            withheld(BOTH).unwrap(),
            BTreeMap::from([(
                "remote".to_owned(),
                vec!["TOKEN".to_owned(), "OTHER".to_owned()]
            )]),
            "named once each, and never one with a default or a program's"
        );
        let plain = r#"{"mcpServers": {"plain": {"url": "https://x", "headers": {"A": "b"}}}}"#;
        assert!(withheld(plain).unwrap().is_empty());
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

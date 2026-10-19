//! A forge tool's wrapper: a small script first on the command's `PATH` that asks glave's
//! helper for its host's token at that moment, sets it in the tool's own environment only, and
//! runs the real tool. Before that it puts back what a command could set to have the tool send
//! the token elsewhere or run a program of its own: another host, its configuration and
//! extensions (pointed at an empty folder glave owns), its pager, editor and browser, and the
//! certificates it trusts (as glave's own environment has them).

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::Path;

use glave_ports::computer::{CredentialHost, ForgeTool};

/// Where a wrapper's pieces are.
pub(super) struct Places<'a> {
    /// The wrappers' folder, first on `PATH`.
    pub(super) bin: &'a Path,
    /// glave's credential helper.
    pub(super) helper: &'a Path,
    /// An empty folder the tool may read and not write: its configuration and extensions.
    pub(super) empty: &'a Path,
}

/// Certificate settings a command could point at a store of its own, to read the token
/// through a proxy it runs.
const TRUST: &[&str] = &["SSL_CERT_FILE", "SSL_CERT_DIR"];

/// The wrapper for `tool`, signed in to `host`, where `environment` is glave's own environment
/// for the command, before the command's script changes it.
pub(super) fn wrapper(
    tool: ForgeTool,
    host: &CredentialHost,
    places: &Places<'_>,
    environment: &[(OsString, OsString)],
) -> String {
    let name = tool.name();
    let (protocol, address) = host.origin().unwrap_or(("https", "invalid"));
    let mut script = format!(
        "#!/bin/sh\n\
         # {name}, signed in by glave for this one process: its token is asked for now and set\n\
         # only in {name}'s own environment.\n\
         wrappers={bin}\n\
         path=\nIFS=:\nset -f\n\
         for dir in $PATH; do [ \"$dir\" = \"$wrappers\" ] || path=\"${{path:+$path:}}$dir\"; done\n\
         set +f\nunset IFS\nPATH=$path\n\
         command -v {name} >/dev/null 2>&1 || {{ echo \"{name}: command not found\" >&2; exit 127; }}\n",
        bin = quoted(&places.bin.to_string_lossy()),
    );
    script.push_str(elsewhere(tool));
    let _ = write!(
        script,
        "token=\n\
         if [ -z \"$elsewhere\" ]; then\n\
         \x20 token=$(printf 'protocol=%s\\nhost=%s\\n' {protocol} {address} | {helper} get | \
         sed -n 's/^password=//p')\n\
         fi\n\
         unset elsewhere {cleared}\n",
        protocol = quoted(protocol),
        address = quoted(address),
        helper = quoted(&places.helper.to_string_lossy()),
        cleared = cleared(tool).join(" "),
    );
    for (variable, value) in fixed(tool, &host.url, places.empty) {
        let _ = writeln!(script, "{variable}={}; export {variable}", quoted(&value));
    }
    for variable in TRUST {
        let kept = environment
            .iter()
            .find(|(name, _)| name == variable)
            .map(|(_, value)| value.to_string_lossy().into_owned());
        let _ = match kept {
            Some(value) => writeln!(script, "{variable}={}; export {variable}", quoted(&value)),
            None => writeln!(script, "unset {variable}"),
        };
    }
    let token = match tool {
        ForgeTool::Gh => "GH_TOKEN",
        ForgeTool::Tea => "GITEA_TOKEN",
    };
    let _ = write!(
        script,
        "if [ -n \"$token\" ]; then {token}=$token; export {token}; fi\n\
         unset token\nexec {name} \"$@\"\n"
    );
    script
}

/// Sets `elsewhere` when the tool may be pointed at a host other than its own, where it runs
/// without the token. gh sends `GH_TOKEN` to github.com, `github.localhost` and GitHub's
/// `*.ghe.com` hosts alone (go-gh's `tokenForHost`), so any argument naming the last two, in a
/// flag, a URL or `HOST/OWNER/REPO`, does; one with a project in it is text, such as a title.
fn elsewhere(tool: ForgeTool) -> &'static str {
    match tool {
        ForgeTool::Gh => {
            "elsewhere=\n\
             for arg in \"$@\"; do\n\
             \x20 case $arg in *[[:space:]]*) ;; \
             *[Ll][Oo][Cc][Aa][Ll][Hh][Oo][Ss][Tt]*|*[Gg][Hh][Ee].[Cc][Oo][Mm]*) elsewhere=1;; esac\n\
             done\n"
        }
        // tea talks only to the instance it is given below.
        ForgeTool::Tea => "elsewhere=\n",
    }
}

/// What each tool is given besides its token, after the command's own: the host it signs in
/// to, its configuration in an empty folder, and a pager that only shows.
fn fixed(tool: ForgeTool, url: &str, empty: &Path) -> Vec<(&'static str, String)> {
    let empty = empty.to_string_lossy().into_owned();
    match tool {
        ForgeTool::Gh => vec![
            ("GH_CONFIG_DIR", empty.clone()),
            ("XDG_DATA_HOME", empty),
            ("GH_NO_UPDATE_NOTIFIER", "1".into()),
            ("GH_PROMPT_DISABLED", "1".into()),
            ("GH_PAGER", "cat".into()),
            ("PAGER", "cat".into()),
        ],
        ForgeTool::Tea => vec![
            ("XDG_CONFIG_HOME", empty),
            ("GITEA_INSTANCE_URL", url.to_owned()),
        ],
    }
}

/// What a command could set to have the tool send its token elsewhere or run a program of its
/// own: cleared for the tool's process.
fn cleared(tool: ForgeTool) -> &'static [&'static str] {
    match tool {
        ForgeTool::Gh => &[
            "GH_ENTERPRISE_TOKEN",
            "GITHUB_ENTERPRISE_TOKEN",
            "GITHUB_TOKEN",
            "GH_HOST",
            "GH_REPO",
            "GH_FORCE_TTY",
            "GH_BROWSER",
            "BROWSER",
            "GH_EDITOR",
            "EDITOR",
            "VISUAL",
            "GH_DEBUG",
            "DEBUG",
        ],
        ForgeTool::Tea => &["GITEA_INSTANCE_INSECURE", "BROWSER", "EDITOR", "VISUAL"],
    }
}

/// `text` in single quotes, for a shell.
pub(super) fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

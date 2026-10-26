//! Commands that print a credential, or could: a token, a password or a key, from a credential
//! helper (glave's own, or one a command gives git), a forge's or cloud's command-line tool,
//! the keychain or a password manager. What they print would reach the model, so they are
//! secrets cautions: no judge clears them, with no one there they are not run, and Full
//! access, which asks no one, declines them.

use crate::judge::{Caution, Kind};
use crate::parse::Word;

/// Why `words` (a command after any wrapper, named `name`) prints a credential, if it does.
pub(crate) fn prints_credential(name: &str, words: &[Word]) -> Option<Caution> {
    let args: Vec<&str> = words.iter().skip(1).map(Word::text).collect();
    let helper = |word: &Word| GLAVE_HELPER.iter().any(|part| word.text().contains(part));
    if words.iter().any(helper) {
        return Some(Caution {
            kind: Kind::Secrets,
            reason: "it reaches glave's credential helper, which hands out tokens".to_owned(),
        });
    }
    if name == "git" && configures_credentials(&args) {
        return Some(Caution {
            kind: Kind::Secrets,
            reason: "it changes how git keeps credentials, which could show one".to_owned(),
        });
    }
    let plain: Vec<&str> = args
        .iter()
        .copied()
        .filter(|arg| !arg.starts_with('-'))
        .collect();
    let has = |flag: &str| args.contains(&flag);
    let first = plain.first().copied();
    let second = plain.get(1).copied();
    let what = match name {
        "git" => git(&args)?,
        _ if name.starts_with("git-credential-") => "git's stored credentials",
        "gh" if first == Some("auth") && shows_token(second, &args) => "a GitHub token",
        "glab" if first == Some("auth") && shows_token(second, &args) => "a GitLab token",
        "security"
            if matches!(
                first,
                Some("find-generic-password" | "find-internet-password")
            ) && args.iter().any(|arg| short(arg, 'w') || short(arg, 'g')) =>
        {
            "a password from the keychain"
        }
        "security" if first == Some("dump-keychain") && args.iter().any(|arg| short(arg, 'd')) => {
            "the keychain's passwords"
        }
        "gcloud" if plain.contains(&"auth") && plain.iter().any(|word| prints_token(word)) => {
            "a Google Cloud token"
        }
        "az" if first == Some("account") && second == Some("get-access-token") => "an Azure token",
        "op" if first == Some("read") || (first == Some("item") && second == Some("get")) => {
            "a secret from 1Password"
        }
        "kubectl" | "oc" if first == Some("config") && second == Some("view") && has("--raw") => {
            "a Kubernetes cluster's credentials"
        }
        "aws"
            if first == Some("configure")
                && matches!(second, Some("get" | "export-credentials")) =>
        {
            "AWS credentials"
        }
        "aws" if first == Some("ecr") && second == Some("get-login-password") => {
            "an AWS registry password"
        }
        "vault" if first == Some("read") || (first == Some("kv") && second == Some("get")) => {
            "a secret from Vault"
        }
        _ => return None,
    };
    Some(Caution {
        kind: Kind::Secrets,
        reason: format!("it prints {what}, which the model would see"),
    })
}

/// `git credential fill` and `git credential-<helper> …`, after git's own options.
fn git(args: &[&str]) -> Option<&'static str> {
    let mut at = 0;
    while let Some(arg) = args.get(at) {
        if matches!(
            *arg,
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace"
        ) {
            at = at.saturating_add(2);
        } else if arg.starts_with('-') {
            at = at.saturating_add(1);
        } else {
            break;
        }
    }
    let subcommand = args.get(at)?;
    let fill = *subcommand == "credential" && args.get(at.saturating_add(1)) == Some(&"fill");
    (fill || subcommand.starts_with("credential-")).then_some("git's stored credentials")
}

/// What names glave's own credential helper, the folder it is in, or git's setting that
/// holds its path, for one command (`specs/computers.md`, "Credentials").
const GLAVE_HELPER: &[&str] = &[
    "git-credential-glave",
    "glave-credentials-",
    "GIT_CONFIG_VALUE_",
];

/// `git -c credential.…` or `--config-env=credential.…`, among git's own options: a helper of
/// the command's own (`store`, `cache`) would keep, and could print, what glave's hands git.
fn configures_credentials(args: &[&str]) -> bool {
    let credential = |setting: &str| setting.to_ascii_lowercase().starts_with("credential.");
    let mut at = 0;
    while let Some(arg) = args.get(at) {
        if *arg == "-c" {
            if args
                .get(at.saturating_add(1))
                .is_some_and(|value| credential(value))
            {
                return true;
            }
            at = at.saturating_add(2);
        } else if *arg == "--config-env" {
            if args
                .get(at.saturating_add(1))
                .is_some_and(|value| credential(value))
            {
                return true;
            }
            at = at.saturating_add(2);
        } else if let Some(value) = arg.strip_prefix("--config-env=") {
            if credential(value) {
                return true;
            }
            at = at.saturating_add(1);
        } else if matches!(*arg, "-C" | "--git-dir" | "--work-tree" | "--namespace") {
            at = at.saturating_add(2);
        } else if arg.starts_with('-') {
            at = at.saturating_add(1);
        } else {
            return false;
        }
    }
    false
}

/// `auth token`, and `auth status --show-token` (or `-t`), of `gh` and `glab`.
fn shows_token(verb: Option<&str>, args: &[&str]) -> bool {
    verb == Some("token")
        || (verb == Some("status")
            && args
                .iter()
                .any(|arg| *arg == "--show-token" || short(arg, 't')))
}

/// `print-access-token`, `print-identity-token` and the like.
fn prints_token(word: &str) -> bool {
    word.starts_with("print-") && word.ends_with("-token")
}

/// A short flag group such as `-gw` holding `letter`.
fn short(arg: &str, letter: char) -> bool {
    arg.starts_with('-') && !arg.starts_with("--") && arg.contains(letter)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::judge::{Context, Kind, Verdict, judge};

    fn verdict(script: &str) -> Verdict {
        let context = Context {
            project: Path::new("/home/ana/app"),
            home: Some(Path::new("/home/ana")),
            temp: None,
            secret: &|_| false,
        };
        judge(script, &context)
    }

    #[test]
    fn commands_that_print_credentials_are_firm_secrets_cautions() {
        for script in [
            "printf 'protocol=https\\nhost=github.com\\n' | git credential fill",
            "git -C . credential fill",
            "git credential-osxkeychain get",
            "git-credential-store get",
            "gh auth token",
            "gh auth status --show-token",
            "glab auth status -t",
            "GH_HOST=github.com gh auth token --hostname github.com",
            "echo $(gh auth token)",
            "security find-generic-password -s github -w",
            "security find-internet-password -gs github.com",
            "security dump-keychain -d",
            "gcloud auth print-access-token",
            "gcloud auth application-default print-access-token",
            "gcloud auth print-identity-token",
            "az account get-access-token",
            "op read op://vault/item/password",
            "op item get GitHub --fields password",
            "kubectl config view --raw",
            "aws configure get aws_secret_access_key",
            "aws configure export-credentials --format env",
            "aws ecr get-login-password",
            "sudo -u ana gh auth token",
            "bash -c 'gh auth token'",
        ] {
            // Firm even behind `sudo`, whose own caution a judge may clear.
            match verdict(script) {
                Verdict::Caution(caution) if caution.kind == Kind::Secrets => {
                    assert!(caution.kind.firm(), "{script}");
                    assert!(caution.reason.starts_with("it prints"), "{script}");
                }
                other => panic!("{script}: {other:?}"),
            }
        }
    }

    #[test]
    fn git_given_a_credential_helper_of_the_command_s_own_is_a_secret() {
        for script in [
            "git -c credential.helper=store push",
            "git -c Credential.https://github.com.helper='store --file=x' fetch",
            "git --config-env=credential.helper=HELPER push",
            "git --config-env credential.helper=HELPER push",
            "printf 'host=github.com\\n' | /tmp/glave-credentials-1/git-credential-glave get",
            "cat /tmp/glave-credentials-*/git-credential-glave",
            "echo \"${GIT_CONFIG_VALUE_1#!}\"",
            "git -C . -c credential.helper=cache push",
        ] {
            match verdict(script) {
                Verdict::Caution(caution) if caution.kind == Kind::Secrets => {
                    assert!(caution.kind.firm(), "{script}");
                }
                other => panic!("{script}: {other:?}"),
            }
        }
        for script in ["git -c user.name=me commit -m x", "git push origin main"] {
            assert!(
                !matches!(verdict(script), Verdict::Caution(ref c) if c.kind == Kind::Secrets),
                "{script}"
            );
        }
    }

    #[test]
    fn commands_that_do_not_print_credentials_are_left_alone() {
        for script in [
            "gh auth status",
            "gh pr list",
            "git status",
            "git config --get credential.helper",
            "git credential approve",
            "security find-generic-password -s github",
            "gcloud auth list",
            "az account show",
            "op item list",
            "kubectl config view",
            "aws configure list",
            "echo gh auth token",
        ] {
            assert!(
                !matches!(verdict(script), Verdict::Caution(ref c) if c.kind == Kind::Secrets),
                "{script}"
            );
        }
    }
}

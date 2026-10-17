//! Credentials asked for at each use (`credentials/request`, `specs/computers.md` rule 8): a
//! command's git gets a connected host's credential from the caller through the computer's
//! helper, a forge's tool gets its token in its own environment only, and the command's
//! environment never holds it.

use super::helpers::{Collect, Trip, failed};
use super::{Verdict, ensure};
use crate::computer::{
    Command, Computers, CredentialHost, Ending, Forge, GitCredential, WorkspaceId,
};
use crate::testing::{FAKE_COMMIT, FakeGitHost, FixedCredentials, fake_secret};

/// git with nothing but the command's own settings, and never asking at a terminal.
const PLAIN_GIT: &str =
    "export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_TERMINAL_PROMPT=0";

pub(super) async fn credentials<C: Computers>(computers: &C, here: WorkspaceId) -> Verdict {
    // The token every check signs in with; it must never show but where it is asked for.
    let token = fake_secret("ghp_");
    let host = FakeGitHost::start("x-access-token", &token)
        .map_err(|error| format!("start a fake git host: {error}"))?;
    let credential =
        GitCredential::new("x-access-token", &token).map_err(failed("a credential"))?;
    let hosts = vec![
        CredentialHost {
            url: host.url.clone(),
            forge: Forge::Other,
        },
        CredentialHost {
            url: "https://github.com".into(),
            forge: Forge::Github,
        },
    ];
    let fixed = FixedCredentials::new(hosts, credential);
    let fetch = format!("{PLAIN_GIT}\ngit ls-remote {}/repo.git", host.url);

    let (ending, printed) = run(computers, here, &fetch, None).await?;
    ensure(
        ending != (Ending::Exited { code: 0 }) && host.refused() > 0,
        "without credentials, a host that needs a sign-in refuses git",
    )?;
    let (ending, printed_with) = run(computers, here, &fetch, Some(&fixed)).await?;
    ensure(
        ending == (Ending::Exited { code: 0 })
            && printed_with.contains(&format!("{FAKE_COMMIT}\trefs/heads/main")),
        &format!("git signs in with the credential it asks for: {printed} / {printed_with}"),
    )?;
    let asked = fixed.asked();
    let address = host.url.trim_start_matches("http://");
    ensure(
        asked
            .iter()
            .any(|ask| ask.protocol == "http" && ask.host == address),
        "the computer asks the caller for the host git is going to",
    )?;

    // A stand-in for gh that shows the token it got, in capitals, so the token itself never
    // shows; on `PATH` right after the wrappers' folder, which comes first, so it is found
    // before any gh installed here, as the real one would be.
    let forge = "mkdir -p tools\ncat > tools/gh <<'FAKE'\n#!/bin/sh\n\
        printf 'gh sees: %s\\n' \"$(printf '%s' \"${GH_TOKEN:-unset}\" | tr a-z A-Z)\"\nFAKE\n\
        chmod +x tools/gh\nPATH=\"${PATH%%:*}:$PWD/tools:${PATH#*:}\"\ngh pr list\n\
        echo \"outside: ${GH_TOKEN:-unset}\"\nenv\nset\nrm -rf tools";
    let (_, printed) = run(computers, here, forge, Some(&fixed)).await?;
    let seen = format!("gh sees: {}", token.to_ascii_uppercase());
    ensure(
        printed.contains(&seen) && printed.contains("outside: unset"),
        &format!("gh gets its token for its own process only: {printed}"),
    )?;
    ensure(
        !printed.contains(&token) && !printed_with.contains(&token),
        "a token never shows in a command's environment or output",
    )
}

/// Runs `script` to its end, with `credentials` when given.
async fn run<C: Computers>(
    computers: &C,
    here: WorkspaceId,
    script: &str,
    credentials: Option<&FixedCredentials>,
) -> Result<(Ending, String), String> {
    let command = Command::new(script);
    let output = Collect::new();
    let never = Trip::default();
    let finished = match credentials {
        Some(credentials) => {
            computers
                .run_with_credentials(here, &command, &output, &never, credentials)
                .await
        }
        None => computers.run(here, &command, &output, &never).await,
    }
    .map_err(failed("run a command with credentials"))?;
    Ok((finished.ending, output.text()))
}

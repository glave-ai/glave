//! Who you are to glave on this computer: a person id and your default agent's id, made on
//! first run and kept in glave's folder, so every thread here records the same `for` and
//! `agent` (see "Who acted" in `specs/events.md`). And which computer this is: an id made the
//! same way, so a project's checkout and a thread's binding name this computer the same way
//! every run.

use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context as _, bail};
use glave_event::PrincipalId;
use glave_harness::{Clock as _, Entropy as _, OsEntropy, SystemClock};
use glave_ports::ComputerId;
use uuid::Uuid;

/// The file, in glave's folder.
const FILE: &str = "identity";

/// The file holding this computer's id, in glave's folder.
const COMPUTER: &str = "computer";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    pub person: PrincipalId,
    pub agent: PrincipalId,
}

/// Your identity from glave's folder, made and saved there the first time.
pub async fn load_or_make(home: &Path) -> anyhow::Result<Identity> {
    let path = home.join(FILE);
    match tokio::fs::read_to_string(&path).await {
        Ok(text) => parse(&text).with_context(|| {
            format!(
                "{} is damaged; move it aside to start a new identity",
                path.display()
            )
        }),
        Err(err) if err.kind() == ErrorKind::NotFound => {
            let identity = Identity {
                person: make()?,
                agent: make()?,
            };
            tokio::fs::write(&path, render(&identity))
                .await
                .with_context(|| format!("could not write {}", path.display()))?;
            Ok(identity)
        }
        Err(err) => Err(err).with_context(|| format!("could not read {}", path.display())),
    }
}

/// This computer's id from glave's folder, made and saved there the first time.
pub async fn computer(home: &Path) -> anyhow::Result<ComputerId> {
    let path = home.join(COMPUTER);
    match tokio::fs::read_to_string(&path).await {
        Ok(text) => Uuid::parse_str(text.trim())
            .map(ComputerId::from_uuid)
            .with_context(|| {
                format!(
                    "{} is damaged; move it aside to start this computer anew",
                    path.display()
                )
            }),
        Err(err) if err.kind() == ErrorKind::NotFound => {
            let id = make()?.as_uuid();
            tokio::fs::write(&path, format!("{id}\n"))
                .await
                .with_context(|| format!("could not write {}", path.display()))?;
            Ok(ComputerId::from_uuid(id))
        }
        Err(err) => Err(err).with_context(|| format!("could not read {}", path.display())),
    }
}

fn make() -> anyhow::Result<PrincipalId> {
    Ok(PrincipalId::new(SystemClock.now(), OsEntropy.bytes())?)
}

fn render(identity: &Identity) -> String {
    format!("person {}\nagent {}\n", identity.person, identity.agent)
}

fn parse(text: &str) -> anyhow::Result<Identity> {
    let mut person = None;
    let mut agent = None;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Some((key, value)) = line.split_once(' ') else {
            bail!("expected `name id`, got {line:?}");
        };
        let id = PrincipalId::try_from(Uuid::parse_str(value.trim())?)?;
        match key {
            "person" => person = Some(id),
            "agent" => agent = Some(id),
            other => bail!("unknown entry {other:?}"),
        }
    }
    match (person, agent) {
        (Some(person), Some(agent)) => Ok(Identity { person, agent }),
        _ => bail!("needs both a person and an agent"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identity_reads_back_as_written_and_a_damaged_one_is_refused() {
        let identity = Identity {
            person: make().unwrap(),
            agent: make().unwrap(),
        };
        assert_eq!(parse(&render(&identity)).unwrap(), identity);
        assert!(parse("person not-a-uuid\n").is_err());
        assert!(parse(&format!("person {}\n", identity.person)).is_err());
    }
}

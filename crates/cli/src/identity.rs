//! Who you are to glave: a person id and your default agent's id, made on first run and kept
//! in the data store, so every thread records the same `for` and `agent` (see "Who acted" in
//! `specs/events.md`), and every server on one store is the same person. And which computer
//! this is: an id made the same way and kept in glave's folder, since it names this machine
//! and is never shared through a store, so a project's checkout and a thread's binding name
//! this computer the same way every run.

use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context as _, bail};
use glave_event::PrincipalId;
use glave_ports::computer::ComputerId;
use glave_ports::{Clock as _, Entropy as _, OsEntropy, SystemClock};
use uuid::Uuid;

use crate::data::Data;

/// The file glave's folder kept the identity in before the store did; moved into the store at
/// the first start that finds it, then deleted.
const FILE: &str = "identity";

/// The file holding this computer's id, in glave's folder.
const COMPUTER: &str = "computer";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    pub person: PrincipalId,
    pub agent: PrincipalId,
}

/// Your identity from the store, made and kept there the first time: from the file an earlier
/// glave kept in `home`, when there is one, which is deleted once the store holds it.
pub async fn load_or_make(home: &Path, store: &Data) -> anyhow::Result<Identity> {
    let path = home.join(FILE);
    let earlier = match tokio::fs::read_to_string(&path).await {
        Ok(text) => Some(parse(&text).with_context(|| {
            format!(
                "{} is damaged; move it aside to start a new identity",
                path.display()
            )
        })?),
        Err(err) if err.kind() == ErrorKind::NotFound => None,
        Err(err) => return Err(err).with_context(|| format!("could not read {}", path.display())),
    };
    let new = match earlier {
        Some(identity) => identity,
        None => Identity {
            person: make()?,
            agent: make()?,
        },
    };
    let (person, agent) = store.identity((new.person, new.agent)).await?;
    if earlier.is_some() {
        // The store holds it now; a crash before this reads the file again, to the same end.
        match tokio::fs::remove_file(&path).await {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err).with_context(|| format!("could not remove {}", path.display()));
            }
        }
    }
    Ok(Identity { person, agent })
}

/// This computer's id from glave's folder, made and saved there the first time. It is created
/// whole or not at all, so a crash never leaves one glave cannot read; when two glaves make it
/// at once, both take the one created first.
pub async fn computer(home: &Path) -> anyhow::Result<ComputerId> {
    let path = home.join(COMPUTER);
    if let Some(id) = read_computer(&path).await? {
        return Ok(ComputerId::from_uuid(id));
    }
    let id = make()?.as_uuid();
    let created = path.clone();
    let made = tokio::task::spawn_blocking(move || {
        glave_ports::file::create(&created, format!("{id}\n").as_bytes())
    })
    .await?;
    let id = match made {
        Ok(()) => id,
        Err(err) if err.kind() == ErrorKind::AlreadyExists => read_computer(&path)
            .await?
            .with_context(|| format!("{} went missing", path.display()))?,
        Err(err) => return Err(err).with_context(|| format!("could not write {}", path.display())),
    };
    Ok(ComputerId::from_uuid(id))
}

/// The computer id kept at `path`; none when there is none yet.
async fn read_computer(path: &Path) -> anyhow::Result<Option<Uuid>> {
    match tokio::fs::read_to_string(path).await {
        Ok(text) => Uuid::parse_str(text.trim()).map(Some).with_context(|| {
            format!(
                "{} is damaged; move it aside to start this computer anew",
                path.display()
            )
        }),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("could not read {}", path.display())),
    }
}

fn make() -> anyhow::Result<PrincipalId> {
    Ok(PrincipalId::new(SystemClock.now(), OsEntropy.bytes()?)?)
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

    fn identity() -> Identity {
        Identity {
            person: make().unwrap(),
            agent: make().unwrap(),
        }
    }

    /// The identity file as an earlier glave wrote it.
    fn written(identity: &Identity) -> String {
        format!("person {}\nagent {}\n", identity.person, identity.agent)
    }

    fn store() -> Data {
        let root = glave_keys::InMemoryRoot::new().unwrap();
        let keys = glave_keys::KeySettings::new(std::sync::Arc::new(root));
        Data::Sqlite(glave_store_sqlite::Store::open_in_memory(keys).unwrap())
    }

    fn home(name: &str) -> std::path::PathBuf {
        let home =
            std::env::temp_dir().join(format!("glave-identity-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        home
    }

    #[test]
    fn an_earlier_identity_file_reads_back_and_a_damaged_one_is_refused() {
        let identity = identity();
        assert_eq!(parse(&written(&identity)).unwrap(), identity);
        assert!(parse("person not-a-uuid\n").is_err());
        assert!(parse(&format!("person {}\n", identity.person)).is_err());
    }

    #[test]
    fn a_new_identity_is_kept_in_the_store_and_no_file_is_written() {
        let home = home("new");
        let store = store();
        glave_ports::block_on!(async {
            let made = load_or_make(&home, &store).await.unwrap();
            assert_eq!(load_or_make(&home, &store).await.unwrap(), made);
        });
        assert!(!home.join(FILE).exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn an_identity_file_moves_into_the_store_once_and_is_never_read_again() {
        let home = home("moved");
        let store = store();
        let earlier = identity();
        std::fs::write(home.join(FILE), written(&earlier)).unwrap();
        glave_ports::block_on!(async {
            assert_eq!(load_or_make(&home, &store).await.unwrap(), earlier);
            assert!(
                !home.join(FILE).exists(),
                "the file goes once the store holds it"
            );
            assert_eq!(load_or_make(&home, &store).await.unwrap(), earlier);
        });
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_move_stopped_before_the_file_was_deleted_ends_the_same_at_the_next_start() {
        let home = home("stopped");
        let store = store();
        let earlier = identity();
        std::fs::write(home.join(FILE), written(&earlier)).unwrap();
        glave_ports::block_on!(async {
            // The store took it, and glave stopped before deleting the file.
            store
                .identity((earlier.person, earlier.agent))
                .await
                .unwrap();
            assert_eq!(load_or_make(&home, &store).await.unwrap(), earlier);
            assert!(!home.join(FILE).exists());
            // A file that turns up later never replaces what the store holds.
            std::fs::write(home.join(FILE), written(&identity())).unwrap();
            assert_eq!(load_or_make(&home, &store).await.unwrap(), earlier);
        });
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_computer_id_whose_creation_stopped_at_any_step_is_read_or_made_whole() {
        use glave_ports::file::{Stop, create_until};
        let home = std::env::temp_dir().join(format!("glave-computer-id-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let planted = make().unwrap().as_uuid();
        let path = home.join(COMPUTER);
        glave_ports::block_on!(async {
            // Stopped before it was put in place: a new id is made, and kept.
            create_until(&path, format!("{planted}\n").as_bytes(), Stop::Written).unwrap();
            let made = computer(&home).await.unwrap();
            assert_ne!(made.as_uuid(), planted);
            assert_eq!(computer(&home).await.unwrap(), made);
            // Stopped once in place: the whole id is read.
            std::fs::remove_file(&path).unwrap();
            create_until(&path, format!("{planted}\n").as_bytes(), Stop::Placed).unwrap();
            assert_eq!(computer(&home).await.unwrap().as_uuid(), planted);
        });
        let _ = std::fs::remove_dir_all(&home);
    }
}

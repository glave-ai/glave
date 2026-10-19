//! Who you are to glave: a person id and your default agent's id, made on first run and kept
//! in the data store, so every thread records the same `for` and `agent` (see "Who acted" in
//! `specs/events.md`), and every server on one store is the same person. And which computer
//! this is: an id made the same way and kept in glave's folder, since it names this machine
//! and is never shared through a store, so a project's checkout and a thread's binding name
//! this computer the same way every run.

use std::io::ErrorKind;
use std::path::Path;

use anyhow::Context as _;
use glave_event::PrincipalId;
use glave_ports::computer::ComputerId;
use glave_ports::{Clock as _, Entropy as _, OsEntropy, SystemClock};
use uuid::Uuid;

use crate::data::Data;

/// The file holding this computer's id, in glave's folder.
const COMPUTER: &str = "computer";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    pub person: PrincipalId,
    pub agent: PrincipalId,
}

/// Your identity from the store, made and kept there the first time.
pub async fn load_or_make(store: &Data) -> anyhow::Result<Identity> {
    let (person, agent) = store.identity((make()?, make()?)).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Data {
        let root = glave_keys::InMemoryRoot::new().unwrap();
        let keys = glave_keys::KeySettings::new(std::sync::Arc::new(root));
        Data::Sqlite(glave_store_sqlite::Store::open_in_memory(keys).unwrap())
    }

    #[test]
    fn a_new_identity_is_kept_in_the_store() {
        let store = store();
        glave_ports::block_on!(async {
            let made = load_or_make(&store).await.unwrap();
            assert_eq!(load_or_make(&store).await.unwrap(), made);
        });
    }

    #[test]
    fn a_computer_id_whose_creation_stopped_at_any_step_is_read_or_made_whole() {
        use glave_ports::file::{Stop, create_until};
        let home = glave_ports::testing::TempDir::new("computer-id").unwrap();
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

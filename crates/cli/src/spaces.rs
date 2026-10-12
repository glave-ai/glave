//! The spaces on this computer (`specs/storage.md`, "Spaces"): the space a folder leads to,
//! found by its canonical path or, for a clone that moved or another checkout, by its `origin`
//! remote; and the person's personal space, where one-off chats and personal memory live.

use std::path::Path;

use glave_event::{At, Principal, PrincipalId};
use glave_ports::{FolderKey, OrgId, Scope, Space, SpaceId, SpaceKind};

use crate::data::Data;
use crate::system::new_uuid_v7;

/// The store's one organization.
async fn org(store: &Data) -> anyhow::Result<OrgId> {
    store.org().await
}

/// A new space record, owned by `owner`, made now.
fn new_space(kind: SpaceKind, name: String, owner: Principal) -> Space {
    let now = At::from(jiff::Timestamp::now());
    Space {
        id: SpaceId::from_uuid(new_uuid_v7()),
        kind,
        name,
        owner,
        members: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

/// The scope of the project space `folder` leads to, for `person`: found by its path, else by
/// the repository's `origin` remote, else made as theirs. Spaces made before spaces were
/// records become theirs first.
pub async fn here(store: &Data, folder: &Path, person: PrincipalId) -> anyhow::Result<Scope> {
    let org = org(store).await?;
    let owner = Principal::person(person);
    store.claim_spaces(&owner).await?;
    let canonical = tokio::fs::canonicalize(folder)
        .await
        .unwrap_or_else(|_| folder.to_owned());
    let mut keys = vec![FolderKey::Path(canonical.to_string_lossy().into_owned())];
    if let Some(remote) = glave_config::trust::remote(&canonical).await {
        keys.push(FolderKey::Remote(remote));
    }
    let name = canonical.file_name().map_or_else(
        || canonical.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let new = new_space(SpaceKind::Project, name, owner);
    let space = store.spaces().folder(org, &keys, &new).await?;
    Ok(Scope {
        org,
        space: space.id,
    })
}

/// The scope of `person`'s personal space, made the first time.
pub async fn personal(store: &Data, person: PrincipalId) -> anyhow::Result<Scope> {
    let org = org(store).await?;
    let new = new_space(
        SpaceKind::Personal,
        glave_session::PERSONAL_NAME.to_owned(),
        Principal::person(person),
    );
    let space = store.spaces().personal(org, &new).await?;
    Ok(Scope {
        org,
        space: space.id,
    })
}

/// The scope glave keeps a record of its own under `name`, such as the gateway's, which no
/// person works in.
pub async fn named(store: &Data, name: &str) -> anyhow::Result<Scope> {
    let org = org(store).await?;
    let new = new_space(SpaceKind::Project, name.to_owned(), Principal::glave());
    let keys = [FolderKey::Path(name.to_owned())];
    let space = store.spaces().folder(org, &keys, &new).await?;
    Ok(Scope {
        org,
        space: space.id,
    })
}

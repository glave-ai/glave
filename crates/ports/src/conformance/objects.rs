//! What every place that keeps objects must do: an object is there, whole, only once its writer
//! finishes; it reads back exactly however it was written; an abandoned one is not there; a
//! deleted one is gone, and deleting what is not there is no error; scopes keep their objects
//! apart; and a name that is not one is refused.

use super::{Verdict, ensure, scope};
use crate::blob::read_all;
use crate::objects::Objects;
use crate::scope::Scope;

/// A limit no object here reaches.
const ROOMY: u64 = 64 * 1024 * 1024;

/// Every object the suite writes, deleted before it starts and after it passes, so a bucket kept
/// between runs holds nothing of it.
const WRITTEN: [(u64, &str); 4] = [
    (61, "c0ffee0001"),
    (61, "c0ffee0003"),
    (63, "c0ffee0004"),
    (64, "c0ffee0006"),
];

/// The suite.
pub async fn objects(objects: &dyn Objects) -> Verdict {
    tidy(objects).await?;
    written_whole(objects).await?;
    large(objects).await?;
    abandoned_and_deleted(objects).await?;
    names(objects).await?;
    tidy(objects).await
}

async fn tidy(objects: &dyn Objects) -> Verdict {
    for (n, object) in WRITTEN {
        objects
            .delete(scope(n), object)
            .await
            .map_err(|e| e.to_string())?;
    }
    objects
        .delete(scope(65), "c0ffee0006")
        .await
        .map_err(|e| e.to_string())
}

/// Writes `pieces` as `object` in `scope` and finishes it.
async fn write(
    objects: &dyn Objects,
    scope: Scope,
    object: &str,
    pieces: &[Vec<u8>],
) -> Result<(), String> {
    let mut writer = objects
        .create(scope, object)
        .await
        .map_err(|e| e.to_string())?;
    for piece in pieces {
        writer
            .write(piece.clone())
            .await
            .map_err(|e| e.to_string())?;
    }
    writer.finish().await.map_err(|e| e.to_string())
}

/// `object`'s bytes in `scope`, read whole, or `None`.
async fn read(
    objects: &dyn Objects,
    scope: Scope,
    object: &str,
) -> Result<Option<Vec<u8>>, String> {
    let Some(mut bytes) = objects
        .read(scope, object)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    read_all(bytes.as_mut(), ROOMY)
        .await
        .map(Some)
        .map_err(|e| e.to_string())
}

async fn written_whole(objects: &dyn Objects) -> Verdict {
    let (here, elsewhere) = (scope(61), scope(62));
    let object = "c0ffee0001";
    let mut writer = objects
        .create(here, object)
        .await
        .map_err(|e| e.to_string())?;
    writer
        .write(b"half ".to_vec())
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        read(objects, here, object).await?.is_none(),
        "an object must not be there before its writer finishes",
    )?;
    writer
        .write(b"and whole".to_vec())
        .await
        .map_err(|e| e.to_string())?;
    writer.finish().await.map_err(|e| e.to_string())?;
    ensure(
        read(objects, here, object).await? == Some(b"half and whole".to_vec()),
        "an object must read back exactly as written",
    )?;
    ensure(
        read(objects, elsewhere, object).await?.is_none(),
        "another scope must not see this object",
    )?;
    ensure(
        read(objects, here, "c0ffee0002").await?.is_none(),
        "an object never written must read as missing",
    )?;
    write(objects, here, "c0ffee0003", &[]).await?;
    ensure(
        read(objects, here, "c0ffee0003").await? == Some(Vec::new()),
        "an empty object must read back empty",
    )?;
    let mut listed: Vec<(Scope, String)> = objects
        .list(here.org())
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|listed| (listed.scope, listed.object))
        .collect();
    listed.sort();
    ensure(
        listed == [(here, "c0ffee0001".into()), (here, "c0ffee0003".into())],
        "a listing must name exactly the organization's finished objects, with their scope",
    )
}

/// Several megabytes, written in pieces larger than a bucket's smallest part, read back whole.
async fn large(objects: &dyn Objects) -> Verdict {
    let here = scope(63);
    let pieces: Vec<Vec<u8>> = (0..7_u8)
        .map(|n| vec![n; 1024 * 1024 + usize::from(n)])
        .collect();
    write(objects, here, "c0ffee0004", &pieces).await?;
    ensure(
        read(objects, here, "c0ffee0004").await? == Some(pieces.concat()),
        "a large object must read back exactly",
    )
}

async fn abandoned_and_deleted(objects: &dyn Objects) -> Verdict {
    let here = scope(64);
    let mut writer = objects
        .create(here, "c0ffee0005")
        .await
        .map_err(|e| e.to_string())?;
    writer
        .write(b"never kept".to_vec())
        .await
        .map_err(|e| e.to_string())?;
    writer.abort().await.map_err(|e| e.to_string())?;
    ensure(
        read(objects, here, "c0ffee0005").await?.is_none(),
        "an abandoned object must not be there",
    )?;
    write(objects, here, "c0ffee0006", &[b"kept".to_vec()]).await?;
    write(objects, scope(65), "c0ffee0006", &[b"kept".to_vec()]).await?;
    objects
        .delete(here, "c0ffee0006")
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        read(objects, here, "c0ffee0006").await?.is_none(),
        "a deleted object must be gone",
    )?;
    ensure(
        read(objects, scope(65), "c0ffee0006").await?.is_some(),
        "deleting an object must not touch another scope's",
    )?;
    objects
        .delete(here, "c0ffee0006")
        .await
        .map_err(|e| e.to_string())?;
    objects
        .delete(scope(65), "c0ffee0006")
        .await
        .map_err(|e| e.to_string())
}

async fn names(objects: &dyn Objects) -> Verdict {
    let here = scope(66);
    for name in ["", "../escape", "a/b", "UPPER", "dot.ted"] {
        ensure(
            objects.create(here, name).await.is_err(),
            "a name that is not one part of a path must be refused",
        )?;
        ensure(
            objects.read(here, name).await.is_err(),
            "reading a name that is not one must be refused",
        )?;
    }
    Ok(())
}

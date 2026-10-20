//! The bucket: objects in a folder or an object store, written in parts, read as a stream.

use std::fmt;
use std::future::poll_fn;
use std::path::Path as FsPath;
use std::pin::Pin;
use std::sync::Arc;

use futures_core::Stream;
use glave_ports::{BlobError, BoxFuture, Chunks, ObjectWriter, Objects, Scope, object_name_ok};
use object_store::local::LocalFileSystem;
use object_store::path::Path;
use object_store::{Error, GetResult, ObjectStore, ObjectStoreExt as _, WriteMultipart};
use thiserror::Error as ThisError;

/// Parts uploaded at once while an object is written.
const PARTS_AT_ONCE: usize = 4;

/// The objects of one deployment: a folder, or a bucket.
#[derive(Clone)]
pub struct Bucket {
    store: Arc<dyn ObjectStore>,
}

/// Why a bucket could not be opened.
#[derive(Debug, ThisError)]
pub enum BucketError {
    #[error("could not make the folder {path} for files: {error}")]
    Folder { path: String, error: String },
}

impl fmt::Debug for Bucket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bucket")
            .field("store", &self.store.to_string())
            .finish()
    }
}

impl Bucket {
    /// The folder `path`, made the first time and readable only by its owner: a person's own
    /// computer or server keeps its blobs there.
    pub async fn folder(path: &FsPath) -> Result<Self, BucketError> {
        let failed = |error: &dyn fmt::Display| BucketError::Folder {
            path: path.display().to_string(),
            error: error.to_string(),
        };
        let mut builder = tokio::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder.create(path).await.map_err(|e| failed(&e))?;
        let store = LocalFileSystem::new_with_prefix(path)
            .map_err(|e| failed(&e))?
            .with_automatic_cleanup(true);
        Ok(Self {
            store: Arc::new(store),
        })
    }

    async fn read_now(&self, path: Path) -> Result<Option<Box<dyn Chunks>>, BlobError> {
        match self.store.get(&path).await {
            Ok(found) => Ok(Some(Box::new(Read::from(found)))),
            Err(Error::NotFound { .. }) => Ok(None),
            Err(error) => Err(failed("read", &error)),
        }
    }

    async fn delete_now(&self, path: Path) -> Result<(), BlobError> {
        match self.store.delete(&path).await {
            Ok(()) | Err(Error::NotFound { .. }) => Ok(()),
            Err(error) => Err(failed("delete", &error)),
        }
    }
}

/// Where `object` of `scope` is kept: `<organization>/<project>/<object>`.
fn located(scope: Scope, object: &str) -> Result<Path, BlobError> {
    if !object_name_ok(object) {
        return Err(BlobError::Storage(format!(
            "{object:?} is not an object name"
        )));
    }
    let (org, project) = (scope.org().to_string(), scope.project().to_string());
    Ok(Path::from_iter([org.as_str(), project.as_str(), object]))
}

fn failed(what: &str, error: &Error) -> BlobError {
    BlobError::Storage(format!("could not {what} a file's bytes: {error}"))
}

impl Objects for Bucket {
    fn create(
        &self,
        scope: Scope,
        object: &str,
    ) -> BoxFuture<'_, Result<Box<dyn ObjectWriter>, BlobError>> {
        let path = located(scope, object);
        Box::pin(async move {
            let upload = self
                .store
                .put_multipart(&path?)
                .await
                .map_err(|e| failed("start writing", &e))?;
            Ok(Box::new(Writer(WriteMultipart::new(upload))) as Box<dyn ObjectWriter>)
        })
    }

    fn read(
        &self,
        scope: Scope,
        object: &str,
    ) -> BoxFuture<'_, Result<Option<Box<dyn Chunks>>, BlobError>> {
        let path = located(scope, object);
        Box::pin(async move { self.read_now(path?).await })
    }

    fn delete(&self, scope: Scope, object: &str) -> BoxFuture<'_, Result<(), BlobError>> {
        let path = located(scope, object);
        Box::pin(async move { self.delete_now(path?).await })
    }
}

/// An object being written, in parts uploaded a few at a time.
struct Writer(WriteMultipart);

impl ObjectWriter for Writer {
    fn write(&mut self, bytes: Vec<u8>) -> BoxFuture<'_, Result<(), BlobError>> {
        Box::pin(async move {
            self.0
                .wait_for_capacity(PARTS_AT_ONCE)
                .await
                .map_err(|e| failed("write", &e))?;
            self.0.put(bytes.into());
            Ok(())
        })
    }

    fn finish(self: Box<Self>) -> BoxFuture<'static, Result<(), BlobError>> {
        Box::pin(async move {
            self.0
                .finish()
                .await
                .map(drop)
                .map_err(|e| failed("finish writing", &e))
        })
    }

    fn abort(self: Box<Self>) -> BoxFuture<'static, Result<(), BlobError>> {
        Box::pin(async move { self.0.abort().await.map_err(|e| failed("abandon", &e)) })
    }
}

/// An object's bytes as object_store streams them.
struct Read(Pin<Box<dyn Stream<Item = object_store::Result<bytes::Bytes>> + Send>>);

impl From<GetResult> for Read {
    fn from(found: GetResult) -> Self {
        Self(found.into_stream())
    }
}

impl Chunks for Read {
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, BlobError>> {
        Box::pin(async move {
            let next = poll_fn(|cx| self.0.as_mut().poll_next(cx)).await;
            next.transpose()
                .map(|bytes| bytes.map(|bytes| bytes.to_vec()))
                .map_err(|e| failed("read", &e))
        })
    }
}

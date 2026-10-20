//! glave's bucket: where blobs' sealed bytes are kept, apart from their records
//! (`specs/storage.md`, "Which stores"). On a person's computer or own server it is a folder in
//! glave's folder. One library, `object_store` (Apache Arrow's), serves every place, so the
//! same code writes, reads and deletes in each.
//!
//! Objects are kept as `<organization>/<project>/<object>`, one prefix per organization, for
//! usage and deletion per organization.

mod bucket;

pub use bucket::{Bucket, BucketError};

//! What the requests of the table of methods (`protocol.rs`) are made of beside the types of
//! other modules: the choices of their params, and how serde reads and writes their fields.

use serde::{Deserialize, Serialize};

/// Whether a flag is off, so it is left out of a request.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if"
)]
pub(super) fn is_false(value: &bool) -> bool {
    !*value
}

/// A field that is given, `null` included, as distinct from one left out, which its
/// `#[serde(default)]` reads as `None`: `null` is `Some(None)`.
pub(super) fn given<'de, D, T>(field: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(field).map(Some)
}

/// Which threads `thread/list` shows.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Show {
    /// Threads that are neither archived nor deleted.
    #[default]
    Active,
    Archived,
    /// Deleted threads, waiting for their content to be erased: for the operator that does it.
    Deleted,
}

/// The person's answer to an approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    AllowOnce,
    AllowForThread,
    Deny,
}

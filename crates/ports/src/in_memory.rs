//! In-memory implementations of the storage, lease and tool ports, for tests and for runs that keep
//! nothing: one module per adapter.

mod blobs;
mod leases;
mod log;
mod objects;
mod projects;
mod tools;

pub use blobs::InMemoryBlobs;
pub use leases::InMemoryLeases;
pub use log::InMemoryLog;
pub use objects::InMemoryObjects;
pub use projects::InMemoryProjects;
pub use tools::InMemoryTools;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::testing::ManualClock;
    use crate::{block_on, conformance};

    #[test]
    fn two_organizations_in_memory_are_kept_apart() {
        let blobs = InMemoryBlobs::with_clock(Arc::new(ManualClock::at_millis(0)));
        let verdict = block_on!(conformance::organizations(
            &InMemoryLog::new(),
            &blobs,
            &InMemoryProjects::new()
        ));
        assert_eq!(verdict, Ok(()));
    }
}

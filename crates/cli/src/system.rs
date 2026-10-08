//! The operating system's randomness for this run's local ids.

/// A random UUID v4, for this run's local organization and space.
pub fn random_uuid() -> uuid::Uuid {
    let mut bytes = [0u8; 16];
    let _ = getrandom::fill(&mut bytes);
    uuid::Builder::from_random_bytes(bytes).into_uuid()
}

//! The operating system's randomness for this run's local ids.

/// A random UUID v4, for this run's local organization and space.
pub fn random_uuid() -> uuid::Uuid {
    let mut bytes = [0u8; 16];
    let _ = getrandom::fill(&mut bytes);
    uuid::Builder::from_random_bytes(bytes).into_uuid()
}

/// A new UUID v7, time-ordered, for an id glave makes here, such as a new space's.
pub fn new_uuid_v7() -> uuid::Uuid {
    let mut random = [0u8; 10];
    let _ = getrandom::fill(&mut random);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        });
    uuid::Builder::from_unix_timestamp_millis(millis, &random).into_uuid()
}

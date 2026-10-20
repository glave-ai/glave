//! The operating system's randomness for this run's local ids.

use glave_ports::{Clock as _, SystemClock};

/// A random UUID v4, for this run's local organization and project.
pub fn random_uuid() -> anyhow::Result<uuid::Uuid> {
    Ok(uuid::Builder::from_random_bytes(random()?).into_uuid())
}

/// A new UUID v7, time-ordered, for an id glave makes here, such as a new project's.
pub fn new_uuid_v7() -> anyhow::Result<uuid::Uuid> {
    let random: [u8; 10] = random()?;
    let millis = u64::try_from(SystemClock.now().as_millisecond()).unwrap_or(0);
    Ok(uuid::Builder::from_unix_timestamp_millis(millis, &random).into_uuid())
}

/// Random bytes from the operating system, or why there are none: an id is never left with a
/// zero-filled random part.
fn random<const N: usize>() -> anyhow::Result<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes)
        .map_err(|e| anyhow::anyhow!("could not get random bytes for a new id: {e}"))?;
    Ok(bytes)
}

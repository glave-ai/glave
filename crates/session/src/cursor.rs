//! `thread/list`'s cursors as clients hold them: opaque text naming the last thread of a page
//! by its own sort key (when it was last updated, and its id), and the list it came from (the
//! project, and which threads it shows). It carries nothing about any other thread, in the
//! caller's project or anyone else's, so no cursor says how busy the store is. A cursor from
//! another list, or one that does not read, is refused plainly.
//!
//! It needs no key: all it holds is a row the caller was just shown, and the store pages only
//! inside the caller's scope, so a cursor made up by hand reaches nothing more than the first
//! page would.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use glave_event::{At, ThreadId};
use glave_ports::{ProjectId, Standing, ThreadCursor};
use uuid::Uuid;

use crate::protocol::{ApiError, code};
use crate::words::Sentence;

/// The layout's version, its first byte.
const VERSION: u8 = 1;

/// Version, project, standing, milliseconds, thread.
const LENGTH: usize = 1 + 16 + 1 + 8 + 16;

/// `after`, the last thread of a page of `standing` threads in `project`, as a client holds it.
pub(crate) fn write(project: ProjectId, standing: Standing, after: ThreadCursor) -> String {
    let mut bytes = Vec::with_capacity(LENGTH);
    bytes.push(VERSION);
    bytes.extend_from_slice(project.as_uuid().as_bytes());
    bytes.push(standing_byte(standing));
    let millis = after.updated_at.timestamp().as_millisecond();
    bytes.extend_from_slice(&millis.to_be_bytes());
    bytes.extend_from_slice(after.thread.as_uuid().as_bytes());
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The place `text` names in the list of `standing` threads in `project`: refused when it does
/// not read, or names a place in another list.
pub(crate) fn read(
    text: &str,
    project: ProjectId,
    standing: Standing,
) -> Result<ThreadCursor, ApiError> {
    let refused = || ApiError::new(code::INVALID_PARAMS, Sentence::PageGone);
    let bytes = URL_SAFE_NO_PAD.decode(text).map_err(|_| refused())?;
    let bytes: &[u8; LENGTH] = bytes.as_slice().try_into().map_err(|_| refused())?;
    let [version, rest @ ..] = bytes;
    let (of, rest) = rest.split_first_chunk::<16>().ok_or_else(refused)?;
    let (shows, rest) = rest.split_first().ok_or_else(refused)?;
    let (millis, thread) = rest.split_first_chunk::<8>().ok_or_else(refused)?;
    let thread: &[u8; 16] = thread.try_into().map_err(|_| refused())?;
    let same_list = *version == VERSION
        && Uuid::from_bytes(*of) == project.as_uuid()
        && *shows == standing_byte(standing);
    if !same_list {
        return Err(refused());
    }
    let at =
        jiff::Timestamp::from_millisecond(i64::from_be_bytes(*millis)).map_err(|_| refused())?;
    let thread = ThreadId::try_from(Uuid::from_bytes(*thread)).map_err(|_| refused())?;
    Ok(ThreadCursor {
        updated_at: At::from(at),
        thread,
    })
}

fn standing_byte(standing: Standing) -> u8 {
    match standing {
        Standing::Active => 0,
        Standing::Archived => 1,
        Standing::Deleted => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(n: u128) -> ProjectId {
        ProjectId::from_uuid(Uuid::from_u128(n))
    }

    fn place() -> ThreadCursor {
        let at = jiff::Timestamp::from_millisecond(1_760_000_000_123).unwrap();
        ThreadCursor {
            updated_at: At::from(at),
            thread: ThreadId::new(at, [7; 10]).unwrap(),
        }
    }

    #[test]
    fn a_cursor_reads_back_in_its_own_list_only() {
        let text = write(project(1), Standing::Active, place());
        assert_eq!(read(&text, project(1), Standing::Active).unwrap(), place());
        for (project, standing) in [
            (project(2), Standing::Active),
            (project(1), Standing::Archived),
        ] {
            let refused = read(&text, project, standing).unwrap_err();
            assert_eq!(refused.code, code::INVALID_PARAMS);
        }
    }

    #[test]
    fn text_that_is_not_a_cursor_is_refused_plainly() {
        let text = write(project(1), Standing::Active, place());
        let shorter = text.get(..text.len() - 2).unwrap();
        let mut newer = URL_SAFE_NO_PAD.decode(&text).unwrap();
        newer[0] = VERSION + 1;
        for bad in [
            "",
            "42",
            "not a cursor",
            shorter,
            &URL_SAFE_NO_PAD.encode(newer),
        ] {
            let refused = read(bad, project(1), Standing::Active).unwrap_err();
            assert_eq!(refused.code, code::INVALID_PARAMS, "{bad:?}");
            assert!(!refused.message.contains("cursor"), "plain words");
        }
    }
}

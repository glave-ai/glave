//! The caller's time zone (`zone/set`): what their browser says it is, kept in their settings
//! ledger as `zone.set`, so a schedule they set up repeats at their own times. Their client gives
//! it whenever it opens; it is kept only when it changed. With none kept, UTC.

use glave_event::{Body, ZoneSet};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};
use jiff::tz::TimeZone;

use crate::protocol::{ApiError, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

/// The zone of a caller who never gave one.
pub(crate) const UTC: &str = "UTC";

/// The time zone named `name` (`Europe/Lisbon`), or why there is none.
pub(crate) fn time_zone(name: &str) -> Result<TimeZone, ApiError> {
    TimeZone::get(name).map_err(|_| {
        ApiError::new(
            code::INVALID_PARAMS,
            Sentence::NoZone {
                zone: name.escape_debug().to_string(),
            },
        )
    })
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `zone/set`: keeps `zone` as the caller's, unless it is the one kept already.
    /// Refused for a zone glave doesn't know.
    pub(crate) async fn set_zone(&self, caller: &Caller, zone: String) -> Result<Reply, ApiError> {
        let zone = zone.trim().to_owned();
        time_zone(&zone)?;
        if self.zone_of(caller).await? != zone {
            self.keep_setting(caller, &ZoneSet { zone }).await?;
        }
        Ok(Reply::Done {})
    }

    /// The caller's time zone, by its name: the one they last gave, else UTC.
    pub(crate) async fn zone_of(&self, caller: &Caller) -> Result<String, ApiError> {
        Ok(match self.setting::<ZoneSet>(caller).await? {
            Some(Body::ZoneSet(set)) => set.zone,
            _ => UTC.to_owned(),
        })
    }
}

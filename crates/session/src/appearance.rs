//! A person's accent (`appearance/get`, `appearance/set`): the choice kept in their settings
//! ledger as `appearance.set`, glave's plain look until they make one, and said to a client with
//! the light and dark values to draw it in, so no client works out a colour.

use glave_event::{Accent, AppearanceSet, Body};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};

use crate::accent::shades;
use crate::protocol::{ApiError, Reply};
use crate::server::Server;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `appearance/get`: the caller's accent, plain until they choose.
    pub(crate) async fn appearance(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let chosen = match self.setting::<AppearanceSet>(caller).await? {
            Some(Body::AppearanceSet(set)) => set.accent,
            _ => Accent::Plain,
        };
        Ok(reply(chosen))
    }

    /// Answers `appearance/set`: keeps `accent` as the caller's.
    pub(crate) async fn set_appearance(
        &self,
        caller: &Caller,
        accent: Accent,
    ) -> Result<Reply, ApiError> {
        self.keep_setting(caller, &AppearanceSet { accent }).await?;
        Ok(reply(accent))
    }
}

/// The reply that says `chosen`, and what it is drawn as.
fn reply(chosen: Accent) -> Reply {
    let shades = match chosen {
        Accent::Plain => None,
        Accent::Color { color, adjust } => Some(shades(color, adjust)),
    };
    Reply::Appearance { chosen, shades }
}

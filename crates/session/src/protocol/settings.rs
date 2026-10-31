//! What of a person's own a request changes, which their other clients hear of
//! (`settings/changed`) and read again.

use serde::{Deserialize, Serialize};

use super::Request;

/// A person's own settings or connections, by what a client reads to show them.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Setting {
    /// Their model accounts (`provider/list`), and so the models threads may choose
    /// (`model/list`).
    Providers,
    /// Their git hosts (`credential/list`).
    Credentials,
    /// Their sign-ins to connections (`connection/list`).
    Connections,
    /// Whether their agent may search the web (`search/get`).
    Search,
    /// How their clients group their threads (`grouping/get`).
    Grouping,
    /// Their filters among a router's providers (`filters/get`).
    Filters,
    /// Their accent (`appearance/get`).
    Appearance,
    /// Whether the computer is kept awake while glave works (`awake/get`).
    Awake,
    /// Whether glave remembers for them (`memory/get`).
    Memory,
    /// Their passkeys (`passkey/list`).
    Passkeys,
    /// The language the server writes its own sentences to them in (`language/get`): what a
    /// client shows of the server's words, an approval's included, it reads again.
    Language,
}

impl Setting {
    /// Every setting, as a watcher that may have missed some is told.
    pub(crate) const ALL: [Self; 11] = [
        Self::Providers,
        Self::Credentials,
        Self::Connections,
        Self::Search,
        Self::Grouping,
        Self::Filters,
        Self::Appearance,
        Self::Awake,
        Self::Memory,
        Self::Passkeys,
        Self::Language,
    ];
}

impl Request {
    /// What of the caller's own this request changes once it is answered, unless with a
    /// sign-in still waiting for them; none for one that changes nothing of theirs.
    pub(crate) fn changes(&self) -> Option<Setting> {
        Some(match self {
            Self::ProviderConnect { .. }
            | Self::ProviderRefresh {}
            | Self::ProviderRemove { .. } => Setting::Providers,
            Self::CredentialConnect { .. }
            | Self::CredentialRemove { .. }
            | Self::CredentialDevicePoll { .. } => Setting::Credentials,
            Self::ConnectionPoll { .. } | Self::ConnectionRemove { .. } => Setting::Connections,
            Self::SearchSet { .. } => Setting::Search,
            Self::GroupingSet { .. } => Setting::Grouping,
            Self::FiltersSet { .. } => Setting::Filters,
            Self::AppearanceSet { .. } | Self::BrandSet { .. } => Setting::Appearance,
            Self::AwakeSet { .. } => Setting::Awake,
            Self::MemorySet { .. } => Setting::Memory,
            Self::PasskeyCreate { .. }
            | Self::PasskeyAdd { .. }
            | Self::PasskeyRemove { .. }
            | Self::PersonName { .. } => Setting::Passkeys,
            Self::LanguageSet { .. } => Setting::Language,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_says_what_it_changes_and_a_read_nothing() {
        assert_eq!(
            Request::SearchSet { on: true }.changes(),
            Some(Setting::Search)
        );
        let poll = Request::ConnectionPoll {
            flow: "flow".into(),
        };
        assert_eq!(poll.changes(), Some(Setting::Connections));
        assert_eq!(Request::SearchGet { usage: false }.changes(), None);
        let filters = Request::FiltersSet {
            filters: glave_event::Filters::default(),
        };
        assert_eq!(filters.changes(), Some(Setting::Filters));
        let appearance = Request::AppearanceSet {
            accent: Some(glave_event::Accent::Plain),
        };
        assert_eq!(appearance.changes(), Some(Setting::Appearance));
        assert_eq!(Request::FiltersGet {}.changes(), None);
        assert_eq!(
            Request::ProviderRefresh {}.changes(),
            Some(Setting::Providers)
        );
        assert_eq!(Request::GroupingGet {}.changes(), None);
        let chosen = Request::LanguageSet {
            language: Some("en-XA".into()),
        };
        assert_eq!(chosen.changes(), Some(Setting::Language));
        assert_eq!(Request::LanguageGet {}.changes(), None);
    }
}

//! The caller's language (`specs/session.md`, "Languages"): the one they chose, kept in their
//! settings ledger as `language.set` (beside `awake.set`), else the first their client asks for
//! (`Accept-Language`) that the server writes in, else English. `language/get` and
//! `language/set` read and keep their choice; a transport writes the server's own sentences in
//! an answer in their language, and reads their choice only when an answer holds one.

use glave_event::{At, Body, EventId, Header, LanguageSet, Payload as _, Seq, decode, encode};
use glave_ports::stream::{Reserved, SETTINGS, append_retrying};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};

use crate::protocol::{ApiError, Reply, code};
use crate::server::Server;
use crate::words::{Language, Sentence, says, translate};

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `language/get`: the language the caller chose, if they chose one the server
    /// writes in.
    pub(crate) async fn language(&self, caller: &Caller) -> Result<Reply, ApiError> {
        Ok(Reply::Language {
            language: self.chosen_language(caller).await?.map(|(tag, _)| tag),
        })
    }

    /// Answers `language/set`: keeps `language` as the caller's, or, with none, follows their
    /// client's again. Refused for a language the server does not write in.
    pub(crate) async fn set_language(
        &self,
        caller: &Caller,
        language: Option<String>,
    ) -> Result<Reply, ApiError> {
        if let Some(tag) = &language
            && Language::of(tag).is_none()
        {
            let tag = tag.escape_debug().to_string();
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                Sentence::NoLanguage { tag },
            ));
        }
        // Kept as the person wrote it, but for the spaces around it.
        let language = language.map(|tag| tag.trim().to_owned());
        let (_, scope) = self.personal_project(caller).await?;
        let (log, ledger) = (self.inner.harness.log(), Reserved::Settings.id());
        let set = LanguageSet {
            language: language.clone(),
        };
        let (this, set, scope) = (self, &set, &scope);
        append_retrying(log, scope, ledger, move || async move {
            let last = log.latest_of(scope, ledger, &SETTINGS).await?;
            let seq = last
                .map_or(Some(Seq::FIRST), |raw| raw.seq.next())
                .ok_or_else(|| ApiError::internal(&"the settings ledger is full"))?;
            let now = this.inner.harness.clock().now();
            let header = Header {
                id: EventId::new(now, this.inner.harness.entropy().bytes()?)?,
                thread: ledger,
                seq,
                at: At::from(now),
            };
            Ok::<_, ApiError>(Some((vec![encode(header, set)?], ())))
        })
        .await?;
        Ok(Reply::Language { language })
    }

    /// `answered` in the caller's language: their choice, else the first of `accepted` (their
    /// client's `Accept-Language`) the server writes in, else English. Their choice is read only
    /// when `answered` holds a sentence of the server's own.
    pub(crate) async fn in_language(
        &self,
        caller: &Caller,
        accepted: Option<&str>,
        mut answered: Result<Reply, ApiError>,
    ) -> Result<Reply, ApiError> {
        if says(&answered) {
            translate(&mut answered, self.language_for(caller, accepted).await);
        }
        answered
    }

    /// The language the server writes to `caller` in. A choice that cannot be read leaves the
    /// client's, then English.
    pub(crate) async fn language_for(&self, caller: &Caller, accepted: Option<&str>) -> Language {
        let chosen = self.chosen_language(caller).await.ok().flatten();
        chosen
            .map(|(_, language)| language)
            .or_else(|| accepted.and_then(Language::accepted))
            .unwrap_or(Language::English)
    }

    /// The caller's choice and its language, if they chose one the server writes in.
    async fn chosen_language(
        &self,
        caller: &Caller,
    ) -> Result<Option<(String, Language)>, ApiError> {
        let (_, scope) = self.personal_project(caller).await?;
        let latest = self
            .inner
            .harness
            .log()
            .latest_of(&scope, Reserved::Settings.id(), &[LanguageSet::TYPE])
            .await?;
        let tag = match latest.map(decode) {
            Some(Ok(event)) => match event.body {
                Body::LanguageSet(set) => set.language,
                _ => None,
            },
            _ => None,
        };
        Ok(tag.and_then(|tag| Language::of(&tag).map(|language| (tag, language))))
    }
}

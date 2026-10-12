//! The models a server can run: `model/list`, and the check that a model a thread asks for, or
//! chose before, is one of them. A thread's choice is its latest `model.chosen` event, so any
//! server reading the log knows it.

use glave_event::{Body, ThreadId, decode};
use glave_ports::{BlobStore, EventLog, ModelProvider};

use crate::protocol::{ApiError, ModelInfo, Reply, code};
use crate::server::Server;

/// The type of the event that records a thread's choice.
const CHOSEN: &str = "model.chosen";

/// The models a server built on `default` runs: the default first, then each of `offered`
/// once, in order. An offered entry for the default lends it its name and its Hope mark.
pub(crate) fn catalog(default: &str, offered: Vec<ModelInfo>) -> Vec<ModelInfo> {
    let mut first = ModelInfo::new(default);
    first.default = true;
    let mut models = vec![first];
    for mut model in offered {
        if model.id.trim().is_empty() {
            continue;
        }
        if let Some(known) = models.iter_mut().find(|known| known.id == model.id) {
            known.name = known.name.take().or(model.name);
            known.hope |= model.hope;
        } else {
            model.default = false;
            models.push(model);
        }
    }
    models
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `model/list`.
    pub(crate) fn list_models(&self) -> Reply {
        Reply::Models {
            models: self.inner.models.clone(),
        }
    }

    /// Refuses `model` with `-32021` unless this server runs it.
    pub(crate) fn known_model(&self, model: &str) -> Result<(), ApiError> {
        if self.inner.models.iter().any(|known| known.id == model) {
            return Ok(());
        }
        Err(ApiError::new(
            code::NO_MODEL,
            format!("this server does not run model {model:?}; model/list names the ones it does"),
        ))
    }

    /// The model `thread`'s next turn goes to: `asked`, else the thread's latest choice, else
    /// the default. Refused with `-32021` when this server does not run it, as when a model
    /// chosen before is no longer offered.
    pub(crate) async fn model_of(
        &self,
        thread: ThreadId,
        asked: Option<&str>,
    ) -> Result<String, ApiError> {
        let model = match asked {
            Some(model) => model.to_owned(),
            None => self
                .chosen(thread)
                .await?
                .unwrap_or_else(|| self.inner.model.clone()),
        };
        self.known_model(&model)?;
        Ok(model)
    }

    /// The model last chosen for `thread`, from its log.
    async fn chosen(&self, thread: ThreadId) -> Result<Option<String>, ApiError> {
        let scope = self.scope_of(thread).await?;
        let failed =
            |error: &dyn std::fmt::Display| ApiError::new(code::INTERNAL, error.to_string());
        let latest = self
            .inner
            .harness
            .log()
            .latest_of(&scope, thread, &[CHOSEN])
            .await
            .map_err(|error| failed(&error))?;
        let Some(raw) = latest else {
            return Ok(None);
        };
        Ok(match decode(raw).map_err(|error| failed(&error))?.body {
            Body::ModelChosen(chosen) => Some(chosen.model),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(id: &str, name: &str, hope: bool) -> ModelInfo {
        ModelInfo {
            name: Some(name.into()),
            hope,
            ..ModelInfo::new(id)
        }
    }

    #[test]
    fn the_default_comes_first_and_every_model_once() {
        let models = catalog(
            "big",
            vec![
                named("hope-12b", "Hope", true),
                named("big", "Big", false),
                ModelInfo::new("hope-12b"),
                ModelInfo::new(" "),
            ],
        );
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["big", "hope-12b"]);
        assert_eq!(
            models[0],
            ModelInfo {
                default: true,
                ..named("big", "Big", false)
            }
        );
        assert_eq!(models[1], named("hope-12b", "Hope", true));
    }

    #[test]
    fn only_the_default_is_marked_default() {
        let mut offered = ModelInfo::new("small");
        offered.default = true;
        let models = catalog("big", vec![offered]);
        assert_eq!(models.iter().filter(|m| m.default).count(), 1, "{models:?}");
    }
}

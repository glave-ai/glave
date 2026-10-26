//! The models a server can run: `model/list`, and the check that a model a thread asks for, or
//! chose before, is one of them. They change when the person connects or removes a model
//! account (`providers.rs`). A thread's choice is its latest `model.chosen` event, so any
//! server reading the log knows it.

use std::sync::PoisonError;

use glave_event::{Body, ThreadId};
use glave_ports::{BlobStore, EventLog, ModelProvider};

use crate::protocol::{ApiError, ModelInfo, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

/// The type of the event that records a thread's choice.
const CHOSEN: &str = "model.chosen";

/// The ids of the `models` that say they read no images; one that says nothing is assumed to.
pub(crate) fn without_images(models: &[ModelInfo]) -> Vec<String> {
    let blind = models.iter().filter(|known| known.images == Some(false));
    blind.map(|known| known.id.clone()).collect()
}

/// The models a server built on `default` runs: the default first, then each of `offered`
/// once, in order. An offered entry for the default describes it. With no default, none:
/// nothing is set up yet.
pub(crate) fn catalog(default: &str, offered: Vec<ModelInfo>) -> Vec<ModelInfo> {
    if default.trim().is_empty() {
        return Vec::new();
    }
    let mut models = vec![ModelInfo::new(default)];
    for mut model in offered {
        if model.id.trim().is_empty() || models.iter().skip(1).any(|known| known.id == model.id) {
            continue;
        }
        if model.id == default {
            model.default = true;
            if let Some(first) = models.first_mut() {
                *first = model;
            }
        } else {
            model.default = false;
            models.push(model);
        }
    }
    if let Some(first) = models.first_mut() {
        first.default = true;
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
            models: self
                .inner
                .models
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        }
    }

    /// Replaces the models threads may be sent to, telling the harness which read no images.
    pub(crate) fn set_catalog(&self, models: Vec<ModelInfo>) {
        self.inner
            .harness
            .set_without_images(without_images(&models));
        *self
            .inner
            .models
            .write()
            .unwrap_or_else(PoisonError::into_inner) = models;
    }

    /// Refuses `model` with `-32021` unless this server runs it.
    pub(crate) fn known_model(&self, model: &str) -> Result<(), ApiError> {
        let models = self
            .inner
            .models
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        if models.iter().any(|known| known.id == model) {
            return Ok(());
        }
        if models.is_empty() {
            return Err(ApiError::new(code::NO_MODEL, Sentence::NoModelYet));
        }
        let offered: Vec<&str> = models.iter().map(|known| known.id.as_str()).collect();
        Err(ApiError::new(
            code::NO_MODEL,
            Sentence::ModelNotRun {
                model: model.escape_debug().to_string(),
                models: offered.join(", "),
            },
        ))
    }

    /// Refuses `effort` with `-32602` unless `model` offers it, as `model/list` says.
    pub(crate) fn offered_effort(&self, model: &str, effort: &str) -> Result<(), ApiError> {
        let models = self
            .inner
            .models
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        let levels = models
            .iter()
            .find(|known| known.id == model)
            .and_then(|known| known.reasoning.as_ref())
            .map_or(&[][..], |reasoning| reasoning.efforts.as_slice());
        if levels.iter().any(|level| level == effort) {
            return Ok(());
        }
        let model = model.to_owned();
        let sentence = if levels.is_empty() {
            Sentence::NoEffortChoice { model }
        } else {
            Sentence::NoSuchEffort {
                model,
                effort: effort.escape_debug().to_string(),
                efforts: levels.join(", "),
            }
        };
        Err(ApiError::new(code::INVALID_PARAMS, sentence))
    }

    /// The model `thread`'s next turn goes to, once the host's settings are read again:
    /// `asked`, else the thread's latest choice, else its project's, else the default. Refused
    /// with `-32021` when this server does not run it, as when a model chosen before is no
    /// longer offered.
    pub(crate) async fn model_of(
        &self,
        thread: ThreadId,
        asked: Option<&str>,
    ) -> Result<String, ApiError> {
        self.reread().await;
        let model = match asked {
            Some(model) => model.to_owned(),
            None => match self.chosen(thread).await? {
                Some(model) => model,
                None => self
                    .project_model(thread)
                    .await?
                    .unwrap_or_else(|| self.inner.harness.default_model()),
            },
        };
        self.known_model(&model)?;
        Ok(model)
    }

    /// The model `thread`'s project chooses for it, when the thread is the person's here
    /// whose trust turns on what a project brings.
    async fn project_model(&self, thread: ThreadId) -> Result<Option<String>, ApiError> {
        let scope = self.scope_of(thread).await?;
        let view = self.inner.harness.log().thread_view(&scope, thread).await?;
        let whose = view.and_then(|view| view.whose());
        if whose.is_some_and(|person| Some(person) != self.person()) {
            return Ok(None);
        }
        Ok(self.project_settled(scope.project()).await.model)
    }

    /// The model last chosen for `thread`, from its log.
    async fn chosen(&self, thread: ThreadId) -> Result<Option<String>, ApiError> {
        let scope = self.scope_of(thread).await?;
        Ok(match self.latest(&scope, thread, CHOSEN).await? {
            Some(Body::ModelChosen(chosen)) => Some(chosen.model),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(id: &str, name: &str) -> ModelInfo {
        ModelInfo {
            name: Some(name.into()),
            ..ModelInfo::new(id)
        }
    }

    #[test]
    fn the_default_comes_first_and_every_model_once() {
        let models = catalog(
            "big",
            vec![
                named("hope-12b", "Hope"),
                named("big", "Big"),
                ModelInfo::new("hope-12b"),
                ModelInfo::new(" "),
            ],
        );
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["big", "hope-12b"]);
        assert!(
            catalog("", vec![ModelInfo::new("big")]).is_empty(),
            "nothing set up"
        );
        assert_eq!(
            models[0],
            ModelInfo {
                default: true,
                ..named("big", "Big")
            }
        );
        assert_eq!(models[1], named("hope-12b", "Hope"));
    }

    #[test]
    fn only_the_default_is_marked_default() {
        let mut offered = ModelInfo::new("small");
        offered.default = true;
        let models = catalog("big", vec![offered]);
        assert_eq!(models.iter().filter(|m| m.default).count(), 1, "{models:?}");
        assert!(models[0].default);
    }
}

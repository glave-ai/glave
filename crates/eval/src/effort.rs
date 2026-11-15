//! The reasoning level `--effort` asks of every model, with the levels each model offers as its
//! service's model list says, read once at the start as glave's catalog reads it: the gateway
//! then sends the level to each model that offers it, and its own to one that does not
//! (`specs/gateway.md`, "Reasoning effort").

use std::collections::BTreeMap;

use glave_event::Reasoning;
use glave_provider_openai::{Config, OpenAiCompatible, lists};
use serde_json::Value;

use crate::floor::READING;

/// The level asked of every model, and how each model reasons, by its id.
#[derive(Debug, Clone)]
pub struct Effort {
    pub(crate) level: String,
    pub(crate) offered: BTreeMap<String, Reasoning>,
}

impl Effort {
    /// `level` for `models` at `endpoint`, with how each reasons as its model list says; a
    /// model the list does not describe offers no level. Refused where the list can't be read
    /// in time.
    pub async fn listed(endpoint: &Config, level: &str, models: &[String]) -> Result<Self, String> {
        let service = OpenAiCompatible::new(endpoint.clone()).map_err(|err| err.to_string())?;
        let listed = tokio::time::timeout(READING, service.get("models"))
            .await
            .map_err(|_| {
                format!(
                    "the list of models was not read within {}",
                    glave_ports::spoken(READING)
                )
            })?
            .map_err(|err| format!("the list of models: {err}"))?;
        let entries = listed.get("data").and_then(Value::as_array);
        let offered = (entries.into_iter().flatten())
            .filter_map(|entry| {
                let id = entry.get("id").and_then(Value::as_str)?;
                models.iter().any(|model| model == id).then_some(())?;
                Some((id.to_owned(), lists::reasoning(entry.get("reasoning"))?))
            })
            .collect();
        Ok(Self {
            level: level.to_owned(),
            offered,
        })
    }
}

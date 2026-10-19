//! The stored record of one project's keys: every project-key version and every data key, each
//! wrapped, or emptied once destroyed. Emptied entries stay so numbering never repeats.

use serde::{Deserialize, Serialize};

use crate::error::KeyError;
use uuid::Uuid;

use crate::key_id::{Month, Period};

/// The record's format version.
const FORMAT: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProjectRecord {
    glave_keys: u32,
    pub(crate) versions: Vec<ProjectVersion>,
    pub(crate) data: Vec<DataKey>,
}

/// One version of the project key, wrapped by the organization key. `None` once destroyed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProjectVersion {
    pub(crate) version: u32,
    pub(crate) wrapped: Option<Vec<u8>>,
}

/// One data key, wrapped by version `project_version` of the project key. `None` once destroyed.
/// A memory's own key has `memory` set, an account's (a model account's or a git host's)
/// `provider`; its `month` is only when it was made.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DataKey {
    pub(crate) month: Month,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) memory: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) provider: Option<Uuid>,
    pub(crate) n: u32,
    /// Kept under its name from before spaces were projects: records are sealed as written.
    #[serde(rename = "space_version")]
    pub(crate) project_version: u32,
    pub(crate) wrapped: Option<Vec<u8>>,
}

impl DataKey {
    /// What the key seals.
    pub(crate) fn period(&self) -> Period {
        match (self.memory, self.provider) {
            (Some(memory), _) => Period::Memory(memory),
            (None, Some(account)) => Period::Account(account),
            (None, None) => Period::Month(self.month),
        }
    }
}

impl Default for ProjectRecord {
    fn default() -> Self {
        Self {
            glave_keys: FORMAT,
            versions: Vec::new(),
            data: Vec::new(),
        }
    }
}

impl ProjectRecord {
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, KeyError> {
        let record: Self =
            serde_json::from_slice(bytes).map_err(|e| KeyError::Corrupt(e.to_string()))?;
        if record.glave_keys != FORMAT {
            return Err(KeyError::Corrupt(format!(
                "key record format {} is unknown to this glave",
                record.glave_keys
            )));
        }
        Ok(record)
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, KeyError> {
        serde_json::to_vec(self).map_err(|e| KeyError::Corrupt(e.to_string()))
    }

    /// The project key in use: the newest version not destroyed.
    pub(crate) fn live(&self) -> Option<&ProjectVersion> {
        self.versions
            .iter()
            .filter(|version| version.wrapped.is_some())
            .max_by_key(|version| version.version)
    }

    pub(crate) fn version(&self, version: u32) -> Option<&ProjectVersion> {
        self.versions.iter().find(|entry| entry.version == version)
    }

    /// The number the next project-key version takes.
    pub(crate) fn next_version(&self) -> u32 {
        next(self.versions.iter().map(|entry| entry.version))
    }

    /// The data key that seals `period`'s new content: its newest key not destroyed.
    pub(crate) fn sealing(&self, period: Period) -> Option<&DataKey> {
        self.data
            .iter()
            .filter(|key| key.period() == period && key.wrapped.is_some())
            .max_by_key(|key| key.n)
    }

    pub(crate) fn data_key(&self, period: Period, n: u32) -> Option<&DataKey> {
        self.data
            .iter()
            .find(|key| key.period() == period && key.n == n)
    }

    /// The number `period`'s next data key takes.
    pub(crate) fn next_n(&self, period: Period) -> u32 {
        next(
            self.data
                .iter()
                .filter(|key| key.period() == period)
                .map(|key| key.n),
        )
    }

    /// Whether a data key was destroyed under the project key in use: backups that hold this
    /// version can still unwrap it, until the project key is rotated (`specs/storage.md`).
    pub(crate) fn rotation_due(&self) -> bool {
        let Some(live) = self.live() else {
            return false;
        };
        self.data
            .iter()
            .any(|key| key.wrapped.is_none() && key.project_version == live.version)
    }
}

fn next(numbers: impl Iterator<Item = u32>) -> u32 {
    numbers.max().map_or(1, |n| n.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn month(text: &str) -> Month {
        text.parse().unwrap()
    }

    #[test]
    fn destroyed_keys_keep_their_numbers_taken() {
        let mut record = ProjectRecord::default();
        record.versions.push(ProjectVersion {
            version: 1,
            wrapped: None,
        });
        record.data.push(DataKey {
            month: month("2026-10"),
            memory: None,
            provider: None,
            n: 1,
            project_version: 1,
            wrapped: None,
        });
        let october = Period::Month(month("2026-10"));
        assert!(record.live().is_none());
        assert_eq!(record.next_version(), 2);
        assert!(record.sealing(october).is_none());
        assert_eq!(record.next_n(october), 2);
        assert_eq!(record.next_n(Period::Month(month("2026-11"))), 1);
        let decoded = ProjectRecord::decode(&record.encode().unwrap()).unwrap();
        assert_eq!(decoded.next_n(october), 2);
    }

    #[test]
    fn a_memory_key_is_its_own_period_and_old_records_read_as_month_keys() {
        let mut record = ProjectRecord::default();
        record.versions.push(ProjectVersion {
            version: 1,
            wrapped: Some(vec![1]),
        });
        let memory = Uuid::from_u128(7);
        record.data.push(DataKey {
            month: month("2026-10"),
            memory: Some(memory),
            provider: None,
            n: 1,
            project_version: 1,
            wrapped: Some(vec![2]),
        });
        assert!(record.sealing(Period::Month(month("2026-10"))).is_none());
        assert!(record.sealing(Period::Memory(memory)).is_some());
        assert!(!record.rotation_due());
        if let Some(key) = record.data.first_mut() {
            key.wrapped = None;
        }
        assert!(
            record.rotation_due(),
            "destroyed under the live project key"
        );
        let old = br#"{"glave_keys":1,"versions":[],"data":[{"month":"2026-09","n":1,"space_version":1,"wrapped":null}]}"#;
        let decoded = ProjectRecord::decode(old).unwrap();
        assert!(
            decoded
                .data_key(Period::Month(month("2026-09")), 1)
                .is_some()
        );
    }

    #[test]
    fn an_unknown_format_is_refused() {
        assert!(ProjectRecord::decode(br#"{"glave_keys":2,"versions":[],"data":[]}"#).is_err());
    }
}

//! The stored record of one space's keys: every space-key version and every data key, each
//! wrapped, or emptied once destroyed. Emptied entries stay so numbering never repeats.

use serde::{Deserialize, Serialize};

use crate::error::KeyError;
use uuid::Uuid;

use crate::key_id::{Month, Period};

/// The record's format version.
const FORMAT: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SpaceRecord {
    glave_keys: u32,
    pub(crate) versions: Vec<SpaceVersion>,
    pub(crate) data: Vec<DataKey>,
}

/// One version of the space key, wrapped by the organization key. `None` once destroyed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SpaceVersion {
    pub(crate) version: u32,
    pub(crate) wrapped: Option<Vec<u8>>,
}

/// One data key, wrapped by version `space_version` of the space key. `None` once destroyed.
/// A memory's own key has `memory` set; its `month` is only when it was made.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DataKey {
    pub(crate) month: Month,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) memory: Option<Uuid>,
    pub(crate) n: u32,
    pub(crate) space_version: u32,
    pub(crate) wrapped: Option<Vec<u8>>,
}

impl DataKey {
    /// What the key seals.
    pub(crate) fn period(&self) -> Period {
        self.memory
            .map_or(Period::Month(self.month), Period::Memory)
    }
}

impl Default for SpaceRecord {
    fn default() -> Self {
        Self {
            glave_keys: FORMAT,
            versions: Vec::new(),
            data: Vec::new(),
        }
    }
}

impl SpaceRecord {
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

    /// The space key in use: the newest version not destroyed.
    pub(crate) fn live(&self) -> Option<&SpaceVersion> {
        self.versions
            .iter()
            .filter(|version| version.wrapped.is_some())
            .max_by_key(|version| version.version)
    }

    pub(crate) fn version(&self, version: u32) -> Option<&SpaceVersion> {
        self.versions.iter().find(|entry| entry.version == version)
    }

    /// The number the next space-key version takes.
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

    /// Whether a data key was destroyed under the space key in use: backups that hold this
    /// version can still unwrap it, until the space key is rotated (`specs/storage.md`).
    pub(crate) fn rotation_due(&self) -> bool {
        let Some(live) = self.live() else {
            return false;
        };
        self.data
            .iter()
            .any(|key| key.wrapped.is_none() && key.space_version == live.version)
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
        let mut record = SpaceRecord::default();
        record.versions.push(SpaceVersion {
            version: 1,
            wrapped: None,
        });
        record.data.push(DataKey {
            month: month("2026-10"),
            memory: None,
            n: 1,
            space_version: 1,
            wrapped: None,
        });
        let october = Period::Month(month("2026-10"));
        assert!(record.live().is_none());
        assert_eq!(record.next_version(), 2);
        assert!(record.sealing(october).is_none());
        assert_eq!(record.next_n(october), 2);
        assert_eq!(record.next_n(Period::Month(month("2026-11"))), 1);
        let decoded = SpaceRecord::decode(&record.encode().unwrap()).unwrap();
        assert_eq!(decoded.next_n(october), 2);
    }

    #[test]
    fn a_memory_key_is_its_own_period_and_old_records_read_as_month_keys() {
        let mut record = SpaceRecord::default();
        record.versions.push(SpaceVersion {
            version: 1,
            wrapped: Some(vec![1]),
        });
        let memory = Uuid::from_u128(7);
        record.data.push(DataKey {
            month: month("2026-10"),
            memory: Some(memory),
            n: 1,
            space_version: 1,
            wrapped: Some(vec![2]),
        });
        assert!(record.sealing(Period::Month(month("2026-10"))).is_none());
        assert!(record.sealing(Period::Memory(memory)).is_some());
        assert!(!record.rotation_due());
        if let Some(key) = record.data.first_mut() {
            key.wrapped = None;
        }
        assert!(record.rotation_due(), "destroyed under the live space key");
        let old = br#"{"glave_keys":1,"versions":[],"data":[{"month":"2026-09","n":1,"space_version":1,"wrapped":null}]}"#;
        let decoded = SpaceRecord::decode(old).unwrap();
        assert!(
            decoded
                .data_key(Period::Month(month("2026-09")), 1)
                .is_some()
        );
    }

    #[test]
    fn an_unknown_format_is_refused() {
        assert!(SpaceRecord::decode(br#"{"glave_keys":2,"versions":[],"data":[]}"#).is_err());
    }
}

//! The stored record of one space's keys: every space-key version and every data key, each
//! wrapped, or emptied once destroyed. Emptied entries stay so numbering never repeats.

use serde::{Deserialize, Serialize};

use crate::error::KeyError;
use crate::key_id::Month;

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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DataKey {
    pub(crate) month: Month,
    pub(crate) n: u32,
    pub(crate) space_version: u32,
    pub(crate) wrapped: Option<Vec<u8>>,
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

    /// The data key that seals `month`'s new content: its newest key not destroyed.
    pub(crate) fn sealing(&self, month: Month) -> Option<&DataKey> {
        self.data
            .iter()
            .filter(|key| key.month == month && key.wrapped.is_some())
            .max_by_key(|key| key.n)
    }

    pub(crate) fn data_key(&self, month: Month, n: u32) -> Option<&DataKey> {
        self.data
            .iter()
            .find(|key| key.month == month && key.n == n)
    }

    /// The number `month`'s next data key takes.
    pub(crate) fn next_n(&self, month: Month) -> u32 {
        next(
            self.data
                .iter()
                .filter(|key| key.month == month)
                .map(|key| key.n),
        )
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
            n: 1,
            space_version: 1,
            wrapped: None,
        });
        assert!(record.live().is_none());
        assert_eq!(record.next_version(), 2);
        assert!(record.sealing(month("2026-10")).is_none());
        assert_eq!(record.next_n(month("2026-10")), 2);
        assert_eq!(record.next_n(month("2026-11")), 1);
        let decoded = SpaceRecord::decode(&record.encode().unwrap()).unwrap();
        assert_eq!(decoded.next_n(month("2026-10")), 2);
    }

    #[test]
    fn an_unknown_format_is_refused() {
        assert!(SpaceRecord::decode(br#"{"glave_keys":2,"versions":[],"data":[]}"#).is_err());
    }
}

//! Unwrapped keys kept in memory, up to a fixed count; the oldest entry leaves first.

use std::collections::HashMap;
use std::sync::Mutex;

use glave_ports::{OrgId, Scope};

use crate::key_id::{KeyId, Month};
use crate::secret::Secret;

use super::hierarchy::SpaceKey;

/// What an entry is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Slot {
    Org(OrgId),
    Space(Scope, u32),
    /// The space-key version in use.
    Live(Scope),
    Data(Scope, KeyId),
    /// The data key that seals a month's new content.
    Sealing(Scope, Month),
}

impl Slot {
    fn scope(&self) -> Option<Scope> {
        match self {
            Self::Org(_) => None,
            Self::Space(scope, _)
            | Self::Live(scope)
            | Self::Data(scope, _)
            | Self::Sealing(scope, _) => Some(*scope),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Value {
    Key(Secret),
    Space(SpaceKey),
    Version(u32),
    Id(KeyId),
}

/// A bounded map of unwrapped keys. The lock is never held across an await.
#[derive(Debug)]
pub(crate) struct Cache {
    capacity: usize,
    entries: Mutex<Entries>,
}

#[derive(Debug, Default)]
struct Entries {
    next: u64,
    map: HashMap<Slot, (u64, Value)>,
}

impl Cache {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: Mutex::new(Entries::default()),
        }
    }

    pub(crate) fn get(&self, slot: &Slot) -> Option<Value> {
        let entries = self.entries.lock().ok()?;
        entries.map.get(slot).map(|(_, value)| value.clone())
    }

    pub(crate) fn put(&self, slot: Slot, value: Value) {
        if self.capacity == 0 {
            return;
        }
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        while entries.map.len() >= self.capacity && !entries.map.contains_key(&slot) {
            let oldest = entries
                .map
                .iter()
                .min_by_key(|(_, (stamp, _))| *stamp)
                .map(|(slot, _)| *slot);
            match oldest {
                Some(oldest) => entries.map.remove(&oldest),
                None => break,
            };
        }
        let stamp = entries.next;
        entries.next = stamp.wrapping_add(1);
        entries.map.insert(slot, (stamp, value));
    }

    /// Drops every key of `scope`, after its keys change.
    pub(crate) fn forget(&self, scope: Scope) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.map.retain(|slot, _| slot.scope() != Some(scope));
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.lock().map_or(0, |e| e.map.len())
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[test]
    fn the_cache_holds_at_most_its_capacity_and_drops_the_oldest_first() {
        let cache = Cache::new(2);
        let org = |n| OrgId::from_uuid(Uuid::from_u128(n));
        for n in 0..5 {
            cache.put(Slot::Org(org(n)), Value::Version(1));
        }
        assert_eq!(cache.len(), 2);
        assert!(cache.get(&Slot::Org(org(0))).is_none());
        assert!(cache.get(&Slot::Org(org(4))).is_some());
    }
}

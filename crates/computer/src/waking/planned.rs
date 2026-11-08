//! A workspace planned on a person's computer, and where it is open while the computer is awake.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError};

use glave_ports::computer::{ComputerId, Computers, WorkspaceId, WorkspaceInfo};

/// A workspace planned on the computer.
pub(super) struct Planned {
    pub(super) info: WorkspaceInfo,
    /// Where it is open, once a call needed it; opened again on a computer woken since.
    pub(super) real: Mutex<Option<Real>>,
    /// Held by the one call opening it, so two calls at once share one wake and one open.
    pub(super) opening: tokio::sync::Mutex<()>,
    /// Let go of while a command of its ran in the background there: kept for that command
    /// until the computer sleeps, which ends it, then forgotten.
    pub(super) closing: AtomicBool,
    /// Where its folder's history stood as it was first opened on the computer awake, which a
    /// plan made asleep can't know: what changes are read since when none is asked.
    pub(super) first_at: Mutex<Option<String>>,
}

impl Planned {
    /// Where its folder's history stood as it was first opened on the computer awake.
    pub(super) fn first_at(&self) -> Option<String> {
        self.first_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Keeps `at` as where its folder's history stood when first opened, unless it was opened
    /// before.
    pub(super) fn opened_at(&self, at: Option<&String>) {
        let mut first = self.first_at.lock().unwrap_or_else(PoisonError::into_inner);
        if first.is_none() {
            *first = at.cloned();
        }
    }

    pub(super) fn real(&self) -> Option<Real> {
        self.real
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// A planned workspace, open on the computer awake.
#[derive(Clone)]
pub(super) struct Real {
    pub(super) computers: Arc<dyn Computers>,
    pub(super) computer: ComputerId,
    pub(super) workspace: WorkspaceId,
}

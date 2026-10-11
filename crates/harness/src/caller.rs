//! Who asked, as the harness records it: the caller (`glave_identity::Caller`) only where
//! `specs/events.md` says to, and only when it is someone other than the thread's `for`.

use glave_event::{Principal, PrincipalId};

use crate::harness::{Harness, Thread};

impl<P, L, B, A> Harness<P, L, B, A> {
    /// `person` as a principal to record, unless it is `thread`'s `for`, which is implied.
    /// `None` is the local person, who never needs naming in the local person's threads.
    pub(crate) fn other_than_for(
        thread: &Thread,
        person: Option<PrincipalId>,
    ) -> Option<Principal> {
        person
            .filter(|person| Some(*person) != thread.for_)
            .map(Principal::person)
    }
}

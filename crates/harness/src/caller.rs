//! Who asked: the verified caller of a thread, a turn or a decision, as the session API
//! establishes it (`specs/session.md`, "Who is calling"). The harness records the caller only
//! where `specs/events.md` says to, and only when it is someone other than the thread's `for`.

use glave_event::{Auth, Principal, PrincipalId};

use crate::harness::{Harness, Thread};

/// Who asked. The default is the local person. A thread started by a caller is for them: its
/// `for` is the caller, or the local person by default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Caller {
    /// The person who asked; `None` for the local person.
    pub person: Option<PrincipalId>,
    /// How they signed in; `None` locally. Never a token.
    pub auth: Option<Auth>,
}

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

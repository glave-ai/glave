//! Who asked: the verified caller of a thread, a turn or a decision, as the session API
//! establishes it (`specs/session.md`, "Who is calling").

use glave_event::{Auth, PrincipalId};

/// Who asked. The default is the local person. A thread started by a caller is for them: its
/// `for` is the caller, or the local person by default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Caller {
    /// The person who asked; `None` for the local person.
    pub person: Option<PrincipalId>,
    /// How they signed in; `None` locally. Never a token.
    pub auth: Option<Auth>,
}

//! Who asked: the verified caller of a thread, a turn or a decision, as the session API
//! establishes it (`specs/session.md`, "Who is calling").

use glave_event::{Auth, PrincipalId, Profile, TraceId};

use crate::OrgId;

/// Who asked. The default is the local person. A thread started by a caller is for them: its
/// `for` is the caller, or the local person by default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Caller {
    /// The person who asked; `None` for the local person.
    pub person: Option<PrincipalId>,
    /// How they signed in; `None` locally. Never a token.
    pub auth: Option<Auth>,
    /// The W3C trace the request belongs to, when the caller sent one (`traceparent`) or a
    /// tracer started one: recorded with the turn, linking it to OpenTelemetry spans.
    pub trace: Option<TraceId>,
    /// What the identity provider's token says of them besides who they are, when they signed
    /// in with one: kept in the organization's people ledger (`specs/model.md`, "Roles").
    pub profile: Option<Profile>,
    /// The organization the request works in, when it is not the server's own: one the caller
    /// joined, established by the transport with who they are and never taken from the
    /// request unchecked (`specs/session.md`, "Who is calling"). `None` is the server's own
    /// organization, the local person's.
    pub org: Option<OrgId>,
}

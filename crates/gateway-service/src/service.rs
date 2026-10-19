//! The service's parts: the gateway, the policy each caller is held to, sign-in, the meter,
//! and the catalog's servers; and the few things every surface asks of them.

use std::sync::Arc;

use glave_event::PrincipalId;
use glave_gateway::Gateway;
use glave_identity::signin::{SignIn, SignInError};
use glave_mcp::McpServer;
use glave_ports::{Caller, Clock, Entropy};
use glave_ports::{EventLog, Scope};

use crate::meter::Meter;

/// What each caller may use. Asked on every call, with the caller as sign-in proved them, so a
/// policy can tell people apart.
pub trait Policy: Send + Sync {
    /// Why `caller` may not use the model `target` (`provider/model`), if they may not: the
    /// rule and where it came from.
    fn refuses_model(&self, caller: &Caller, target: &str) -> Option<String>;

    /// Why `caller` may not use `tool` of the catalog's server `server`, if they may not.
    fn refuses_tool(&self, caller: &Caller, server: &str, tool: &str) -> Option<String>;
}

/// Where the service says what went wrong outside any request: a record it could not write, a
/// catalog server that would not list. Never content, never a key.
pub type Notices = Arc<dyn Fn(&str) + Send + Sync>;

/// Everything the service is made of.
pub struct Wiring<L> {
    /// The gateway calls go through, with its routes, keys and budgets. Its person budget
    /// applies to each caller, per day.
    pub gateway: Gateway,
    pub policy: Arc<dyn Policy>,
    pub signin: SignIn,
    /// Where calls are recorded, in `scope`: each caller's in a stream of their own.
    pub log: L,
    pub scope: Scope,
    pub clock: Box<dyn Clock>,
    pub entropy: Box<dyn Entropy>,
    /// The catalog's servers, started by whoever wires the service, served at `/mcp`.
    pub catalog: Vec<Arc<McpServer>>,
    /// Values never to show anyone, such as the providers' keys: replaced in every message
    /// the service sends.
    pub secrets: Vec<String>,
    pub notices: Notices,
}

/// The gateway as a service.
pub struct Service<L> {
    pub(crate) gateway: Gateway,
    pub(crate) policy: Arc<dyn Policy>,
    signin: SignIn,
    pub(crate) meter: Meter<L>,
    pub(crate) catalog: Vec<Arc<McpServer>>,
    secrets: Vec<String>,
    pub(crate) notices: Notices,
}

/// A caller as sign-in proved them, and the person they are.
#[derive(Debug, Clone)]
pub(crate) struct Verified {
    pub(crate) caller: Caller,
    pub(crate) person: PrincipalId,
}

impl<L: EventLog> Service<L> {
    pub fn new(wiring: Wiring<L>) -> Arc<Self> {
        Arc::new(Self {
            gateway: wiring.gateway,
            policy: wiring.policy,
            signin: wiring.signin,
            meter: Meter::new(wiring.log, wiring.scope, wiring.clock, wiring.entropy),
            catalog: wiring.catalog,
            secrets: wiring.secrets,
            notices: wiring.notices,
        })
    }

    /// The caller a token proves.
    pub(crate) async fn verify(&self, token: &str) -> Result<Verified, SignInError> {
        let caller = self.signin.verify(token).await?;
        let person = caller
            .person
            .ok_or_else(|| SignInError::Invalid("the token names no person".into()))?;
        Ok(Verified { caller, person })
    }

    /// `text` with every secret replaced.
    pub(crate) fn redact(&self, text: &str) -> String {
        glave_ports::auth::redact(text, self.secrets.iter().map(String::as_str))
    }

    pub(crate) fn notice(&self, message: &str) {
        (self.notices)(&self.redact(message));
    }
}

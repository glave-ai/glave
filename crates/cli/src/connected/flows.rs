//! Sign-ins waiting for the person's browser: started with the authorization server's page to
//! open, and finished when the browser comes back to glave with the sign-in's `state`, which
//! only that browser was given. The code it brings is traded for a token in a task the host
//! owns, so a browser that goes away mid-way never leaves a code spent and its tokens unkept;
//! the client that started the sign-in waits for it with `connection/poll`.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use glave_event::{At, ConnectionConnected, Principal, Secret};
use glave_mcp::{SignIn, SignInError};
use glave_ports::{Clock as _, SystemClock};
use glave_session::Returned;
use glave_session::protocol::{ApiError, ConnectionInfo, ConnectionStatus, SignInLink, code};
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::Connected;
use crate::accounts::Ledger;
use crate::credentials::{new_flow, new_id};

/// Sign-ins waiting at once; a new one past this ends the oldest.
const FLOWS: usize = 4;
/// How long a sign-in waits for the browser.
const LASTS: Duration = Duration::from_mins(10);
/// The longest one poll waits.
const POLL: Duration = Duration::from_secs(10);
/// The longest the browser waits for its sign-in to finish.
const FINISH: Duration = Duration::from_secs(60);

/// How a sign-in ended, once it has.
type Outcome = Option<Result<ConnectionInfo, ApiError>>;

/// One sign-in waiting.
struct Flow {
    id: String,
    url: String,
    /// What the person calls the server.
    name: String,
    state: String,
    /// Until the browser comes back.
    signin: Option<SignIn>,
    started: Instant,
    ended: Arc<watch::Sender<Outcome>>,
}

/// The sign-ins waiting, and the codes being traded.
#[derive(Default)]
pub(super) struct Flows {
    waiting: Mutex<Vec<Flow>>,
    trading: Mutex<JoinSet<()>>,
}

impl Flows {
    /// The sign-ins still waiting.
    fn waiting(&self) -> MutexGuard<'_, Vec<Flow>> {
        let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        waiting.retain(|flow| flow.started.elapsed() < LASTS);
        waiting
    }
}

impl Connected {
    /// Starts signing in to the person's remote server `server`, by its name in their settings
    /// or its address, the browser to come back to `redirect`. Nothing is asked of a server
    /// their settings do not name, and none starts with nowhere for the browser to come back
    /// to.
    pub(super) async fn started(
        &self,
        server: String,
        redirect: Option<String>,
    ) -> Result<Option<SignInLink>, ApiError> {
        let (url, name) = self.resolved(&server).await.ok_or_else(|| {
            ApiError::new(
                code::NO_SERVER,
                format!("none of your connections is called {server:?} or is at that address"),
            )
        })?;
        let Some(redirect) = redirect else {
            return Ok(None);
        };
        let (signin, page) = SignIn::start(&url, &name, &redirect)
            .await
            .map_err(|error| refused(&error, &name))?;
        let id = new_flow().map_err(|e| ApiError::internal(&e))?;
        let mut waiting = self.flows.waiting();
        if waiting.len() >= FLOWS {
            waiting.remove(0);
        }
        waiting.push(Flow {
            id: id.clone(),
            url,
            name,
            state: signin.state().to_owned(),
            signin: Some(signin),
            started: Instant::now(),
            ended: Arc::new(watch::channel(None).0),
        });
        Ok(Some(SignInLink {
            flow: id,
            url: page,
        }))
    }

    /// Waits a little for the sign-in `flow`: the connection once signed in to, none while it
    /// waits. One that ended stays until it lapses, so a page loaded again says how it ended.
    pub(super) async fn polled(&self, flow: String) -> Result<Option<ConnectionInfo>, ApiError> {
        let mut ended = {
            let waiting = self.flows.waiting();
            let found = waiting.iter().find(|waiting| waiting.id == flow);
            found
                .ok_or_else(|| {
                    ApiError::new(
                        code::NO_CREDENTIAL,
                        "glave is no longer waiting for that sign-in: start it again",
                    )
                })?
                .ended
                .subscribe()
        };
        let waited = tokio::time::timeout(POLL, ended.wait_for(Option::is_some)).await;
        let outcome = match waited {
            Ok(Ok(outcome)) => outcome.clone(),
            _ => None,
        };
        outcome.map_or(Ok(None), |outcome| outcome.map(Some))
    }

    /// Finishes the sign-in the browser came back from: what to tell the person there.
    pub(super) async fn finished(&self, returned: Returned) -> String {
        let found = self
            .flows
            .waiting()
            .iter_mut()
            .find(|flow| flow.state == returned.state)
            .map(|flow| {
                let named = (flow.url.clone(), flow.name.clone());
                (named, flow.signin.take(), Arc::clone(&flow.ended))
            });
        let Some(((url, name), signin, ended)) = found else {
            return "glave is no longer waiting for this sign-in. Sign in again.".to_owned();
        };
        // The page loaded again: the browser came back already.
        let Some(signin) = signin else {
            let outcome = ended.borrow().clone();
            return told(&name, outcome);
        };
        let (Some(code), None) = (returned.code, &returned.error) else {
            let why = match returned.error {
                Some(_) => "the sign-in was declined",
                None => "the sign-in came back with nothing to finish it",
            };
            let said = format!("glave was not signed in to {name}: {why}");
            let outcome = Some(Err(ApiError::new(code::HOST, said)));
            ended.send_replace(outcome.clone());
            return told(&name, outcome);
        };
        let mut waiting = ended.subscribe();
        let trade = Trade {
            ledger: self.ledger.clone(),
            url,
            name: name.clone(),
        };
        let issuer = returned.issuer;
        let traded = async move {
            let name = trade.name.clone();
            let traded = trade.traded(signin, &code, issuer.as_deref()).await;
            // The sign-in's own refusals say what to do; anything else is glave's.
            let traded = traded.map_err(|error| {
                if error.code == code::HOST {
                    return error;
                }
                let said = format!(
                    "glave could not finish signing in to {name}: {}",
                    error.message
                );
                ApiError::new(error.code, said)
            });
            ended.send_replace(Some(traded));
        };
        {
            let mut trading = self
                .flows
                .trading
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            while trading.try_join_next().is_some() {}
            trading.spawn(traded);
        }
        let waited = tokio::time::timeout(FINISH, waiting.wait_for(Option::is_some)).await;
        let outcome = match waited {
            Ok(Ok(outcome)) => outcome.clone(),
            _ => None,
        };
        told(&name, outcome)
    }
}

/// What the page the browser came back to says of the sign-in to `name`, which ended with
/// `outcome`, or has not ended yet.
fn told(name: &str, outcome: Outcome) -> String {
    match outcome {
        Some(Ok(_)) => format!("Signed in to {name}. You can close this tab and go back to glave."),
        Some(Err(error)) => format!("{}. You can close this tab.", error.message),
        None => format!("glave is still signing in to {name}: go back to glave to see it."),
    }
}

/// Trading a code for a sign-in to the server at `url`, which the person calls `name`.
struct Trade {
    ledger: Ledger,
    url: String,
    name: String,
}

impl Trade {
    /// The sign-in finished with `code`, from the authorization server `issuer`, and kept.
    async fn traded(
        self,
        signin: SignIn,
        code: &Secret,
        issuer: Option<&str>,
    ) -> Result<ConnectionInfo, ApiError> {
        let granted = signin
            .finish(code.expose(), issuer)
            .await
            .map_err(|error| refused(&error, &self.name))?;
        let connection = ConnectionConnected {
            connection: new_id().map_err(|e| ApiError::internal(&e))?,
            url: self.url,
            token: granted.token,
            expires: expires(granted.lasts),
            refresh: granted.refresh,
            client: granted.client,
            by: Principal::person(self.ledger.person()),
        };
        self.ledger.keep_connection(&connection).await?;
        Ok(ConnectionInfo {
            url: connection.url,
            name: Some(self.name),
            status: ConnectionStatus::SignedIn,
            usage: None,
        })
    }
}

/// When a token that lasts `lasts` from now expires.
pub(super) fn expires(lasts: Option<Duration>) -> Option<At> {
    let lasts = jiff::SignedDuration::try_from(lasts?).ok()?;
    SystemClock.now().checked_add(lasts).ok().map(At::from)
}

/// A sign-in to the server the person calls `name` that could not start or finish, in words
/// they can act on; rmcp's own go to the log.
fn refused(error: &SignInError, name: &str) -> ApiError {
    tracing::warn!("{error}");
    let said = match error {
        SignInError::NotOffered { .. } => {
            format!("{name} asks no one to sign in: it works as it is")
        }
        SignInError::Unreachable { .. } => {
            format!("glave could not reach the sign-in for {name}: try again in a while")
        }
        SignInError::Refused { .. } => format!(
            "the sign-in for {name} refused glave: sign in again, or ask whoever runs {name}"
        ),
        SignInError::Unregistered { .. } => format!(
            "glave can't sign in to {name} yet: its sign-in doesn't let glave register there"
        ),
    };
    ApiError::new(code::HOST, said)
}

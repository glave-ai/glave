//! Sign-in on network transports (`specs/session.md`, "Signing in"): OIDC access tokens,
//! checked against the identity provider's published keys. A token is signed with RS256 or
//! ES256 by a key in the issuer's JWKS, names a trusted issuer and this server's audience, and
//! is within its lifetime. The verified caller is a person whose id comes from the token's
//! issuer and subject. And local sign-in: a program on the person's own computer presents the
//! server's local token from there, and is the local person, as is a browser there that traded
//! a link's code for a browser session, or a browser anywhere signed in with a passkey through
//! the server's public origin. Which way a request came in is its [`Door`]. A token is never
//! logged or stored.

mod accounts;
mod browsers;
mod keys;
mod local;

use std::sync::Arc;

use glave_event::{Auth, AuthMethod, PrincipalId, Profile};
use glave_ports::auth::safe_to_send;
use glave_ports::{Caller, Clock};
use jsonwebtoken::{Algorithm, Validation, decode, decode_header};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

pub use self::accounts::Accounts;
pub use self::browsers::{
    AcceptedPasskeys, BrowserError, Browsers, Ceremony, Checked, Discovered, PasskeyInfo, Proof,
    Session, UnknownPasskey, kept, one_time_code, same_kept,
};
use self::browsers::{SESSION_PREFIX, account_of};
use self::keys::Keys;
use self::local::LOCAL_PREFIX;
pub use self::local::{LOCAL_BYTES, LocalToken};

/// How far the clocks of the server and the identity provider may disagree, in seconds.
const SKEW: f64 = 60.0;

/// Which way a request came in, as its transport saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Door {
    /// From this computer, to the server's own address.
    Here,
    /// Through the server's public origin: a tunnel, a proxy, or its HTTPS of its own.
    Public,
    /// From another computer, to a server with no public origin.
    Elsewhere,
}

/// Who may sign in: the identity providers trusted, and the audience their tokens must name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trust {
    /// Each issuer's URL exactly as its tokens' `iss` says it, such as
    /// `https://accounts.google.com`. `https`, or `http` only on this computer.
    pub issuers: Vec<String>,
    /// The `aud` a token must name: this server, as the identity provider knows it.
    pub audience: String,
}

/// Why a request was not signed in. Says what is wrong, never what the token was.
#[derive(Debug, Error)]
pub enum SignInError {
    #[error(
        "no sign-in: send an OIDC access token, or from this computer the local token in \
         serve.token, as `Authorization: Bearer <token>`"
    )]
    Missing,
    #[error("the token is not a JWT signed with RS256 or ES256: {0}")]
    Malformed(String),
    #[error("the token's issuer is not one this server trusts")]
    UnknownIssuer,
    #[error("could not get the signing keys of {issuer}: {reason}")]
    Keys { issuer: String, reason: String },
    #[error("the token was not signed by a key {issuer} publishes")]
    UnknownKey { issuer: String },
    #[error("the token is not valid: {0}")]
    Invalid(String),
    #[error("the token has expired")]
    Expired,
    #[error("the token is not valid yet")]
    NotYet,
    #[error("the issuer {0:?} is not an https URL (http is allowed only on this computer)")]
    Insecure(String),
    #[error("no issuer is trusted: a network listener never runs without sign-in")]
    NoIssuers,
    #[error("could not make the HTTP client that fetches keys: {0}")]
    Client(String),
    #[error("the local token is accepted only from this computer")]
    NotLoopback,
    #[error("that is not this server's local token: read it again from serve.token")]
    WrongLocalToken,
    #[error("this browser's sign-in has ended: open glave again to sign in")]
    EndedSession,
    #[error(
        "this browser was signed in on the computer glave runs on, and its sign-in works only \
         there: sign in with a passkey here"
    )]
    OnlyThere,
    #[error("this account belongs to no organization here: ask its admins for an invite")]
    NoOrganization,
}

/// Checks sign-in tokens for one server. Keys are fetched from each issuer when first needed,
/// cached, and fetched again when a token names a key not seen yet.
pub struct SignIn {
    audience: String,
    issuers: Vec<Keys>,
    clock: Box<dyn Clock>,
    /// The local token, accepted from this computer as the local person.
    local: Option<LocalToken>,
    /// Browser sessions, accepted as the local person: a link's from this computer, a
    /// passkey's through the public origin too.
    browsers: Option<Arc<Browsers>>,
    /// glave accounts' sessions, each its person's.
    accounts: Option<Arc<Accounts>>,
}

/// The claims glave reads. `iss` and `aud` are checked while decoding.
#[derive(Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    exp: f64,
    #[serde(default)]
    nbf: Option<f64>,
    #[serde(default)]
    sid: Option<String>,
    #[serde(default)]
    name: Option<serde_json::Value>,
    #[serde(default)]
    email: Option<serde_json::Value>,
    #[serde(default)]
    groups: Option<serde_json::Value>,
}

/// The most groups kept of a token's, as Entra ID sends at most 200 before it sends a link
/// instead.
const GROUPS: usize = 256;
/// The longest name, email or group kept of a token's, in bytes; a longer one is left out.
const LONGEST: usize = 256;

/// A claim kept when it is text of at most [`LONGEST`] bytes, not empty: a token's other
/// claims never refuse it.
fn text(claim: Option<&serde_json::Value>) -> Option<String> {
    claim
        .and_then(serde_json::Value::as_str)
        .filter(|text| !text.is_empty() && text.len() <= LONGEST)
        .map(str::to_owned)
}

/// What the token says of the person besides who they are: their name, email and groups,
/// where it says them as text.
fn profile(claims: &Claims) -> Profile {
    let groups = claims
        .groups
        .as_ref()
        .and_then(serde_json::Value::as_array)
        .map(|groups| {
            groups
                .iter()
                .filter_map(|group| text(Some(group)))
                .take(GROUPS)
                .collect()
        })
        .unwrap_or_default();
    Profile {
        name: text(claims.name.as_ref()),
        email: text(claims.email.as_ref()),
        groups,
    }
}

/// Only the issuer, read before the signature is checked to know whose keys check it.
#[derive(Deserialize)]
struct Issuer {
    iss: String,
}

impl SignIn {
    pub fn new(trust: Trust, clock: Box<dyn Clock>) -> Result<Self, SignInError> {
        if trust.issuers.is_empty() {
            return Err(SignInError::NoIssuers);
        }
        let http = keys::client()?;
        let issuers = trust
            .issuers
            .into_iter()
            .map(|issuer| {
                if safe_to_send(&issuer, &[]) {
                    Ok(Keys::new(issuer, http.clone()))
                } else {
                    Err(SignInError::Insecure(issuer))
                }
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            audience: trust.audience,
            issuers,
            clock,
            local: None,
            browsers: None,
            accounts: None,
        })
    }

    /// Sign-in with the local token alone: only a browser on this computer can sign in.
    pub fn local(token: LocalToken, clock: Box<dyn Clock>) -> Self {
        Self {
            audience: String::new(),
            issuers: Vec::new(),
            clock,
            local: Some(token),
            browsers: None,
            accounts: None,
        }
    }

    /// Accepts `token` too, from this computer, as the local person.
    #[must_use]
    pub fn with_local(mut self, token: LocalToken) -> Self {
        self.local = Some(token);
        self
    }

    /// Accepts the sessions of `browsers` too, as the local person.
    #[must_use]
    pub fn with_browsers(mut self, browsers: Arc<Browsers>) -> Self {
        self.browsers = Some(browsers);
        self
    }

    /// Accepts the sessions of `accounts` too, each as its person.
    #[must_use]
    pub fn with_accounts(mut self, accounts: Arc<Accounts>) -> Self {
        self.accounts = Some(accounts);
        self
    }

    /// The person whose glave account began the session `token`, in the first organization
    /// they joined, when it comes in at `door`.
    async fn account(&self, token: &str, door: Door) -> Result<Caller, SignInError> {
        let accounts = self.accounts.as_ref().ok_or(SignInError::EndedSession)?;
        if door == Door::Elsewhere {
            return Err(SignInError::OnlyThere);
        }
        let signed = accounts
            .verify(token, door == Door::Here)
            .await
            .map_err(|_| SignInError::EndedSession)?;
        let org = signed
            .orgs
            .first()
            .copied()
            .ok_or(SignInError::NoOrganization)?;
        Ok(Caller {
            person: Some(signed.person),
            auth: Some(Auth {
                method: AuthMethod::Account,
                issuer: None,
                subject: None,
                session: Some(signed.session),
            }),
            trace: None,
            profile: None,
            org: Some(org),
            from: None,
        })
    }

    /// The caller a token proves when it comes in at `door`: the local person for the local
    /// token from this computer, or a browser session (a link's from this computer, a
    /// passkey's through the public origin too); else whoever an OIDC token names.
    pub async fn verify_from(&self, token: &str, door: Door) -> Result<Caller, SignInError> {
        if account_of(token).is_some() {
            return self.account(token, door).await;
        }
        if token.starts_with(SESSION_PREFIX) {
            let browsers = self.browsers.as_ref().ok_or(SignInError::EndedSession)?;
            // Checked first, so a session is never renewed from elsewhere.
            if door == Door::Elsewhere {
                return Err(SignInError::OnlyThere);
            }
            let session = browsers
                .verify(token, door == Door::Here)
                .await
                .map_err(|refused| match refused {
                    BrowserError::Elsewhere => SignInError::OnlyThere,
                    _ => SignInError::EndedSession,
                })?;
            return Ok(local_person(Some(session)));
        }
        if token.starts_with(LOCAL_PREFIX) {
            if !self
                .local
                .as_ref()
                .is_some_and(|local| local.matches(token))
            {
                return Err(SignInError::WrongLocalToken);
            }
            if door != Door::Here {
                return Err(SignInError::NotLoopback);
            }
            return Ok(local_person(None));
        }
        if self.issuers.is_empty() {
            return Err(SignInError::UnknownIssuer);
        }
        self.verify(token).await
    }

    /// The caller a token proves, or why it proves no one.
    pub async fn verify(&self, token: &str) -> Result<Caller, SignInError> {
        let header = decode_header(token).map_err(|e| SignInError::Malformed(e.to_string()))?;
        if !matches!(header.alg, Algorithm::RS256 | Algorithm::ES256) {
            return Err(SignInError::Malformed(format!(
                "signed with {:?}",
                header.alg
            )));
        }
        let named: Issuer = jsonwebtoken::dangerous::insecure_decode_claims(token)
            .map_err(|e| SignInError::Malformed(e.to_string()))?;
        let issuer = self
            .issuers
            .iter()
            .find(|keys| keys.issuer() == named.iss)
            .ok_or(SignInError::UnknownIssuer)?;
        let now = self.clock.now();
        let key = issuer.key(header.kid.as_deref(), header.alg, now).await?;
        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[issuer.issuer()]);
        validation.set_audience(&[&self.audience]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        // Lifetimes are checked below against the injected clock.
        validation.validate_exp = false;
        validation.validate_nbf = false;
        let claims = decode::<Claims>(token, &key, &validation)
            .map_err(|e| SignInError::Invalid(e.to_string()))?
            .claims;
        #[expect(
            clippy::cast_precision_loss,
            reason = "seconds since 1970 fit an f64 exactly for millions of years"
        )]
        let seconds = now.as_second() as f64;
        if seconds > claims.exp + SKEW {
            return Err(SignInError::Expired);
        }
        if claims.nbf.is_some_and(|nbf| seconds + SKEW < nbf) {
            return Err(SignInError::NotYet);
        }
        if claims.sub.is_empty() {
            return Err(SignInError::Invalid("the subject is empty".into()));
        }
        let profile = profile(&claims);
        Ok(Caller {
            person: Some(person(&claims.iss, &claims.sub)?),
            profile: Some(profile),
            auth: Some(Auth {
                method: AuthMethod::Oidc,
                issuer: Some(claims.iss),
                subject: Some(claims.sub),
                session: claims.sid,
            }),
            trace: None,
            org: None,
            from: None,
        })
    }
}

/// The local person, signed in locally: with the local token, or in the browser `session`.
fn local_person(session: Option<String>) -> Caller {
    Caller {
        person: None,
        auth: Some(Auth {
            method: AuthMethod::Local,
            issuer: None,
            subject: None,
            session,
        }),
        trace: None,
        profile: None,
        org: None,
        from: None,
    }
}

/// The person an issuer's subject is: the same id on every server, with no table to keep. A
/// UUID v7 at time zero whose random bits are the start of SHA-256 over the issuer and the
/// subject. An identity source (SCIM) that assigns ids replaces this.
pub fn person(issuer: &str, subject: &str) -> Result<PrincipalId, SignInError> {
    let mut hash = Sha256::new();
    hash.update(b"glave person\0");
    hash.update(issuer.as_bytes());
    hash.update(b"\0");
    hash.update(subject.as_bytes());
    let digest = hash.finalize();
    let mut random = [0u8; 10];
    for (byte, from) in random.iter_mut().zip(digest.iter()) {
        *byte = *from;
    }
    PrincipalId::new(jiff::Timestamp::UNIX_EPOCH, random)
        .map_err(|e| SignInError::Invalid(e.to_string()))
}

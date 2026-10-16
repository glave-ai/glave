//! Signing in to GitHub in the browser, through the GitHub App `[github]` names: GitHub's device
//! flow. glave asks GitHub for a code, the person enters it on GitHub's page and approves the
//! app, and glave, polling no faster than GitHub allows, receives a user token. The device code
//! that redeems it never leaves glave: a client names the sign-in by a flow id of its own. A
//! GitHub App's user token lasts eight hours and comes with a refresh token, which renews it
//! without a client secret, since it came from the device flow.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use glave_config::policy::GithubFile;
use glave_event::{At, Renewal, Secret};
use glave_ports::auth::safe_to_send;
use glave_session::protocol::{ApiError, DeviceCode, code};
use serde_json::Value;

use super::check::read;

/// Sign-ins waiting at once; a new one past this ends the oldest.
const FLOWS: usize = 4;
/// The longest one poll waits before asking GitHub.
const LONGEST_WAIT: Duration = Duration::from_secs(10);

/// The GitHub App sign-ins go through.
#[derive(Debug, Clone)]
pub struct Github {
    client_id: String,
    /// Where GitHub is: `https://github.com`, or a GitHub Enterprise Server.
    pub url: String,
    /// The app's name in its URL, for the page where the person chooses its repositories.
    app: Option<String>,
}

/// What GitHub granted: a user token, and how to renew it when it expires.
pub struct Grant {
    pub token: Secret,
    pub renewal: Option<Renewal>,
}

/// One sign-in waiting for the person.
struct Flow {
    device_code: Secret,
    interval: Duration,
    next: Instant,
    expires: Instant,
}

/// The sign-ins waiting, by flow id.
#[derive(Default)]
pub struct Flows(Mutex<HashMap<String, Flow>>);

/// How a poll went.
pub enum Polled {
    Waiting,
    Granted(Grant),
}

impl Github {
    /// The app `[github]` names.
    pub fn new(file: &GithubFile) -> Self {
        Self {
            client_id: file.client_id.trim().to_owned(),
            url: file.url.as_deref().map_or_else(
                || "https://github.com".to_owned(),
                |url| url.trim().trim_end_matches('/').to_owned(),
            ),
            app: file.app.as_deref().map(str::trim).map(str::to_owned),
        }
    }

    /// The page where the person installs the app on their account or organizations and
    /// chooses its repositories, when the app's name is known.
    pub fn install(&self) -> Option<String> {
        let app = self.app.as_ref()?;
        let apps = if self.url == "https://github.com" {
            "apps"
        } else {
            "github-apps"
        };
        Some(format!("{}/{apps}/{app}/installations/new", self.url))
    }

    /// Asks GitHub for a code for the person to enter, and keeps the sign-in as `flow`.
    pub async fn start(
        &self,
        http: &reqwest::Client,
        flows: &Flows,
        flow: String,
    ) -> Result<DeviceCode, ApiError> {
        let form = format!("client_id={}", encoded(&self.client_id));
        let answer = post(http, &format!("{}/login/device/code", self.url), form).await?;
        let text = |key: &str| answer.get(key).and_then(Value::as_str).map(str::to_owned);
        let number = |key: &str| answer.get(key).and_then(Value::as_u64);
        let (Some(device_code), Some(code), Some(url), Some(expires_in)) = (
            text("device_code"),
            text("user_code"),
            text("verification_uri"),
            number("expires_in"),
        ) else {
            return Err(refused(&answer, "GitHub would not start the sign-in"));
        };
        if !safe_to_send(&url, &[]) {
            return Err(ApiError::new(
                code::HOST,
                "GitHub gave a sign-in page that is not https",
            ));
        }
        let now = Instant::now();
        let waiting = Flow {
            device_code: Secret::new(device_code),
            interval: Duration::from_secs(number("interval").unwrap_or(5)),
            next: now,
            expires: now + Duration::from_secs(expires_in),
        };
        flows.keep(flow.clone(), waiting);
        Ok(DeviceCode {
            flow,
            code,
            url,
            expires_in,
        })
    }

    /// Waits until GitHub allows the next poll of `flow`, then asks once: the grant once the
    /// person approved the app.
    pub async fn poll(
        &self,
        http: &reqwest::Client,
        flows: &Flows,
        flow: &str,
    ) -> Result<Polled, ApiError> {
        let (device_code, next) = flows.waiting(flow)?;
        let wait = next.saturating_duration_since(Instant::now());
        tokio::time::sleep(wait.min(LONGEST_WAIT)).await;
        let form = format!(
            "client_id={}&device_code={}&grant_type={}",
            encoded(&self.client_id),
            encoded(device_code.expose()),
            encoded("urn:ietf:params:oauth:grant-type:device_code")
        );
        let answer = post(http, &self.token_url(), form).await?;
        if let Some(grant) = granted(&answer) {
            flows.end(flow);
            return Ok(Polled::Granted(grant));
        }
        let error = answer.get("error").and_then(Value::as_str).unwrap_or("");
        match error {
            "authorization_pending" => {
                flows.after(flow, None);
                return Ok(Polled::Waiting);
            }
            "slow_down" => {
                let asked = answer.get("interval").and_then(Value::as_u64);
                flows.after(flow, Some(asked.map_or(5, |secs| secs.max(5))));
                return Ok(Polled::Waiting);
            }
            _ => flows.end(flow),
        }
        Err(match error {
            "expired_token" => ApiError::new(
                code::HOST,
                "the code expired before it was entered: start again",
            ),
            "access_denied" => ApiError::new(code::HOST, "the sign-in was declined on GitHub"),
            _ => refused(&answer, "GitHub did not finish the sign-in"),
        })
    }

    /// A new token for the one `refresh` renews, and a new refresh token with it.
    pub async fn refresh(
        &self,
        http: &reqwest::Client,
        refresh: &Secret,
    ) -> Result<Grant, ApiError> {
        let form = format!(
            "client_id={}&grant_type=refresh_token&refresh_token={}",
            encoded(&self.client_id),
            encoded(refresh.expose())
        );
        let answer = post(http, &self.token_url(), form).await?;
        granted(&answer).ok_or_else(|| refused(&answer, "GitHub would not renew the sign-in"))
    }

    fn token_url(&self) -> String {
        format!("{}/login/oauth/access_token", self.url)
    }
}

/// The grant in GitHub's answer, when it holds one: the token, and for a token that expires,
/// when, and the refresh token that renews it.
fn granted(answer: &Value) -> Option<Grant> {
    let token = answer.get("access_token").and_then(Value::as_str)?;
    let expires_in = answer.get("expires_in").and_then(Value::as_i64);
    let refresh = answer.get("refresh_token").and_then(Value::as_str);
    let renewal = match (expires_in, refresh) {
        (Some(seconds), Some(refresh)) => {
            let expires = jiff::Timestamp::now()
                .checked_add(jiff::SignedDuration::from_secs(seconds))
                .ok()?;
            Some(Renewal {
                expires: At::from(expires),
                refresh: Secret::new(refresh),
            })
        }
        _ => None,
    };
    Some(Grant {
        token: Secret::new(token),
        renewal,
    })
}

impl Flows {
    /// Keeps `flow`, ending the oldest when too many wait.
    fn keep(&self, id: String, flow: Flow) {
        let Ok(mut flows) = self.0.lock() else {
            return;
        };
        let now = Instant::now();
        flows.retain(|_, waiting| waiting.expires > now);
        if flows.len() >= FLOWS
            && let Some(oldest) = flows
                .iter()
                .min_by_key(|(_, waiting)| waiting.expires)
                .map(|(id, _)| id.clone())
        {
            flows.remove(&oldest);
        }
        flows.insert(id, flow);
    }

    /// The device code of `flow` and when it may next be polled; refused when it is not
    /// waiting, or has expired.
    fn waiting(&self, id: &str) -> Result<(Secret, Instant), ApiError> {
        let mut flows = self.0.lock().map_err(|_| no_flow())?;
        let flow = flows.get(id).ok_or_else(no_flow)?;
        if flow.expires <= Instant::now() {
            flows.remove(id);
            return Err(ApiError::new(
                code::HOST,
                "the code expired before it was entered: start again",
            ));
        }
        Ok((flow.device_code.clone(), flow.next))
    }

    /// Sets when `flow` may be polled next: after its interval, or `slower` seconds.
    fn after(&self, id: &str, slower: Option<u64>) {
        if let Ok(mut flows) = self.0.lock()
            && let Some(flow) = flows.get_mut(id)
        {
            if let Some(secs) = slower {
                flow.interval = flow.interval.max(Duration::from_secs(secs));
            }
            flow.next = Instant::now() + flow.interval;
        }
    }

    fn end(&self, id: &str) {
        if let Ok(mut flows) = self.0.lock() {
            flows.remove(id);
        }
    }
}

/// Posts `form` to GitHub at `url`, answered as JSON.
async fn post(http: &reqwest::Client, url: &str, form: String) -> Result<Value, ApiError> {
    let sent = http
        .post(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(form)
        .send()
        .await
        .map_err(|_| {
            ApiError::new(
                code::HOST,
                "glave could not reach GitHub: check your connection",
            )
        })?;
    read(sent).await.ok_or_else(|| {
        ApiError::new(
            code::HOST,
            "GitHub did not answer as glave expected: try again",
        )
    })
}

/// GitHub's refusal, with its own description when it gave one.
fn refused(answer: &Value, what: &str) -> ApiError {
    let said = answer
        .get("error_description")
        .or_else(|| answer.get("error"))
        .and_then(Value::as_str);
    ApiError::new(
        code::HOST,
        match said {
            Some(said) => format!("{what}: {said}"),
            None => format!("{what}: try again"),
        },
    )
}

/// `text` as a form value: every byte but letters, digits and `-._~` percent-encoded.
fn encoded(text: &str) -> String {
    text.bytes().fold(String::new(), |mut out, byte| {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
        out
    })
}

fn no_flow() -> ApiError {
    ApiError::new(
        code::NO_CREDENTIAL,
        "that sign-in is not waiting any more: start again",
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn form_values_are_percent_encoded() {
        assert_eq!(encoded("a b:c"), "a%20b%3Ac");
        assert_eq!(encoded("Iv1.abc-_~"), "Iv1.abc-_~");
        assert_eq!(encoded("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn a_grant_that_expires_keeps_how_to_renew_it() {
        let lasting = granted(&json!({"access_token": "gho_x", "token_type": "bearer"})).unwrap();
        assert!(lasting.renewal.is_none());
        let expiring = granted(&json!({
            "access_token": "ghu_x", "expires_in": 28800,
            "refresh_token": "ghr_x", "refresh_token_expires_in": 15_897_600
        }))
        .unwrap();
        let renewal = expiring.renewal.unwrap();
        assert_eq!(renewal.refresh.expose(), "ghr_x");
        let left = renewal
            .expires
            .timestamp()
            .duration_since(jiff::Timestamp::now());
        assert!(left.as_secs() > 28_000, "{left:?}");
        assert!(granted(&json!({"error": "authorization_pending"})).is_none());
    }

    #[test]
    fn the_install_page_is_the_app_s_own() {
        let file = |url: Option<&str>, app: Option<&str>| GithubFile {
            client_id: "Iv23li0123".into(),
            url: url.map(str::to_owned),
            app: app.map(str::to_owned),
        };
        assert_eq!(
            Github::new(&file(None, Some("glave"))).install().as_deref(),
            Some("https://github.com/apps/glave/installations/new")
        );
        assert_eq!(
            Github::new(&file(Some("https://git.example.com/"), Some("glave")))
                .install()
                .as_deref(),
            Some("https://git.example.com/github-apps/glave/installations/new")
        );
        assert_eq!(Github::new(&file(None, None)).install(), None);
    }
}

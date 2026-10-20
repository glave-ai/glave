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
use glave_event::{At, Forge, Renewal, Secret};
use glave_ports::auth::safe_to_send;
use glave_ports::computer::CredentialHost;
use glave_ports::{Clock as _, SystemClock};
use glave_session::protocol::{ApiError, DeviceCode, code};
use serde_json::Value;

use super::check::{Http, read};

/// Sign-ins waiting at once; a new one past this ends the oldest.
const FLOWS: usize = 4;
/// The longest one poll waits before asking GitHub.
const LONGEST_WAIT: Duration = Duration::from_secs(10);
/// The longest a code is taken to last, and the longest GitHub may ask glave to wait between
/// polls, in seconds: GitHub's own are 900 and 5, and a server saying more is not believed.
const LONGEST_CODE: u64 = 3600;
const LONGEST_INTERVAL: u64 = 60;
/// The shortest wait between polls, in seconds, whatever a server says.
const SHORTEST_INTERVAL: u64 = 1;
/// The most of GitHub's own words passed on, in characters.
const LONGEST_SAID: usize = 300;
/// The longest a token is taken to last, in seconds: a year.
const LONGEST_TOKEN: i64 = 366 * 24 * 3600;

/// The GitHub App sign-ins go through.
#[derive(Debug, Clone)]
pub(super) struct Github {
    client_id: String,
    /// Where GitHub is: `https://github.com`, or a GitHub Enterprise Server.
    pub url: String,
    /// The app's name in its URL, for the page where the person chooses its repositories.
    app: Option<String>,
}

/// What GitHub granted: a user token, and how to renew it when it expires.
pub(super) struct Grant {
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
pub(super) struct Flows(Mutex<HashMap<String, Flow>>);

/// How a poll went.
pub(super) enum Polled {
    Waiting,
    Granted(Grant),
}

impl Github {
    /// The app `[github]` names; none when its `url` is not a host's origin, which reading
    /// the configuration refuses first.
    pub(super) fn new(file: &GithubFile) -> Option<Self> {
        let url = file.url.as_deref().unwrap_or("https://github.com");
        Some(Self {
            client_id: file.client_id.trim().to_owned(),
            url: CredentialHost::named(url, Forge::Github)?.url,
            app: file.app.as_deref().map(str::trim).map(str::to_owned),
        })
    }

    /// The page where the person installs the app on their account or organizations and
    /// chooses its repositories, when the app's name is known.
    pub(super) fn install(&self) -> Option<String> {
        let app = self.app.as_ref()?;
        let apps = if self.url == "https://github.com" {
            "apps"
        } else {
            "github-apps"
        };
        Some(format!("{}/{apps}/{app}/installations/new", self.url))
    }

    /// Asks GitHub for a code for the person to enter, and keeps the sign-in as `flow`.
    pub(super) async fn start(
        &self,
        http: &Http,
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
        let expires_in = expires_in.min(LONGEST_CODE);
        let interval = number("interval")
            .unwrap_or(5)
            .clamp(SHORTEST_INTERVAL, LONGEST_INTERVAL);
        let now = Instant::now();
        let waiting = Flow {
            device_code: Secret::new(device_code),
            interval: Duration::from_secs(interval),
            next: now,
            expires: now
                .checked_add(Duration::from_secs(expires_in))
                .unwrap_or(now),
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
    /// person approved the app. Only waits, saying so, when that is further off than one poll
    /// waits.
    pub(super) async fn poll(
        &self,
        http: &Http,
        flows: &Flows,
        flow: &str,
    ) -> Result<Polled, ApiError> {
        let Some((device_code, slot)) = flows.waiting(flow)? else {
            tokio::time::sleep(LONGEST_WAIT).await;
            return Ok(Polled::Waiting);
        };
        tokio::time::sleep(slot.saturating_duration_since(Instant::now())).await;
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
                flows.answered(flow, None);
                return Ok(Polled::Waiting);
            }
            "slow_down" => {
                let asked = answer.get("interval").and_then(Value::as_u64);
                let secs = asked.map_or(5, |secs| secs.clamp(5, LONGEST_INTERVAL));
                flows.answered(flow, Some(Duration::from_secs(secs)));
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
    pub(super) async fn refresh(&self, http: &Http, refresh: &Secret) -> Result<Grant, ApiError> {
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
            let seconds = seconds.clamp(0, LONGEST_TOKEN);
            let expires = SystemClock
                .now()
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

    /// The device code of `flow` and the time this poll may ask GitHub, taken from it so a
    /// poll beside this one waits its own turn an interval later; none when that time is
    /// further off than one poll waits. Refused when it is not waiting, or has expired.
    fn waiting(&self, id: &str) -> Result<Option<(Secret, Instant)>, ApiError> {
        let mut flows = self.0.lock().map_err(|_| no_flow())?;
        let now = Instant::now();
        let flow = flows.get_mut(id).ok_or_else(no_flow)?;
        if flow.expires <= now {
            flows.remove(id);
            return Err(ApiError::new(
                code::HOST,
                "the code expired before it was entered: start again",
            ));
        }
        let slot = flow.next.max(now);
        if slot.saturating_duration_since(now) > LONGEST_WAIT {
            return Ok(None);
        }
        flow.next = slot.checked_add(flow.interval).unwrap_or(slot);
        Ok(Some((flow.device_code.clone(), slot)))
    }

    /// Polls `flow` no sooner than its interval after now, when GitHub answered a poll, so
    /// delays on the way never bring polls closer than GitHub allows; `slower` apart from now
    /// on when GitHub asked for that.
    fn answered(&self, id: &str, slower: Option<Duration>) {
        if let Ok(mut flows) = self.0.lock()
            && let Some(flow) = flows.get_mut(id)
        {
            flow.interval = flow.interval.max(slower.unwrap_or_default());
            let now = Instant::now();
            let after = now.checked_add(flow.interval).unwrap_or(now);
            flow.next = flow.next.max(after);
        }
    }

    fn end(&self, id: &str) {
        if let Ok(mut flows) = self.0.lock() {
            flows.remove(id);
        }
    }
}

/// Posts `form` to GitHub at `url`, answered as JSON.
async fn post(http: &Http, url: &str, form: String) -> Result<Value, ApiError> {
    let sent = http
        .to(url)
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
    // Plain words only, and not too many: a server's text goes on to clients and logs.
    let said = answer
        .get("error_description")
        .or_else(|| answer.get("error"))
        .and_then(Value::as_str)
        .map(|said| -> String {
            said.chars()
                .filter(|c| !c.is_control())
                .take(LONGEST_SAID)
                .collect()
        });
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
    use glave_ports::testing::{FAKE_SECRET, fake_secret};
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
        let lasting = granted(&json!({"access_token": FAKE_SECRET, "token_type": "bearer"}));
        assert!(lasting.unwrap().renewal.is_none());
        let refresh = fake_secret("ghr_");
        let expiring = granted(&json!({
            "access_token": FAKE_SECRET, "expires_in": 28800,
            "refresh_token": refresh, "refresh_token_expires_in": 15_897_600
        }))
        .unwrap();
        let renewal = expiring.renewal.unwrap();
        assert_eq!(renewal.refresh.expose(), refresh);
        let left = renewal
            .expires
            .timestamp()
            .duration_since(SystemClock.now());
        assert!(left.as_secs() > 28_000, "{left:?}");
        assert!(granted(&json!({"error": "authorization_pending"})).is_none());
        for absurd in [-1, i64::MAX] {
            let grant = granted(&json!({
                "access_token": FAKE_SECRET, "expires_in": absurd, "refresh_token": refresh
            }));
            assert!(grant.and_then(|grant| grant.renewal).is_some(), "{absurd}");
        }
    }

    #[test]
    fn polls_beside_each_other_take_turns_an_interval_apart() {
        let flows = Flows::default();
        let now = Instant::now();
        let flow = |interval: u64| Flow {
            device_code: Secret::new("dc"),
            interval: Duration::from_secs(interval),
            next: now,
            expires: now + Duration::from_mins(15),
        };
        flows.keep("a".into(), flow(4));
        let first = flows.waiting("a").unwrap().unwrap().1;
        let second = flows.waiting("a").unwrap().unwrap().1;
        assert_eq!(second.duration_since(first), Duration::from_secs(4));
        let third = flows.waiting("a").unwrap().unwrap().1;
        assert_eq!(third.duration_since(first), Duration::from_secs(8));
        assert!(flows.waiting("a").unwrap().is_none(), "12 s off: only wait");
        flows.answered("a", Some(Duration::from_secs(30)));
        assert!(flows.waiting("a").unwrap().is_none());
        assert!(flows.waiting("b").is_err());
    }

    #[test]
    fn the_install_page_is_the_app_s_own() {
        let file = |url: Option<&str>, app: Option<&str>| GithubFile {
            client_id: "Iv23li0123".into(),
            url: url.map(str::to_owned),
            app: app.map(str::to_owned),
        };
        let install = |url, app| Github::new(&file(url, app)).unwrap().install();
        assert_eq!(
            install(None, Some("glave")).as_deref(),
            Some("https://github.com/apps/glave/installations/new")
        );
        assert_eq!(
            install(Some("https://Git.Example.com/"), Some("glave")).as_deref(),
            Some("https://git.example.com/github-apps/glave/installations/new")
        );
        assert_eq!(install(None, None), None);
        assert!(Github::new(&file(Some("https://git.example.com/api"), None)).is_none());
    }
}

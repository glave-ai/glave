//! What a throwaway glave's organization starts with, so its Admin shows at once with people
//! in it: two people who joined by invites and gave their names, an Admin and a Member, and an
//! invite still waiting. All through the session API over HTTP, as the people themselves would: the owner's
//! requests from this computer with the server's local token, at its address; each invitee's
//! at `localhost`, the door people who share this computer come in by, as a browser making its
//! first passkey there, played by glave-identity's authenticator in software. Nothing in glave
//! knows they are samples.

use std::path::Path;
use std::time::Duration;

use glave_identity::testing::Authenticator;
use serde_json::{Value, json};

/// The roles and names of the people it starts with, each joined by an invite of its own.
const JOINED: [(&str, &str); 2] = [("admin", "Dana Lee"), ("member", "Sam Ortiz")];

/// The role of the invite left waiting.
const WAITING: &str = "member";

/// How long one request may take: the server is on this computer, and answers at once.
const ANSWER_WITHIN: Duration = Duration::from_secs(30);

/// Gives the organization of the glave whose folder is `home`, serving now, its people and
/// its waiting invite.
pub(crate) async fn seed(home: &Path) -> Result<(), String> {
    let read = |name: &str| {
        std::fs::read_to_string(home.join(name))
            .map(|text| text.trim().to_owned())
            .map_err(|err| format!("could not read glave's {name}: {err}"))
    };
    let address = read("serve.url")?;
    let port = address
        .rsplit_once(':')
        .map(|(_, port)| port)
        .ok_or_else(|| format!("glave serves at {address}, which names no port"))?;
    let http = reqwest::Client::builder()
        .timeout(ANSWER_WITHIN)
        .build()
        .map_err(|err| format!("could not make an HTTP client: {err}"))?;
    let owner = Api {
        http: &http,
        url: address.clone(),
        token: Some(read("serve.token")?),
    };
    let people = format!("http://localhost:{port}");
    let browser = Api {
        http: &http,
        url: people.clone(),
        token: None,
    };
    for (role, name) in JOINED {
        let invited = owner.call("invite/create", json!({ "role": role })).await?;
        let code = invited
            .get("code")
            .cloned()
            .ok_or("glave answered invite/create with no code")?;
        join(&browser, &people, code, name).await?;
    }
    owner
        .call("invite/create", json!({ "role": WAITING }))
        .await
        .map(drop)
}

/// Joins with the invite `code` in a new browser at `origin`, making its first passkey there:
/// a new glave account for the person named `name`, in the organization with the invite's role.
async fn join(browser: &Api<'_>, origin: &str, code: Value, name: &str) -> Result<(), String> {
    let mut device = Authenticator::new(origin)
        .map_err(|_| "could not make a passkey in software".to_owned())?;
    let asked = json!({ "code": code, "link": "invite" });
    let options = browser.call("passkey/create_options", asked).await?;
    let credential = options
        .get("options")
        .ok_or("glave answered passkey/create_options with no options")
        .and_then(|options| {
            device
                .create(options)
                .map_err(|_| "could not answer glave's request for a passkey")
        })?;
    let made = json!({
        "ceremony": options.get("ceremony"),
        "credential": credential,
        "person_name": name,
    });
    browser.call("passkey/create", made).await.map(drop)
}

/// The session API at `url`, as the owner with their local token or as a browser not signed in.
struct Api<'a> {
    http: &'a reqwest::Client,
    url: String,
    token: Option<String>,
}

impl Api<'_> {
    /// `method` called with `params`: its result, or what glave said was wrong. Codes and
    /// tokens stay out of what it says.
    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let message = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        let mut post = self
            .http
            .post(format!("{}/", self.url))
            .header("content-type", "application/json")
            .body(message.to_string());
        if let Some(token) = &self.token {
            post = post.bearer_auth(token);
        }
        let failed = |err: reqwest::Error| format!("could not call glave's {method}: {err}");
        let body = post
            .send()
            .await
            .map_err(failed)?
            .bytes()
            .await
            .map_err(failed)?;
        let mut answer: Value = serde_json::from_slice(&body)
            .map_err(|_| format!("glave answered {method} with something other than JSON"))?;
        if let Some(result) = answer.get_mut("result") {
            return Ok(result.take());
        }
        let said = answer
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("nothing it could say");
        Err(format!("glave refused {method}: {said}"))
    }
}

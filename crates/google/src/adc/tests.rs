use std::collections::HashMap;

use aws_lc_rs::signature::{
    KeyPair as _, RSA_PKCS1_2048_8192_SHA256, RsaKeyPair, UnparsedPublicKey,
};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use glave_ports::auth::Auth;
use glave_ports::block_on;
use glave_ports::testing::{FakeHttp, ManualClock, Received};

use super::*;

mod gemini;

const REQUEST: Outgoing<'static> = Outgoing {
    method: "POST",
    url: "https://aiplatform.googleapis.com/v1/projects/p/locations/global/endpoints/openapi/chat/completions",
    headers: &[],
    body: b"{}",
};

/// The RSA key of RFC 7515, Appendix A.2: a published test key.
const KEY: &str = include_str!("../jwt/rfc7515-a2.pem");

fn token(answer: &str, expires_in: u32) -> (u16, String) {
    (
        200,
        format!(r#"{{"access_token":"{answer}","expires_in":{expires_in},"token_type":"Bearer"}}"#),
    )
}

/// A request's body as a form.
fn form(received: &Received) -> HashMap<String, String> {
    url::form_urlencoded::parse(received.body.as_bytes())
        .into_owned()
        .collect()
}

/// A folder of credential files, and a clock.
struct Pod {
    folder: PathBuf,
    clock: ManualClock,
}

impl Pod {
    fn new(name: &str) -> Self {
        let folder =
            std::env::temp_dir().join(format!("glave-google-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        Self {
            folder,
            clock: ManualClock::at_second(1_000_000),
        }
    }

    fn file(&self, name: &str, text: &str) -> String {
        let path = self.folder.join(name);
        std::fs::write(&path, text).unwrap();
        path.display().to_string()
    }

    fn adc(&self, vars: &[(&str, &str)]) -> Auth {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        let adc = Adc::from_env(
            |name| vars.get(name).cloned(),
            SCOPE,
            Arc::new(self.clock.clone()),
        );
        Auth::Changing(Arc::new(adc.unwrap()))
    }
}

impl Drop for Pod {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

fn bearer(auth: &Auth) -> Result<String, AuthError> {
    let headers = block_on!(auth.headers(&REQUEST))?;
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].name, "authorization");
    Ok(headers[0].value.expose().to_owned())
}

#[test]
fn on_gke_the_metadata_server_s_token_is_kept_until_near_expiry() {
    let google =
        FakeHttp::start(vec![token("ya29.meta-1", 3599), token("ya29.meta-2", 3599)]).unwrap();
    let pod = Pod::new("metadata");
    let home = pod.folder.display().to_string();
    let auth = pod.adc(&[("GCE_METADATA_HOST", &google.host), ("HOME", &home)]);
    assert_eq!(bearer(&auth).unwrap(), "Bearer ya29.meta-1");
    pod.clock.advance_seconds(3_000);
    assert_eq!(bearer(&auth).unwrap(), "Bearer ya29.meta-1", "kept");
    pod.clock.advance_seconds(400);
    assert_eq!(bearer(&auth).unwrap(), "Bearer ya29.meta-2", "replaced");
    let received = google.received();
    assert_eq!(received.len(), 2);
    assert_eq!(received[0].method, "GET");
    assert_eq!(
        received[0].path,
        "/computeMetadata/v1/instance/service-accounts/default/token"
    );
    assert_eq!(received[0].headers["metadata-flavor"], "Google");
}

#[test]
fn a_service_account_key_signs_an_assertion_google_can_verify() {
    let google = FakeHttp::start(vec![token("ya29.sa", 3600)]).unwrap();
    let pod = Pod::new("service-account");
    let file = serde_json::json!({
        "type": "service_account",
        "project_id": "acme-ai",
        "private_key_id": "key-1",
        "private_key": KEY,
        "client_email": "glave@acme-ai.iam.gserviceaccount.com",
        "token_uri": format!("{}/token", google.url),
    });
    let path = pod.file("sa.json", &file.to_string());
    let auth = pod.adc(&[("GOOGLE_APPLICATION_CREDENTIALS", &path)]);
    assert_eq!(bearer(&auth).unwrap(), "Bearer ya29.sa");
    let token_uri = format!("{}/token", google.url);
    let received = google.received();
    assert_eq!(received[0].path, "/token");
    let form = form(&received[0]);
    assert_eq!(
        form["grant_type"],
        "urn:ietf:params:oauth:grant-type:jwt-bearer"
    );
    let parts: Vec<&str> = form["assertion"].split('.').collect();
    let json = |part: &str| -> serde_json::Value {
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
    };
    assert_eq!(
        json(parts[0]),
        serde_json::json!({"alg": "RS256", "typ": "JWT", "kid": "key-1"})
    );
    assert_eq!(
        json(parts[1]),
        serde_json::json!({
            "iss": "glave@acme-ai.iam.gserviceaccount.com",
            "scope": SCOPE,
            "aud": token_uri,
            "iat": 1_000_000,
            "exp": 1_003_600,
        })
    );
    // Verified as Google would, with the key's public half.
    let body: String = KEY
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    let key = RsaKeyPair::from_pkcs8(&STANDARD.decode(body).unwrap()).unwrap();
    let public = UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, key.public_key().as_ref());
    let signed = format!("{}.{}", parts[0], parts[1]);
    public
        .verify(
            signed.as_bytes(),
            &URL_SAFE_NO_PAD.decode(parts[2]).unwrap(),
        )
        .unwrap();
}

#[test]
fn gcloud_s_user_credentials_are_found_and_refreshed() {
    let google = FakeHttp::start(vec![token("ya29.user", 3599)]).unwrap();
    let pod = Pod::new("gcloud");
    let file = serde_json::json!({
        "type": "authorized_user",
        "client_id": "client-1.apps.googleusercontent.com",
        "client_secret": "d-secret",
        "refresh_token": "1//refresh",
        "token_uri": format!("{}/token", google.url),
    });
    pod.file("application_default_credentials.json", &file.to_string());
    let config = pod.folder.display().to_string();
    let auth = pod.adc(&[
        ("CLOUDSDK_CONFIG", &config),
        ("GCE_METADATA_HOST", "127.0.0.1:9"),
    ]);
    assert_eq!(bearer(&auth).unwrap(), "Bearer ya29.user");
    let form = form(&google.received()[0]);
    assert_eq!(form["grant_type"], "refresh_token");
    assert_eq!(form["refresh_token"], "1//refresh");
    assert_eq!(form["client_id"], "client-1.apps.googleusercontent.com");
}

#[test]
fn an_external_account_exchanges_its_token_then_impersonates() {
    let google = FakeHttp::start(vec![
        token("ya29.federated", 3600),
        (
            200,
            r#"{"accessToken":"ya29.impersonated","expireTime":"1970-01-12T14:46:40Z"}"#.into(),
        ),
    ])
    .unwrap();
    let pod = Pod::new("external");
    let subject = pod.file("subject", "cluster-jwt-1\n");
    let file = serde_json::json!({
        "type": "external_account",
        "audience": "//iam.googleapis.com/projects/1/locations/global/workloadIdentityPools/p/providers/eks",
        "subject_token_type": "urn:ietf:params:oauth:token-type:jwt",
        "token_url": format!("{}/v1/token", google.url),
        "service_account_impersonation_url": format!("{}/v1/projects/-/serviceAccounts/glave@acme-ai.iam.gserviceaccount.com:generateAccessToken", google.url),
        "service_account_impersonation": {"token_lifetime_seconds": 1800},
        "credential_source": {"file": subject, "format": {"type": "text"}},
    });
    let path = pod.file("wif.json", &file.to_string());
    let auth = pod.adc(&[("GOOGLE_APPLICATION_CREDENTIALS", &path)]);
    assert_eq!(bearer(&auth).unwrap(), "Bearer ya29.impersonated");
    let received = google.received();
    let sts = form(&received[0]);
    assert_eq!(received[0].path, "/v1/token");
    assert_eq!(sts["grant_type"], GRANT);
    assert_eq!(sts["subject_token"], "cluster-jwt-1");
    assert_eq!(
        sts["subject_token_type"],
        "urn:ietf:params:oauth:token-type:jwt"
    );
    assert_eq!(
        sts["requested_token_type"],
        "urn:ietf:params:oauth:token-type:access_token"
    );
    assert_eq!(sts["scope"], SCOPE);
    assert!(sts["audience"].ends_with("/providers/eks"));
    assert!(received[1].path.ends_with(":generateAccessToken"));
    assert_eq!(
        received[1].headers["authorization"],
        "Bearer ya29.federated"
    );
    let asked: serde_json::Value = serde_json::from_str(&received[1].body).unwrap();
    assert_eq!(
        asked,
        serde_json::json!({"scope": [SCOPE], "lifetime": "1800s"})
    );
}

const GRANT: &str = "urn:ietf:params:oauth:grant-type:token-exchange";

#[test]
fn an_external_account_reads_a_json_token_from_a_url() {
    let google = FakeHttp::start(vec![
        (200, r#"{"access_token":"idp-token-1"}"#.into()),
        token("ya29.federated", 3600),
    ])
    .unwrap();
    let pod = Pod::new("external-url");
    let file = serde_json::json!({
        "type": "external_account",
        "audience": "//iam.googleapis.com/projects/1/locations/global/workloadIdentityPools/p/providers/aks",
        "subject_token_type": "urn:ietf:params:oauth:token-type:jwt",
        "token_url": format!("{}/v1/token", google.url),
        "credential_source": {
            "url": format!("{}/identity", google.url),
            "headers": {"Metadata": "True"},
            "format": {"type": "json", "subject_token_field_name": "access_token"},
        },
    });
    let path = pod.file("wif.json", &file.to_string());
    let auth = pod.adc(&[("GOOGLE_APPLICATION_CREDENTIALS", &path)]);
    assert_eq!(bearer(&auth).unwrap(), "Bearer ya29.federated");
    let received = google.received();
    assert_eq!(received[0].path, "/identity");
    assert_eq!(received[0].headers["metadata"], "True");
    assert_eq!(form(&received[1])["subject_token"], "idp-token-1");
}

#[test]
fn a_refused_exchange_says_why_without_any_secret() {
    let google = FakeHttp::start(vec![
        (
            400,
            r#"{"error":"invalid_grant","error_description":"Invalid JWT Signature."}"#.into(),
        ),
        (503, "{}".into()),
    ])
    .unwrap();
    let pod = Pod::new("refused");
    let file = serde_json::json!({
        "type": "service_account",
        "private_key": KEY,
        "client_email": "glave@acme-ai.iam.gserviceaccount.com",
        "token_uri": format!("{}/token", google.url),
    });
    let path = pod.file("sa.json", &file.to_string());
    let auth = pod.adc(&[("GOOGLE_APPLICATION_CREDENTIALS", &path)]);
    let refused = bearer(&auth).unwrap_err();
    assert!(!refused.is_transient());
    let message = refused.to_string();
    assert!(
        message.contains("invalid_grant: Invalid JWT Signature."),
        "{message}"
    );
    assert!(message.contains("glave@acme-ai"), "{message}");
    assert!(
        !message.contains("PRIVATE KEY") && !message.contains("eyJ"),
        "{message}"
    );
    assert!(bearer(&auth).unwrap_err().is_transient(), "a 503 may pass");
    assert_eq!(google.received().len(), 2);
    let shown = format!("{auth:?}");
    assert!(
        !shown.contains("PRIVATE") && shown.contains("sa.json"),
        "{shown}"
    );
}

#[test]
fn files_glave_cannot_use_are_named_and_never_shown() {
    let pod = Pod::new("unusable");
    let missing = pod.folder.join("missing.json").display().to_string();
    let error = bearer(&pod.adc(&[("GOOGLE_APPLICATION_CREDENTIALS", &missing)])).unwrap_err();
    assert!(error.to_string().contains("missing.json"), "{error}");
    let other = pod.file(
        "other.json",
        r#"{"type":"impersonated_service_account","x":"s3cret"}"#,
    );
    let error = bearer(&pod.adc(&[("GOOGLE_APPLICATION_CREDENTIALS", &other)])).unwrap_err();
    assert!(
        error.to_string().contains("impersonated_service_account"),
        "{error}"
    );
    assert!(!error.to_string().contains("s3cret"), "{error}");
    let broken = pod.file(
        "broken.json",
        r#"{"type":"service_account","private_key":"s3cret"#,
    );
    let error = bearer(&pod.adc(&[("GOOGLE_APPLICATION_CREDENTIALS", &broken)])).unwrap_err();
    assert!(!error.to_string().contains("s3cret"), "{error}");
    let aws = pod.file(
        "aws.json",
        r#"{"type":"external_account","audience":"a","subject_token_type":"t","token_url":"https://sts.googleapis.com/v1/token","credential_source":{"environment_id":"aws1"}}"#,
    );
    let error = bearer(&pod.adc(&[("GOOGLE_APPLICATION_CREDENTIALS", &aws)])).unwrap_err();
    assert!(error.to_string().contains("AWS or executable"), "{error}");
}

#[test]
fn off_google_cloud_the_metadata_server_s_absence_says_what_to_set() {
    let pod = Pod::new("off-cloud");
    let home = pod.folder.display().to_string();
    // Nothing listens on port 9 of this computer.
    let auth = pod.adc(&[("GCE_METADATA_HOST", "127.0.0.1:9"), ("HOME", &home)]);
    let error = bearer(&auth).unwrap_err();
    assert!(error.is_transient());
    assert!(
        error.to_string().contains("GOOGLE_APPLICATION_CREDENTIALS"),
        "{error}"
    );
    let clock: Arc<dyn Clock> = Arc::new(ManualClock::at_second(0));
    let wrong = Adc::from_env(
        |name| (name == "GCE_METADATA_HOST").then(|| "evil.com/x?".to_owned()),
        SCOPE,
        clock,
    );
    assert!(
        wrong
            .err()
            .unwrap()
            .to_string()
            .contains("GCE_METADATA_HOST")
    );
}

#[test]
fn where_credentials_come_from_is_described() {
    let pod = Pod::new("describe");
    let named = pod.adc(&[("GOOGLE_APPLICATION_CREDENTIALS", "/var/run/google/sa.json")]);
    assert_eq!(
        named.describe(),
        "Google credentials in /var/run/google/sa.json (GOOGLE_APPLICATION_CREDENTIALS)"
    );
    let found = pod.adc(&[("HOME", "/home/glave")]);
    assert!(
        found
            .describe()
            .contains("/home/glave/.config/gcloud/application_default_credentials.json if present, else the metadata server"),
        "{}",
        found.describe()
    );
}

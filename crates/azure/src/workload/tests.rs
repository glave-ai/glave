use std::collections::HashMap;
use std::sync::Arc;

use glave_ports::auth::{Auth, Outgoing};
use glave_ports::block_on;
use glave_ports::testing::{FakeHttp, ManualClock, Received};

use super::*;

/// A request's body as a form.
fn form(received: &Received) -> HashMap<String, String> {
    url::form_urlencoded::parse(received.body.as_bytes())
        .into_owned()
        .collect()
}

const TOKEN: &str = r#"{"token_type":"Bearer","expires_in":3599,"access_token":"entra-1"}"#;
const TOKEN_2: &str = r#"{"token_type":"Bearer","expires_in":"3599","access_token":"entra-2"}"#;

const REQUEST: Outgoing<'static> = Outgoing {
    method: "POST",
    url: "https://acme.openai.azure.com/openai/v1/chat/completions",
    headers: &[],
    body: b"{}",
};

struct Pod {
    folder: PathBuf,
    clock: ManualClock,
}

impl Pod {
    fn new(name: &str) -> Self {
        let folder =
            std::env::temp_dir().join(format!("glave-azure-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("token"), "cluster-jwt-1\n").unwrap();
        Self {
            folder,
            clock: ManualClock::at_second(1_000_000),
        }
    }

    fn identity(&self, authority: &str) -> WorkloadIdentity {
        let file = self.folder.join("token").display().to_string();
        let env = |name: &str| match name {
            "AZURE_FEDERATED_TOKEN_FILE" => Some(file.clone()),
            "AZURE_CLIENT_ID" => Some("client-1".to_owned()),
            "AZURE_TENANT_ID" => Some("tenant-1".to_owned()),
            "AZURE_AUTHORITY_HOST" => Some(format!("{authority}/")),
            _ => None,
        };
        WorkloadIdentity::from_env(env, DEFAULT_SCOPE, Arc::new(self.clock.clone()))
            .unwrap()
            .unwrap()
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
fn the_cluster_token_is_exchanged_and_the_entra_token_kept_until_near_expiry() {
    let entra = FakeHttp::start(vec![(200, TOKEN.into()), (200, TOKEN_2.into())]).unwrap();
    let pod = Pod::new("exchange");
    let auth = Auth::Changing(Arc::new(pod.identity(&entra.url)));
    assert_eq!(bearer(&auth).unwrap(), "Bearer entra-1");
    pod.clock.advance_seconds(3_000);
    assert_eq!(
        bearer(&auth).unwrap(),
        "Bearer entra-1",
        "kept, not asked again"
    );
    std::fs::write(pod.folder.join("token"), "cluster-jwt-2").unwrap();
    pod.clock.advance_seconds(400);
    assert_eq!(
        bearer(&auth).unwrap(),
        "Bearer entra-2",
        "replaced near expiry"
    );
    let received = entra.received();
    assert_eq!(received.len(), 2);
    assert_eq!(received[0].path, "/tenant-1/oauth2/v2.0/token");
    let first = form(&received[0]);
    assert_eq!(first["client_id"], "client-1");
    assert_eq!(first["scope"], "https://ai.azure.com/.default");
    assert_eq!(first["grant_type"], "client_credentials");
    assert_eq!(first["client_assertion_type"], ASSERTION_TYPE);
    assert_eq!(first["client_assertion"], "cluster-jwt-1");
    assert_eq!(
        form(&received[1])["client_assertion"],
        "cluster-jwt-2",
        "the file is read again, as Kubernetes rewrites it"
    );
}

#[test]
fn a_refused_exchange_says_why_without_the_assertion() {
    let entra = FakeHttp::start(vec![
        (
            400,
            r#"{"error":"invalid_client","error_description":"AADSTS700016: Application with identifier 'client-1' was not found.\r\nTrace ID: 1"}"#
                .into(),
        ),
        (503, "{}".into()),
    ]).unwrap();
    let pod = Pod::new("refused");
    let auth = Auth::Changing(Arc::new(pod.identity(&entra.url)));
    let refused = bearer(&auth).unwrap_err();
    assert!(!refused.is_transient());
    let message = refused.to_string();
    assert!(
        message.contains("invalid_client: AADSTS700016"),
        "{message}"
    );
    assert!(!message.contains("Trace ID"), "{message}");
    assert!(!message.contains("cluster-jwt"), "{message}");
    let down = bearer(&auth).unwrap_err();
    assert!(down.is_transient(), "{down}");
    assert_eq!(entra.received().len(), 2);
}

#[test]
fn a_missing_token_file_is_named() {
    let pod = Pod::new("missing");
    let identity = pod.identity("https://login.microsoftonline.com");
    std::fs::remove_file(pod.folder.join("token")).unwrap();
    let error = bearer(&Auth::Changing(Arc::new(identity))).unwrap_err();
    assert!(
        error.to_string().contains("AZURE_FEDERATED_TOKEN_FILE"),
        "{error}"
    );
    assert!(!error.is_transient());
}

#[test]
fn the_environment_is_checked_naming_what_is_wrong() {
    let clock: Arc<dyn Clock> = Arc::new(ManualClock::at_second(0));
    let build = |vars: &[(&str, &str)]| {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        WorkloadIdentity::from_env(|name| vars.get(name).cloned(), DEFAULT_SCOPE, clock.clone())
            .map(|identity| identity.map(|identity| identity.describe()))
    };
    assert_eq!(build(&[]), Ok(None), "not a pod with workload identity");
    let file = ("AZURE_FEDERATED_TOKEN_FILE", "/var/run/token");
    let client = ("AZURE_CLIENT_ID", "client-1");
    let tenant = ("AZURE_TENANT_ID", "tenant-1");
    let missing = build(&[file, tenant]).unwrap_err().to_string();
    assert!(missing.contains("AZURE_CLIENT_ID"), "{missing}");
    assert_eq!(
        build(&[file, client, tenant]).unwrap().as_deref(),
        Some(
            "Microsoft Entra workload identity (client client-1, scope https://ai.azure.com/.default)"
        )
    );
    let plain = build(&[
        file,
        client,
        tenant,
        ("AZURE_AUTHORITY_HOST", "http://login.example.com"),
    ]);
    assert!(
        plain
            .unwrap_err()
            .to_string()
            .contains("AZURE_AUTHORITY_HOST")
    );
    let sneaky = build(&[file, client, ("AZURE_TENANT_ID", "../evil")]);
    assert!(sneaky.unwrap_err().to_string().contains("AZURE_TENANT_ID"));
}

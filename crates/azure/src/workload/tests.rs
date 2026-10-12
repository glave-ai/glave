use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use glave_ports::auth::{Auth, Outgoing};
use jiff::Timestamp;

use super::*;

/// A clock the test moves.
struct Manual(AtomicI64);

impl Clock for Manual {
    fn now(&self) -> Timestamp {
        Timestamp::from_second(self.0.load(Ordering::SeqCst)).unwrap()
    }
}

/// A request the fake Entra received: its path and its form.
struct Received {
    path: String,
    form: HashMap<String, String>,
}

/// A fake Entra on 127.0.0.1 that gives each request the next of its answers.
struct FakeEntra {
    url: String,
    received: Arc<Mutex<Vec<Received>>>,
    thread: Option<JoinHandle<()>>,
}

impl FakeEntra {
    fn start(answers: Vec<(u16, &'static str)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let received = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&received);
        let thread = std::thread::spawn(move || {
            for (status, body) in answers {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap().to_owned();
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    let header = header.trim_end();
                    if header.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut form = vec![0; length];
                reader.read_exact(&mut form).unwrap();
                let form = url::form_urlencoded::parse(&form).into_owned().collect();
                log.lock().unwrap().push(Received { path, form });
                let mut stream = stream;
                write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: \
                     {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        Self {
            url,
            received,
            thread: Some(thread),
        }
    }

    /// Every request, once all the answers have been given.
    fn received(mut self) -> Vec<Received> {
        self.thread.take().unwrap().join().unwrap();
        std::mem::take(&mut *self.received.lock().unwrap())
    }
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
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
    clock: Arc<Manual>,
}

impl Pod {
    fn new(name: &str) -> Self {
        let folder =
            std::env::temp_dir().join(format!("glave-azure-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("token"), "cluster-jwt-1\n").unwrap();
        Self {
            folder,
            clock: Arc::new(Manual(AtomicI64::new(1_000_000))),
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
        WorkloadIdentity::from_env(env, DEFAULT_SCOPE, self.clock.clone())
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
    let headers = block_on(auth.headers(&REQUEST))?;
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].name, "authorization");
    Ok(headers[0].value.expose().to_owned())
}

#[test]
fn the_cluster_token_is_exchanged_and_the_entra_token_kept_until_near_expiry() {
    let entra = FakeEntra::start(vec![(200, TOKEN), (200, TOKEN_2)]);
    let pod = Pod::new("exchange");
    let auth = Auth::Changing(Arc::new(pod.identity(&entra.url)));
    assert_eq!(bearer(&auth).unwrap(), "Bearer entra-1");
    pod.clock.0.fetch_add(3_000, Ordering::SeqCst);
    assert_eq!(
        bearer(&auth).unwrap(),
        "Bearer entra-1",
        "kept, not asked again"
    );
    std::fs::write(pod.folder.join("token"), "cluster-jwt-2").unwrap();
    pod.clock.0.fetch_add(400, Ordering::SeqCst);
    assert_eq!(
        bearer(&auth).unwrap(),
        "Bearer entra-2",
        "replaced near expiry"
    );
    let received = entra.received();
    assert_eq!(received.len(), 2);
    assert_eq!(received[0].path, "/tenant-1/oauth2/v2.0/token");
    let form = &received[0].form;
    assert_eq!(form["client_id"], "client-1");
    assert_eq!(form["scope"], "https://ai.azure.com/.default");
    assert_eq!(form["grant_type"], "client_credentials");
    assert_eq!(form["client_assertion_type"], ASSERTION_TYPE);
    assert_eq!(form["client_assertion"], "cluster-jwt-1");
    assert_eq!(
        received[1].form["client_assertion"], "cluster-jwt-2",
        "the file is read again, as Kubernetes rewrites it"
    );
}

#[test]
fn a_refused_exchange_says_why_without_the_assertion() {
    let entra = FakeEntra::start(vec![
        (
            400,
            r#"{"error":"invalid_client","error_description":"AADSTS700016: Application with identifier 'client-1' was not found.\r\nTrace ID: 1"}"#,
        ),
        (503, "{}"),
    ]);
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
    let clock: Arc<dyn Clock> = Arc::new(Manual(AtomicI64::new(0)));
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

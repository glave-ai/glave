use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};
use std::thread::JoinHandle;

use glave_ports::auth::Auth;

use super::*;

/// The keys of AWS's SigV4 test suite (`aws-signing-test-suite/v4`, as aws-sigv4 ships it).
fn suite_keys(token: Option<&str>) -> Keys {
    Keys {
        access_key_id: "AKIDEXAMPLE".into(),
        secret_access_key: Secret::new("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY"),
        session_token: token.map(Secret::new),
    }
}

/// The suite's time: 2015-08-30T12:36:00Z.
fn suite_time() -> Timestamp {
    "2015-08-30T12:36:00Z".parse().unwrap()
}

const SUITE_TOKEN: &str = "AQoDYXdzEPT//////////wEXAMPLEtc764bNrC9SAPBSM22wDOk4x4HIZ8j4FZTwdQWLWsKWHGBuFqwAeMicRXmxfpSPfIeoIYRqTflfKD8YUuwthAx7mSEI/qkPpKPi/kMcGdQrmGdeehM4IC1NtBmUpp2wUE8phUZampKsburEDy0KPkyQDYwT7WZ0wq5VSXDvp75YU9HFvlRd8Tx6q6fE8YQcHNVXAkiY9q6d+xo0rKwT38xVqr7ZD0u0iPPkUL64lIZbqBAz+scqKmlzm8FDrypNC9Yjc8fPOLn9FX9KSYvKTr4rvx3iSIlTJabIQwj2ICCR/oLxBA==";

fn suite_post(keys: &Keys) -> HashMap<String, String> {
    let request = Outgoing {
        method: "POST",
        url: "https://example.amazonaws.com/",
        headers: &[],
        body: b"",
    };
    signed(keys, "us-east-1", "service", suite_time(), &request)
        .unwrap()
        .into_iter()
        .map(|header| (header.name, header.value.expose().to_owned()))
        .collect()
}

#[test]
fn post_vanilla_signs_as_aws_s_suite_says() {
    let headers = suite_post(&suite_keys(None));
    assert_eq!(headers["x-amz-date"], "20150830T123600Z");
    assert_eq!(
        headers["authorization"],
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
         SignedHeaders=host;x-amz-date, \
         Signature=5da7c1a2acd57cee7505fc6676e4e544621c30862966e37dddb68e92efbe5d6b"
    );
    assert!(!headers.contains_key("x-amz-security-token"));
}

#[test]
fn post_sts_header_before_signs_the_session_token_as_aws_s_suite_says() {
    let headers = suite_post(&suite_keys(Some(SUITE_TOKEN)));
    assert_eq!(headers["x-amz-security-token"], SUITE_TOKEN);
    assert_eq!(
        headers["authorization"],
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
         SignedHeaders=host;x-amz-date;x-amz-security-token, \
         Signature=85d96828115b5dc0cfc3bd16ad9e210dd772bbebba041836c64533a82be05ead"
    );
}

#[test]
fn a_bedrock_request_signs_its_headers_and_body_for_bedrock_in_its_region() {
    let request = Outgoing {
        method: "POST",
        url: "https://bedrock-runtime.eu-west-1.amazonaws.com/openai/v1/chat/completions",
        headers: &[("content-type", "application/json")],
        body: br#"{"model":"openai.gpt-oss-120b-1:0"}"#,
    };
    let sign = |body: &'static [u8]| {
        let request = Outgoing { body, ..request };
        let headers = signed(
            &suite_keys(None),
            "eu-west-1",
            "bedrock",
            suite_time(),
            &request,
        )
        .unwrap();
        headers
            .into_iter()
            .find(|header| header.name == "authorization")
            .unwrap()
            .value
            .expose()
            .to_owned()
    };
    let authorization = sign(request.body);
    assert!(
        authorization.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/eu-west-1/bedrock/aws4_request, \
             SignedHeaders=content-type;host;x-amz-date, Signature="
        ),
        "{authorization}"
    );
    assert_ne!(authorization, sign(b"{}"), "the body is signed");
}

/// A clock the test moves.
struct Manual(AtomicI64);

impl Clock for Manual {
    fn now(&self) -> Timestamp {
        Timestamp::from_second(self.0.load(Ordering::SeqCst)).unwrap()
    }
}

/// A request a fake AWS endpoint received.
struct Received {
    line: String,
    headers: HashMap<String, String>,
    body: String,
}

/// A fake AWS endpoint on 127.0.0.1 that gives each request the next of its answers.
struct Fake {
    url: String,
    received: Arc<Mutex<Vec<Received>>>,
    thread: Option<JoinHandle<()>>,
}

impl Fake {
    fn start(answers: Vec<(u16, String)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let received = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&received);
        let thread = std::thread::spawn(move || {
            for (status, answer) in answers {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut headers = HashMap::new();
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    let header = header.trim_end();
                    if header.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':') {
                        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
                    }
                }
                let length = headers
                    .get("content-length")
                    .map_or(0, |length| length.parse().unwrap());
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                log.lock().unwrap().push(Received {
                    line: line.trim_end().to_owned(),
                    headers,
                    body: String::from_utf8(body).unwrap(),
                });
                let mut stream = stream;
                write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                    answer.len()
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

fn sts_answer(key: &str, expires: &str) -> String {
    format!(
        "<AssumeRoleWithWebIdentityResponse xmlns=\"https://sts.amazonaws.com/doc/2011-06-15/\">\
         <AssumeRoleWithWebIdentityResult><Credentials><AccessKeyId>{key}</AccessKeyId>\
         <SecretAccessKey>secret-{key}</SecretAccessKey><SessionToken>token-{key}</SessionToken>\
         <Expiration>{expires}</Expiration></Credentials></AssumeRoleWithWebIdentityResult>\
         </AssumeRoleWithWebIdentityResponse>"
    )
}

const BEDROCK: Outgoing<'static> = Outgoing {
    method: "POST",
    url: "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1/chat/completions",
    headers: &[("content-type", "application/json")],
    body: b"{}",
};

fn header(auth: &Auth, name: &str) -> Result<String, AuthError> {
    let headers = block_on(auth.headers(&BEDROCK))?;
    Ok(headers
        .into_iter()
        .find(|header| header.name == name)
        .map(|header| header.value.expose().to_owned())
        .unwrap_or_default())
}

/// A pod's folder, for the token files Kubernetes would project.
struct Pod(std::path::PathBuf);

impl Pod {
    fn new(name: &str) -> Self {
        let folder = std::env::temp_dir().join(format!("glave-aws-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        Self(folder)
    }

    fn file(&self, name: &str, text: &str) -> String {
        let path = self.0.join(name);
        std::fs::write(&path, text).unwrap();
        path.display().to_string()
    }
}

impl Drop for Pod {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn signer(vars: &[(&str, String)], clock: Arc<Manual>) -> Auth {
    let vars: HashMap<String, String> = vars
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect();
    let region = Region::new("us-east-1").unwrap();
    let source = Source::from_env(&|name| vars.get(name).cloned(), &region)
        .unwrap()
        .unwrap();
    Auth::Changing(Arc::new(
        SigV4::new(source, region, Endpoint::Runtime, clock).unwrap(),
    ))
}

#[test]
fn irsa_assumes_the_role_with_the_cluster_token_and_keeps_the_keys_until_near_expiry() {
    let sts = Fake::start(vec![
        (200, sts_answer("ASIA1", "2026-10-12T13:00:00Z")),
        (200, sts_answer("ASIA2", "2026-10-12T14:00:00Z")),
    ]);
    let pod = Pod::new("irsa");
    let clock = Arc::new(Manual(AtomicI64::new(
        "2026-10-12T12:00:00Z"
            .parse::<Timestamp>()
            .unwrap()
            .as_second(),
    )));
    let auth = signer(
        &[
            (
                "AWS_WEB_IDENTITY_TOKEN_FILE",
                pod.file("token", "cluster-jwt-1\n"),
            ),
            (
                "AWS_ROLE_ARN",
                "arn:aws:iam::123456789012:role/glave".into(),
            ),
            ("AWS_ENDPOINT_URL_STS", format!("{}/", sts.url)),
        ],
        clock.clone(),
    );
    assert_eq!(
        header(&auth, "x-amz-security-token").unwrap(),
        "token-ASIA1"
    );
    assert!(
        header(&auth, "authorization")
            .unwrap()
            .contains("Credential=ASIA1/20261012/us-east-1/bedrock/aws4_request")
    );
    clock.0.fetch_add(3_000, Ordering::SeqCst);
    assert_eq!(
        header(&auth, "x-amz-security-token").unwrap(),
        "token-ASIA1",
        "ten minutes left: kept"
    );
    clock.0.fetch_add(400, Ordering::SeqCst);
    pod.file("token", "cluster-jwt-2");
    assert_eq!(
        header(&auth, "x-amz-security-token").unwrap(),
        "token-ASIA2"
    );
    let received = sts.received();
    assert_eq!(received.len(), 2, "the keys were kept in between");
    assert_eq!(received[0].line, "POST / HTTP/1.1");
    let form: HashMap<String, String> = url::form_urlencoded::parse(received[0].body.as_bytes())
        .into_owned()
        .collect();
    assert_eq!(form["Action"], "AssumeRoleWithWebIdentity");
    assert_eq!(form["Version"], "2011-06-15");
    assert_eq!(form["RoleArn"], "arn:aws:iam::123456789012:role/glave");
    assert_eq!(form["RoleSessionName"], "glave");
    assert_eq!(form["WebIdentityToken"], "cluster-jwt-1");
    assert!(received[1].body.contains("WebIdentityToken=cluster-jwt-2"));
}

#[test]
fn a_refused_assume_role_says_why_without_the_token() {
    let sts = Fake::start(vec![(
        400,
        "<ErrorResponse><Error><Code>AccessDenied</Code><Message>Not authorized to perform \
         sts:AssumeRoleWithWebIdentity</Message></Error></ErrorResponse>"
            .to_owned(),
    )]);
    let pod = Pod::new("refused");
    let auth = signer(
        &[
            (
                "AWS_WEB_IDENTITY_TOKEN_FILE",
                pod.file("token", "cluster-jwt"),
            ),
            (
                "AWS_ROLE_ARN",
                "arn:aws:iam::123456789012:role/glave".into(),
            ),
            ("AWS_ENDPOINT_URL_STS", sts.url.clone()),
        ],
        Arc::new(Manual(AtomicI64::new(0))),
    );
    let error = header(&auth, "authorization").unwrap_err();
    assert!(!error.is_transient());
    let message = error.to_string();
    assert!(
        message.contains("AccessDenied: Not authorized"),
        "{message}"
    );
    assert!(!message.contains("cluster-jwt"), "{message}");
    assert_eq!(sts.received().len(), 1);
}

#[test]
fn pod_identity_fetches_keys_with_the_pod_s_token() {
    let agent = Fake::start(vec![(
        200,
        r#"{"AccessKeyId":"ASIAPOD","SecretAccessKey":"secret","Token":"pod-session","AccountId":"123456789012","Expiration":"2026-10-12T18:00:00Z"}"#
            .to_owned(),
    )]);
    let pod = Pod::new("pod-identity");
    let auth = signer(
        &[
            (
                "AWS_CONTAINER_CREDENTIALS_FULL_URI",
                format!("{}/v1/credentials", agent.url),
            ),
            (
                "AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE",
                pod.file("eks-pod-identity-token", "pod-jwt\n"),
            ),
        ],
        Arc::new(Manual(AtomicI64::new(0))),
    );
    assert_eq!(
        header(&auth, "x-amz-security-token").unwrap(),
        "pod-session"
    );
    let received = agent.received();
    assert_eq!(received[0].line, "GET /v1/credentials HTTP/1.1");
    assert_eq!(received[0].headers["authorization"], "pod-jwt");
}

#[test]
fn keys_in_the_environment_sign_without_asking_anyone() {
    let auth = signer(
        &[
            ("AWS_ACCESS_KEY_ID", "AKIDEXAMPLE".into()),
            ("AWS_SECRET_ACCESS_KEY", "wJalrXUtnFEMI".into()),
        ],
        Arc::new(Manual(AtomicI64::new(0))),
    );
    assert!(
        header(&auth, "authorization")
            .unwrap()
            .starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/19700101/us-east-1/bedrock/")
    );
    let described = format!("{auth:?}");
    assert!(
        described.contains("AWS SigV4 for bedrock in us-east-1"),
        "{described}"
    );
    assert!(!described.contains("wJalr"), "{described}");
}

//! The server's own words in the caller's language (`specs/session.md`, "Languages"): the one
//! they chose (`language/set`, kept in their settings), else the first their client asks for,
//! else English. In process, where glave's own programs and the model read them, they stay
//! English.

use glave_event::ThreadId;
use glave_session::Answering;
use glave_session::protocol::{Reply, Request, code};
use glave_session::rpc;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader, Lines,
};

use crate::support::issuer::{Issuer, Signer};
use crate::support::web::{Web, serve_http};
use crate::support::*;

/// A thread nobody has.
fn missing() -> ThreadId {
    ThreadId::new(jiff::Timestamp::UNIX_EPOCH, [7; 10]).unwrap()
}

/// What the server says of [`missing`] in English.
fn english() -> String {
    format!("no thread {} in this project", missing())
}

/// The error message of a JSON-RPC answer.
fn said(answer: &Value) -> String {
    answer["error"]["message"].as_str().unwrap().to_owned()
}

/// Sends `request` with `id` as one line, and reads the line that answers it.
async fn ask(
    write: &mut (impl AsyncWrite + Unpin),
    lines: &mut Lines<BufReader<impl AsyncRead + Unpin>>,
    id: u64,
    request: &Request,
) -> Value {
    let line = format!("{}\n", rpc::message(id, request));
    write.write_all(line.as_bytes()).await.unwrap();
    serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap()
}

/// Whether `message` is the server's sentence about [`missing`] in `en-XA`: accented, its
/// values as they are, longer and bracketed.
fn accented(message: &str) -> bool {
    message.starts_with(&format!("[ñö ţĥŕéåð {} îñ ţĥîš þŕöĵéçţ one", missing()))
        && message.ends_with(']')
}

#[test]
fn over_http_the_clients_language_counts_until_the_person_chooses_one() {
    block_on!(async {
        let issuer = Issuer::start().await;
        let signer = Signer::es256("key-1");
        issuer.publish(&signer);
        let dir = folder("languages-http");
        let served = serve_http(server(&dir, Vec::new(), Answering::Clients).await, &issuer).await;
        let token = signer.sign(&issuer.claims("ada"));
        let get = Request::ThreadGet { thread: missing() };
        let mut plain = Web::new(&served, Some(token.clone()));
        let mut reading = Web::new(&served, Some(token.clone())).reading("de-DE, en-XA;q=0.9");
        assert_eq!(said(&plain.call(&get).await), english());
        assert!(accented(&said(&reading.call(&get).await)));

        // Chosen, the person's language wins over any client's.
        let chosen = plain
            .call(&Request::LanguageSet {
                language: Some("ar-XB".into()),
            })
            .await;
        assert_eq!(chosen["result"], json!({"language": "ar-XB"}));
        let got = plain.call(&Request::LanguageGet {}).await;
        assert_eq!(got["result"], json!({"language": "ar-XB"}));
        for web in [&mut plain, &mut reading] {
            let message = said(&web.call(&get).await);
            assert!(message.contains("\u{202e}thread\u{202c}"), "{message}");
        }

        // A language glave does not write in is refused, in the person's own.
        let refused = plain
            .call(&Request::LanguageSet {
                language: Some("tlh".into()),
            })
            .await;
        assert_eq!(refused["error"]["code"], code::INVALID_PARAMS);
        assert!(said(&refused).contains("\u{202e}write\u{202c}"));

        // Cleared, the client's counts again.
        plain.call(&Request::LanguageSet { language: None }).await;
        assert_eq!(said(&plain.call(&get).await), english());
        assert!(accented(&said(&reading.call(&get).await)));
        served.close().await;
    });
}

#[test]
fn a_program_on_this_computer_reads_the_persons_language_and_in_process_stays_english() {
    block_on!(async {
        let dir = folder("languages-local");
        let server = server(&dir, Vec::new(), Answering::Clients).await;
        let get = Request::ThreadGet { thread: missing() };
        let (client, connection) = tokio::io::duplex(1 << 16);
        let (read, write) = tokio::io::split(connection);
        let serving = tokio::spawn(rpc::serve_connection(
            server.clone(),
            BufReader::new(read),
            write,
        ));
        let (client_read, mut out) = tokio::io::split(client);
        let mut lines = BufReader::new(client_read).lines();
        assert_eq!(said(&ask(&mut out, &mut lines, 1, &get).await), english());
        let chosen = Request::LanguageSet {
            language: Some("en-XA".into()),
        };
        ask(&mut out, &mut lines, 2, &chosen).await;
        assert!(accented(&said(&ask(&mut out, &mut lines, 3, &get).await)));
        // In process, the same person reads English: what glave's own programs keep and the
        // model reads is written once.
        let Err(refused) = server.request(get.clone()).await else {
            panic!("a thread nobody has")
        };
        assert_eq!(refused.message, english().as_str());
        let Ok(Reply::Language { language }) = server.request(Request::LanguageGet {}).await else {
            panic!("the person's language")
        };
        assert_eq!(language.as_deref(), Some("en-XA"));
        serving.abort();
    });
}

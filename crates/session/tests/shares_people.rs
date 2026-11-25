//! Sharing with specific people (`specs/session.md`, "Sharing"): the page asks for an email and
//! emails a one-time link only to one the share lists, answering alike either way; opening the
//! link, once, gives the browser a proof for that share's name alone, which any change to who
//! may open the share ends for good, and asking for another link leaves the first working.

use std::net::SocketAddr;
use std::sync::Arc;

use glave_event::Audience;
use glave_identity::signin::{LocalToken, SignIn};
use glave_ports::Mailer;
use glave_ports::testing::Outbox;
use glave_session::protocol::{
    ApiError, Reply, Request, ShareChoice, ShareInfo, ShareOf, ShareSetting, code,
};

use crate::support::issuer::fixed_clock;
use crate::support::web::serve_with;
use crate::support::*;

fn people(addresses: &[&str]) -> Audience {
    Audience::People {
        people: addresses.iter().map(|each| (*each).to_owned()).collect(),
    }
}

/// A server sharing at the tests' domain and sending email to `outbox`, and the fox its model
/// drew, as sharing names it.
async fn drawn_fox(
    dir: &std::path::Path,
    outbox: &Arc<Outbox>,
) -> (Tested, ShareOf, glave_ports::testing::ManualClock) {
    let mail = Some(Arc::clone(outbox) as Arc<dyn Mailer>);
    let (server, item, clock) = crate::shares::fox_sending(dir, mail).await;
    let of = ShareOf::Item {
        project: item.project,
        asset: item.asset,
    };
    (server, of, clock)
}

/// `share/set` of `of` for `audience`.
async fn shared(server: &Tested, of: &ShareOf, audience: Audience) -> Result<ShareInfo, ApiError> {
    let set = ShareSetting {
        of: of.clone(),
        audience: Some(audience),
        update: false,
    };
    match server.request(Request::ShareSet { set }).await? {
        Reply::Share { share } => Ok(share),
        other => panic!("a share, not {other:?}"),
    }
}

#[test]
fn a_share_lists_addresses_read_one_way_only_where_the_server_sends_email() {
    let dir = folder("shares-people-listed");
    block_on!(async {
        let outbox = Arc::new(Outbox::default());
        let (server, of, _) = drawn_fox(&dir, &outbox).await;
        let Ok(Reply::Server(info)) = server.request(Request::Discover {}).await else {
            panic!("the server");
        };
        assert_eq!(info.shares, [ShareChoice::People, ShareChoice::Anyone]);
        let refused = shared(&server, &of, people(&["Ada <ada@example.com>"])).await;
        assert_eq!(refused.unwrap_err().code, code::INVALID_PARAMS);
        assert_eq!(
            shared(&server, &of, people(&[])).await.unwrap_err().code,
            code::INVALID_PARAMS
        );
        let share = shared(
            &server,
            &of,
            people(&["Ada@Example.com", "ada@example.com"]),
        )
        .await
        .unwrap();
        assert_eq!(share.audience, people(&["ada@example.com"]));
        assert_eq!(&*share.said, "Shared with 1 person");
    });
}

/// A browser at a share's own name on the server at `address`, following no redirect.
struct Browser {
    client: reqwest::Client,
    base: String,
}

impl Browser {
    fn new(host: &str, address: SocketAddr) -> Self {
        let client = reqwest::Client::builder()
            .resolve(host, address)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .unwrap();
        let base = format!("http://{host}:{}", address.port());
        Self { client, base }
    }

    async fn get(&self, path: &str, cookie: &str) -> reqwest::Response {
        let url = format!("{}{path}", self.base);
        self.client
            .get(url)
            .header("cookie", cookie)
            .send()
            .await
            .unwrap()
    }

    async fn post(&self, path: &str, form: &str) -> reqwest::Response {
        let url = format!("{}{path}", self.base);
        let form = form.to_owned();
        let posting = self.client.post(url).body(form);
        let posting = posting.header("content-type", "application/x-www-form-urlencoded");
        posting.send().await.unwrap()
    }

    /// Opens the emailed link `text` holds with one click: the proof's cookie it is given, or
    /// none, with the page answered.
    async fn open(&self, text: &str) -> (Option<String>, String) {
        let (_, rest) = text.split_once("/open/").unwrap();
        let token = rest.split_whitespace().next().unwrap();
        let page = self
            .get(&format!("/open/{token}"), "")
            .await
            .text()
            .await
            .unwrap();
        assert!(
            page.contains("name=\"token\"") && !page.contains("fox"),
            "{page}"
        );
        let opened = self.post("/open", &format!("token={token}")).await;
        let cookie =
            (opened.headers().get("set-cookie")).map(|cookie| cookie.to_str().unwrap().to_owned());
        (cookie, opened.text().await.unwrap())
    }
}

#[test]
fn only_a_listed_address_gets_a_link_and_its_proof_ends_with_any_change_to_who_may_open() {
    let dir = folder("shares-people");
    block_on!(async {
        let outbox = Arc::new(Outbox::default());
        let (server, of, _) = drawn_fox(&dir, &outbox).await;
        let share = shared(&server, &of, people(&["ada@example.com"]))
            .await
            .unwrap();
        let local = LocalToken::new([7; 32]);
        let signin = SignIn::local(local, Box::new(fixed_clock()));
        let listening = serve_with(server.clone(), signin, None).await;
        let id = share
            .link
            .trim_start_matches("https://")
            .split('.')
            .next()
            .unwrap();
        let browser = Browser::new(&format!("{id}.share.example.net"), listening.address);

        // Without a proof, the page asks for an email, naming nothing it shares.
        let asked = browser.get("/", "").await.text().await.unwrap();
        assert!(
            asked.contains("Enter your email") && !asked.contains("fox"),
            "{asked}"
        );
        assert_eq!(browser.get("/file", "").await.status(), 404);

        // An address the share doesn't list is answered alike, and sent nothing.
        let stranger = browser.post("/link", "email=bea%40example.com").await;
        let listed = browser.post("/link", "email=ada%40example.com").await;
        assert_eq!(listed.text().await.unwrap(), stranger.text().await.unwrap());
        let mails = outbox.sent_by(1).await;
        let [mail] = mails.as_slice() else {
            panic!("one link, to ada: {mails:?}");
        };
        assert_eq!(mail.to, "ada@example.com");

        // One click opens it, once, with a proof for this name alone.
        let (cookie, _) = browser.open(&mail.text).await;
        let cookie = cookie.expect("a proof");
        assert!(
            cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax"),
            "{cookie}"
        );
        assert!(!cookie.contains("Domain"), "host-only: {cookie}");
        let proof = cookie.split(';').next().unwrap().to_owned();
        let (again, said) = browser.open(&mail.text).await;
        assert_eq!(again, None, "a link works once");
        assert!(said.contains("used or have expired"), "{said}");
        let page = browser.get("/", &proof).await.text().await.unwrap();
        assert!(page.contains("<h1>A fox at dawn</h1>"), "{page}");
        assert_eq!(browser.get("/file", &proof).await.status(), 200);

        // Taken off and put back, or the share opened to anyone and closed again, the proof
        // given before opens it no more.
        for audience in [people(&["bea@example.com"]), people(&["ada@example.com"])] {
            shared(&server, &of, audience).await.unwrap();
        }
        let page = browser.get("/", &proof).await.text().await.unwrap();
        assert!(page.contains("Enter your email"), "re-added: {page}");
        listening.close().await;
    });
}

#[test]
fn asking_for_another_link_leaves_the_first_working() {
    let dir = folder("shares-people-again");
    block_on!(async {
        let outbox = Arc::new(Outbox::default());
        let (server, of, clock) = drawn_fox(&dir, &outbox).await;
        let share = shared(&server, &of, people(&["ada@example.com"]))
            .await
            .unwrap();
        let local = LocalToken::new([7; 32]);
        let signin = SignIn::local(local, Box::new(fixed_clock()));
        let listening = serve_with(server.clone(), signin, None).await;
        let id = share
            .link
            .trim_start_matches("https://")
            .split('.')
            .next()
            .unwrap();
        let browser = Browser::new(&format!("{id}.share.example.net"), listening.address);
        browser.post("/link", "email=ada%40example.com").await;
        let first = outbox.sent_by(1).await[0].text.clone();
        clock.advance_millis(61_000);
        browser.post("/link", "email=ada%40example.com").await;
        let mails = outbox.sent_by(2).await;
        assert_eq!(mails.len(), 2, "a minute after, a second link");
        let (cookie, _) = browser.open(&first).await;
        assert!(cookie.is_some(), "the first link still opens it");
        listening.close().await;
    });
}

//! Sharing a file at a link of its own (`specs/session.md`, "Sharing"): only where the server
//! names a share domain, as `server/discover` says; a frozen copy, served at a name of its own
//! under that domain to anyone with the link and nothing of glave's there, nor any share at
//! glave's own address; changing who may open it keeps the link, a newer version moves to it
//! only when asked, and Only you stops it at once (`specs/invariants.md`, 37).

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::{Audience, Parts, ThreadId};
use glave_identity::signin::{LocalToken, SignIn};
use glave_ports::host::PublicOrigin;
use glave_ports::{MadeRead, ReadMade};
use glave_session::ShareDomain;
use glave_session::protocol::{
    LibraryItem, Reply, Request, ShareChoice, ShareInfo, ShareOf, ShareSetting, code,
};
use serde_json::json;

use crate::support::issuer::fixed_clock;
use crate::support::web::serve_with;
use crate::support::*;

/// The share domain the tests share at.
fn domain() -> ShareDomain {
    ShareDomain {
        origin: PublicOrigin::parse("https://share.example.net").unwrap(),
        report: Some("abuse@example.net".to_owned()),
    }
}

/// A server sharing at [`domain`], whose model draws a fox, and the fox in its Library.
async fn shared_fox(dir: &std::path::Path) -> (Tested, LibraryItem) {
    let mut setup = Setup::new(Vec::new());
    setup.model = Some(drawing(vec![
        answer(
            None,
            Some(("image_generate", json!({"prompt": "a fox at dawn"}))),
        ),
        answer(Some("Here it is."), None),
    ]));
    setup.shares = Some(domain());
    let (server, _) = setup.build(dir).await;
    let thread = start(&server, false).await;
    turn(&server, thread, "draw me a fox").await.unwrap();
    let Ok(Reply::Library { items }) = server.request(Request::LibraryList {}).await else {
        panic!("the Library");
    };
    (server, items.into_iter().next().unwrap())
}

fn item_of(item: &LibraryItem) -> ShareOf {
    ShareOf::Item {
        project: item.project,
        asset: item.asset,
    }
}

/// `share/set` of `of` for `audience`, moving it to `of`'s version when `update`.
async fn set(
    server: &Tested,
    of: ShareOf,
    audience: Option<Audience>,
    update: bool,
) -> Result<Option<ShareInfo>, glave_session::protocol::ApiError> {
    let setting = ShareSetting {
        of,
        audience,
        update,
    };
    match server.request(Request::ShareSet { set: setting }).await? {
        Reply::Share { share } => Ok(share),
        other => panic!("a share, not {other:?}"),
    }
}

async fn get(server: &Tested, of: ShareOf) -> Option<ShareInfo> {
    match server.request(Request::ShareGet { of }).await.unwrap() {
        Reply::Share { share } => share,
        other => panic!("a share, not {other:?}"),
    }
}

/// The id a link names under the share domain.
fn id_of(link: &str) -> String {
    let rest = link.strip_prefix("https://").unwrap();
    let (id, domain) = rest.split_once('.').unwrap();
    assert_eq!(domain, "share.example.net/");
    assert_eq!(id.len(), 26, "128 bits");
    assert!(
        id.bytes()
            .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
    );
    id.to_owned()
}

#[test]
fn a_picture_is_shared_at_a_link_of_its_own_until_only_you_stops_it() {
    let dir = folder("shares-picture");
    block_on!(async {
        let (server, item) = shared_fox(&dir).await;
        let Ok(Reply::Server(info)) = server.request(Request::Discover {}).await else {
            panic!("the server");
        };
        assert_eq!(info.shares, [ShareChoice::Anyone]);
        assert_eq!(get(&server, item_of(&item)).await, None, "not shared yet");

        let shared = set(&server, item_of(&item), Some(Audience::Anyone), false)
            .await
            .unwrap()
            .unwrap();
        let id = id_of(&shared.link);
        assert_eq!(&*shared.said, "Shared with anyone who has the link");
        assert_eq!(
            get(&server, item_of(&item)).await.unwrap().link,
            shared.link
        );
        let Ok(Reply::Library { items }) = server.request(Request::LibraryList {}).await else {
            panic!("the Library");
        };
        let said = items[0].shared.as_deref().map(str::to_owned);
        assert_eq!(said.as_deref(), Some("Shared with anyone who has the link"));

        // Specific people wait for the codes that prove them.
        let people = Audience::People {
            people: vec!["ada@example.com".to_owned()],
        };
        let refused = set(&server, item_of(&item), Some(people), false).await;
        assert_eq!(refused.unwrap_err().code, code::NOT_AVAILABLE);

        // Only you stops it: its link opens nothing, and sharing again makes a new one.
        assert_eq!(
            set(&server, item_of(&item), None, false).await.unwrap(),
            None
        );
        assert_eq!(get(&server, item_of(&item)).await, None);
        let again = set(&server, item_of(&item), Some(Audience::Anyone), false)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(id_of(&again.link), id);
    });
}

#[test]
fn without_a_share_domain_nothing_is_offered_or_shared() {
    let dir = folder("shares-none");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.model = Some(drawing(vec![
            answer(None, Some(("image_generate", json!({"prompt": "a fox"})))),
            answer(Some("Here."), None),
        ]));
        let (server, _) = setup.build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "draw").await.unwrap();
        let Ok(Reply::Server(info)) = server.request(Request::Discover {}).await else {
            panic!("the server");
        };
        assert!(info.shares.is_empty());
        let Ok(Reply::Library { items }) = server.request(Request::LibraryList {}).await else {
            panic!("the Library");
        };
        let refused = set(&server, item_of(&items[0]), Some(Audience::Anyone), false).await;
        assert_eq!(refused.unwrap_err().code, code::NOT_AVAILABLE);
    });
}

/// A reader standing in for glave's own: a CSV file's preview is its text.
fn previewing() -> ReadMade {
    Arc::new(|bytes, _| {
        Box::pin(async move {
            Some(MadeRead {
                parts: Some(Parts::Sheets(1)),
                markdown: Some(String::from_utf8(bytes).ok()?),
                ..MadeRead::default()
            })
        })
    })
}

/// The kept files of `thread`'s turns, in order: their content's hashes and what they say of
/// being shared.
async fn made(server: &Tested, thread: ThreadId) -> Vec<(glave_event::Sha256, Option<String>)> {
    let Ok(Reply::Files { made }) = server.request(Request::ThreadFiles { thread }).await else {
        panic!("files");
    };
    made.into_iter()
        .flat_map(|turn| turn.files)
        .map(|file| {
            (
                file.sha256.unwrap(),
                file.shared.map(|said| said.to_string()),
            )
        })
        .collect()
}

#[test]
fn a_made_file_is_shared_as_it_was_until_moved_to_its_newer_version() {
    let dir = folder("shares-made");
    block_on!(async {
        let write = |content: &str| json!({"path": "budget.csv", "content": content});
        let mut setup = Setup::new(vec![
            answer(None, Some(("write", write("Item,Total\nFlights,780\n")))),
            answer(Some("Made it."), None),
            answer(None, Some(("write", write("Item,Total\nFlights,640\n")))),
            answer(Some("Changed it."), None),
        ]);
        setup.settings.titles = false;
        setup.reader = Some(previewing());
        setup.shares = Some(domain());
        let (server, _) = setup.build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "make a budget").await.unwrap();
        let first = made(&server, thread).await[0].0.clone();
        let of = |sha256: &glave_event::Sha256| ShareOf::File {
            thread,
            sha256: sha256.clone(),
        };
        let shared = set(&server, of(&first), Some(Audience::Anyone), false)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(shared.sha256, first);

        turn(&server, thread, "flights are cheaper").await.unwrap();
        let files = made(&server, thread).await;
        let [(old, old_said), (new, new_said)] = files.as_slice() else {
            panic!("two versions: {files:?}");
        };
        assert_eq!(*old, first);
        let said = Some("Shared with anyone who has the link".to_owned());
        assert_eq!((old_said, new_said), (&said, &said), "one file, one share");

        // Changing who may open it from the newer card keeps the version shared...
        let kept = set(&server, of(new), Some(Audience::Anyone), false)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (kept.link.as_str(), &kept.sha256),
            (shared.link.as_str(), old)
        );
        // ...until it is moved there, the same link.
        let moved = set(&server, of(new), Some(Audience::Anyone), true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (moved.link.as_str(), &moved.sha256),
            (shared.link.as_str(), new)
        );
    });
}

/// A GET of `path` at `id`'s name under the share domain, on the server served at `address`.
async fn fetch(address: std::net::SocketAddr, host: &str, path: &str) -> reqwest::Response {
    let client = reqwest::Client::builder()
        .resolve(host, address)
        .no_proxy()
        .build()
        .unwrap();
    let url = format!("http://{host}:{}{path}", address.port());
    client.get(url).send().await.unwrap()
}

#[test]
fn a_link_serves_its_share_alone_at_a_name_of_its_own_and_stops_at_once() {
    let dir = folder("shares-served");
    block_on!(async {
        let (server, item) = shared_fox(&dir).await;
        let link = set(&server, item_of(&item), Some(Audience::Anyone), false)
            .await
            .unwrap()
            .unwrap()
            .link;
        let id = id_of(&link);
        let local = LocalToken::new([7; 32]);
        let signin = SignIn::local(local, Box::new(fixed_clock()));
        let served = serve_with(server.clone(), signin, None).await;
        let host = format!("{id}.share.example.net");

        let page = fetch(served.address, &host, "/").await;
        assert_eq!(page.status(), 200);
        let headers = page.headers().clone();
        let header = |name: &str| headers.get(name).unwrap().to_str().unwrap().to_owned();
        assert_eq!(header("cache-control"), "no-store");
        assert_eq!(header("referrer-policy"), "no-referrer");
        assert_eq!(header("x-robots-tag"), "noindex, nofollow");
        let policy = header("content-security-policy");
        assert!(policy.starts_with("default-src 'none'") && !policy.contains("script"));
        let text = page.text().await.unwrap();
        assert!(text.contains("<h1>A fox at dawn</h1>"), "{text}");
        assert!(text.contains("noindex") && text.contains("Shared with Glave"));
        assert!(text.contains("mailto:abuse@example.net") && !text.contains("<script"));

        let file = fetch(served.address, &host, "/file").await;
        assert_eq!(header_of(&file, "content-type"), "image/png");
        let pixel = STANDARD.decode(PIXEL).unwrap();
        assert_eq!(file.bytes().await.unwrap().as_ref(), pixel.as_slice());
        let download = fetch(served.address, &host, "/download").await;
        let saved = header_of(&download, "content-disposition");
        assert!(
            saved.starts_with("attachment; filename=\"a-fox-at-dawn.png\""),
            "{saved}"
        );
        assert!(header_of(&download, "content-security-policy").contains("sandbox"));

        // A share's name answers nothing of glave's: no API.
        let client = reqwest::Client::builder()
            .resolve(&host, served.address)
            .no_proxy()
            .build()
            .unwrap();
        let api = format!("http://{host}:{}/", served.address.port());
        let discover = json!({"jsonrpc": "2.0", "id": 1, "method": "server/discover"});
        let posted = client.post(api).json(&discover).send().await.unwrap();
        assert_eq!(posted.status(), 405);
        // Nor does glave's own address answer a share.
        let own = reqwest::get(format!("{}file", served.url)).await.unwrap();
        assert_eq!(own.status(), 404);

        // A link never made and one stopped are answered alike.
        let never = fetch(
            served.address,
            "abcdefghijklmnopqrstuvwxyz.share.example.net",
            "/",
        )
        .await;
        assert_eq!(never.status(), 404);
        let never = never.text().await.unwrap();
        set(&server, item_of(&item), None, false).await.unwrap();
        let stopped = fetch(served.address, &host, "/").await;
        assert_eq!(stopped.status(), 404);
        assert_eq!(stopped.text().await.unwrap(), never);
        assert_eq!(fetch(served.address, &host, "/file").await.status(), 404);
        served.close().await;
    });
}

fn header_of(response: &reqwest::Response, name: &str) -> String {
    response
        .headers()
        .get(name)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
}

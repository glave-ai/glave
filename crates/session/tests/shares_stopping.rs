//! What stops a share, and what never opens one (`specs/session.md`, "Sharing"): deleting the
//! chat that made the file, taking the item out of the Library, or a project removed; two shares
//! of one file made at once both stop with one Only you; someone taken out of the
//! organization shares nothing there; and any other name under the share domain opens nothing
//! of glave's.

use glave_event::{Audience, Body, PersonRemoved, Principal, decode};
use glave_identity::signin::{LocalToken, SignIn};
use glave_ports::stream::Reserved;
use glave_ports::{Caller, EventLog as _, Scope};
use glave_session::protocol::{Reply, Request, ShareOf};
use serde_json::json;

use crate::schedule_firing::append;
use crate::shares::{
    client_at, discover_at, fetch, get, id_of, item_of, previewing, set, shared_fox,
};
use crate::support::issuer::fixed_clock;
use crate::support::web::{Served, serve_with};
use crate::support::*;

/// `server` served over HTTP to the person on this computer.
async fn served<L: glave_ports::EventLog + 'static>(server: &Tested<L>) -> Served {
    let local = LocalToken::new([7; 32]);
    serve_with(
        server.clone(),
        SignIn::local(local, Box::new(fixed_clock())),
        None,
    )
    .await
}

/// What a link that opens nothing answers, on the server at `served`.
async fn nothing(served: &Served) -> String {
    let never = "abcdefghijklmnopqrstuvwxyz.share.example.net";
    let answered = fetch(served.address, never, "/").await;
    assert_eq!(answered.status(), 404);
    answered.text().await.unwrap()
}

/// What `link`'s page answers on the server at `served`: its status and its page.
async fn opened(served: &Served, link: &str) -> (u16, String) {
    let host = format!("{}.share.example.net", id_of(link));
    let answered = fetch(served.address, &host, "/").await;
    (answered.status().as_u16(), answered.text().await.unwrap())
}

#[test]
fn removing_the_item_or_deleting_the_chat_stops_its_shares_at_once() {
    let dir = folder("shares-stop-item");
    block_on!(async {
        let (server, item) = shared_fox(&dir).await;
        let link = set(&server, item_of(&item), Some(Audience::Anyone), false)
            .await
            .unwrap()
            .unwrap()
            .link;
        let listening = served(&server).await;
        assert_eq!(opened(&listening, &link).await.0, 200);
        let remove = Request::LibraryRemove {
            project: item.project,
            asset: item.asset,
        };
        assert!(matches!(server.request(remove).await, Ok(Reply::Done {})));
        let never = nothing(&listening).await;
        assert_eq!(opened(&listening, &link).await, (404, never));
        listening.close().await;
    });

    let dir = folder("shares-stop-chat");
    block_on!(async {
        let write = json!({"path": "budget.csv", "content": "Item,Total\n"});
        let mut setup = Setup::new(vec![
            answer(None, Some(("write", write))),
            answer(Some("Made it."), None),
        ]);
        setup.settings.titles = false;
        setup.reader = Some(previewing());
        setup.shares = Some(crate::shares::domain());
        let (server, _) = setup.build(&dir).await;
        let thread = start(&server, false).await;
        turn(&server, thread, "make a budget").await.unwrap();
        let Ok(Reply::Files { made }) = server.request(Request::ThreadFiles { thread }).await
        else {
            panic!("files");
        };
        let sha256 = made[0].files[0].sha256.clone().unwrap();
        let of = ShareOf::File { thread, sha256 };
        let shared = set(&server, of.clone(), Some(Audience::Anyone), false).await;
        let link = shared.unwrap().unwrap().link;
        let listening = served(&server).await;
        let delete = Request::ThreadDelete { thread };
        assert!(matches!(server.request(delete).await, Ok(Reply::Done {})));
        let never = nothing(&listening).await;
        assert_eq!(opened(&listening, &link).await, (404, never));
        listening.close().await;
    });
}

#[test]
fn two_shares_of_one_file_made_at_once_both_stop_with_only_you() {
    let dir = folder("shares-race");
    block_on!(async {
        let (server, item) = shared_fox(&dir).await;
        let (one, other) = tokio::join!(
            set(&server, item_of(&item), Some(Audience::Anyone), false),
            set(&server, item_of(&item), Some(Audience::Anyone), false),
        );
        let links = [one.unwrap().unwrap().link, other.unwrap().unwrap().link];
        assert_eq!(
            set(&server, item_of(&item), None, false).await.unwrap(),
            None
        );
        assert_eq!(get(&server, item_of(&item)).await, None);
        let listening = served(&server).await;
        let never = nothing(&listening).await;
        for link in &links {
            assert_eq!(
                opened(&listening, link).await,
                (404, never.clone()),
                "{link}"
            );
        }
        listening.close().await;
    });
}

#[test]
fn no_other_name_under_the_share_domain_opens_anything_of_glave_s() {
    let dir = folder("shares-names");
    block_on!(async {
        let (server, item) = shared_fox(&dir).await;
        let link = set(&server, item_of(&item), Some(Audience::Anyone), false)
            .await
            .unwrap()
            .unwrap()
            .link;
        let listening = served(&server).await;
        let never = nothing(&listening).await;
        let id = id_of(&link);
        for host in [
            "share.example.net".to_owned(),
            "login.share.example.net".to_owned(),
            format!("login.{id}.share.example.net"),
        ] {
            let page = fetch(listening.address, &host, "/").await;
            assert_eq!(page.status(), 404, "{host}");
            assert_eq!(page.text().await.unwrap(), never, "{host}");
            let status = discover_at(listening.address, &host).await;
            assert_ne!(status, 200, "no API at {host}");
        }
        // A HEAD reads the file's length, and none of its bytes.
        let host = format!("{id}.share.example.net");
        let client = client_at(&host, listening.address);
        let url = format!("http://{host}:{}/file", listening.address.port());
        let head = client.head(url).send().await.unwrap();
        assert_eq!(head.status(), 200);
        let length = head.headers()["content-length"]
            .to_str()
            .unwrap()
            .to_owned();
        assert_eq!(length, item.bytes.to_string());
        assert!(head.bytes().await.unwrap().is_empty());
        listening.close().await;
    });
}

#[test]
fn a_removed_project_s_shares_stop_and_its_links_open_nothing_as_one_never_made() {
    let dir = folder("shares-project");
    let garden = dir.join("garden");
    std::fs::create_dir_all(&garden).unwrap();
    block_on!(async {
        let log = Shared::default();
        let mut setup = Setup::new(Vec::new());
        setup.model = Some(drawing(vec![
            answer(None, Some(("image_generate", json!({"prompt": "a rose"})))),
            answer(Some("Here."), None),
        ]));
        setup.shares = Some(crate::shares::domain());
        let (server, _) = setup.build_on(&dir, log.clone()).await;
        let project = make_project(&server, "Garden", &garden).await;
        let start = Request::ThreadStart {
            project: Some(project),
            worktree: Some(false),
            background: false,
            computer: None,
            model: None,
            permissions: None,
            effort: None,
        };
        let Ok(Reply::Thread { thread }) = server.request(start).await else {
            panic!("a thread");
        };
        turn(&server, thread.id, "draw a rose").await.unwrap();
        let Ok(Reply::Library { items }) = server.request(Request::LibraryList {}).await else {
            panic!("the Library");
        };
        let link = set(&server, item_of(&items[0]), Some(Audience::Anyone), false)
            .await
            .unwrap()
            .unwrap()
            .link;
        let listening = served(&server).await;
        assert_eq!(opened(&listening, &link).await.0, 200);
        let removed = server.request(Request::ProjectRemove { project }).await;
        assert!(matches!(removed, Ok(Reply::Done {})), "{removed:?}");
        let never = nothing(&listening).await;
        assert_eq!(opened(&listening, &link).await, (404, never));
        // Stopped, so its link counts against its owner no more.
        let garden_scope = Scope::glaves_own(scope().org(), project);
        let read = log.read(&garden_scope, Reserved::Shares.id(), None, usize::MAX);
        let stopped = (read.await.unwrap().into_iter())
            .any(|raw| matches!(decode(raw).unwrap().body, Body::ShareStopped(_)));
        assert!(stopped, "its share stopped");
        listening.close().await;
    });
}

#[test]
fn someone_taken_out_of_the_organization_shares_nothing_there() {
    let dir = folder("shares-removed");
    block_on!(async {
        let log = Shared::default();
        let mut setup = Setup::new(Vec::new());
        setup.identity_provider = true;
        setup.model = Some(drawing(vec![
            answer(None, Some(("image_generate", json!({"prompt": "a fox"})))),
            answer(Some("Here."), None),
        ]));
        setup.shares = Some(crate::shares::domain());
        // Her turns run on a computer of her own.
        let homes = crate::homes::Here::new(&dir.join("people"));
        setup.homes = Some(std::sync::Arc::new(homes) as std::sync::Arc<dyn glave_harness::Homes>);
        let (server, _) = setup.build_on(&dir, log.clone()).await;
        let ada = signed_in("ada");
        let started = Request::ThreadStart {
            project: None,
            worktree: None,
            background: false,
            computer: None,
            model: None,
            permissions: None,
            effort: None,
        };
        let Ok(Reply::Thread { thread }) = server.request_as(&ada, started).await else {
            panic!("a thread");
        };
        let drawn = server.request_as(&ada, turn_start(thread.id, "draw a fox", None));
        drawn.await.unwrap();
        settle_for(&server, &ada).await;
        let Ok(Reply::Library { items }) = server.request_as(&ada, Request::LibraryList {}).await
        else {
            panic!("her Library");
        };
        let setting = glave_session::protocol::ShareSetting {
            of: item_of(&items[0]),
            audience: Some(Audience::Anyone),
            update: false,
        };
        let shared = server.request_as(&ada, Request::ShareSet { set: setting });
        let Ok(Reply::Share { share }) = shared.await else {
            panic!("her share");
        };
        let listening = served(&server).await;
        assert_eq!(opened(&listening, &share.link).await.0, 200);

        // Taken out on another server, before any server stops her shares: her link opens
        // nothing already.
        let people = Scope::of_organization(scope().org());
        let ledger = Reserved::People.id();
        let before = log.read(&people, ledger, None, usize::MAX).await.unwrap();
        let removed = PersonRemoved {
            person: id("ada"),
            by: Principal::person(server.person().unwrap()),
        };
        append(&log, &people, ledger, before.len() as u64 + 1, &removed).await;
        let never = nothing(&listening).await;
        assert_eq!(opened(&listening, &share.link).await, (404, never));

        // Taking her out stops them, as asked again it finishes what that server left.
        let remove = Request::PersonRemove { person: id("ada") };
        server.request_as(&Caller::default(), remove).await.unwrap();
        let hers = Scope::glaves_own(scope().org(), items[0].project);
        let ledger_read = log.read(&hers, Reserved::Shares.id(), None, usize::MAX);
        let stopped = (ledger_read.await.unwrap().into_iter())
            .any(|raw| matches!(decode(raw).unwrap().body, Body::ShareStopped(_)));
        assert!(stopped, "her share stopped");
        listening.close().await;
    });
}

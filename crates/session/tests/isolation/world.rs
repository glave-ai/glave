//! The isolation suite's world: three organizations on one store, served by two servers. Each
//! server's own organization has two people: the person on its computer and one signed in with
//! OIDC. The third is served by the first server beside its own, with two people signed in
//! with glave accounts, each request of theirs carrying it, so one server serves two
//! organizations. Each person
//! makes their own things as any client would (a project, threads, a schedule, a task,
//! memories; the person on the computer also browser sessions, accounts and a turn waiting for
//! them), and the world keeps the ids.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use glave_event::{Auth, AuthMethod, PrincipalId, ThreadId};
use glave_memory::{GlaveMemory, Memories, Personal, Rules};
use glave_ports::testing::{CountingEntropy, ManualClock, TempDir, any_secret};
use glave_ports::{Caller, Eraser, EventLog, OrgId, ProjectId, Projects, Scope};
use glave_session::protocol::{Notification, Reply, Request, TaskStatus, Update};
use glave_session::signin::{Browsers, person};
use glave_session::{Answering, TurnItem, TurnStream};
use serde_json::{Value, json};

use super::table::Kind;
use crate::connections::InMemory;
use crate::credentials::Hosts;
use crate::providers::Accounts;
use crate::support::*;

/// Who the signed-in people's identity provider is.
const ISSUER: &str = "https://id.example.com";

/// What a store gives each organization's server.
pub(super) struct Parts<L> {
    pub log: L,
    pub projects: Arc<dyn Projects>,
    pub eraser: Arc<dyn Eraser>,
}

/// One organization: its server, and the browsers it signs in.
pub(super) struct Org<L: EventLog + 'static> {
    pub server: Tested<L>,
    pub browsers: Arc<Browsers>,
}

/// One person: their organization, the server they call, how they call, and the ids of what
/// they made.
pub(super) struct Person {
    pub name: &'static str,
    /// Their organization: 0 and 1 are the servers' own, 2 the one the first serves beside.
    pub org: usize,
    /// The server they call, in [`World::orgs`].
    pub server: usize,
    pub caller: Caller,
    pub ids: BTreeMap<Kind, Vec<Value>>,
    /// Their browser's token, while they have one.
    pub token: Option<String>,
}

impl Person {
    /// Their first id of `kind`: theirs to call with beside another's.
    pub(super) fn own(&self, kind: Kind) -> Option<&Value> {
        self.ids.get(&kind).and_then(|ids| ids.first())
    }

    fn keep(&mut self, kind: Kind, id: Value) {
        self.ids.entry(kind).or_default().push(id);
    }
}

pub(super) struct World<L: EventLog + 'static> {
    pub orgs: Vec<Org<L>>,
    pub people: Vec<Person>,
    /// Turns waiting for the person, by organization and thread, followed until the end.
    pub held: Vec<(usize, ThreadId, TurnStream)>,
    /// The organizations' folders, removed with the world.
    _folders: Vec<TempDir>,
}

/// The organization, server project and the person on the computer of organization `n`.
fn home(n: u128) -> (Scope, PrincipalId, Scope) {
    let org = OrgId::from_uuid(uuid::Uuid::from_u128(n));
    let scope = Scope::glaves_own(org, ProjectId::from_uuid(uuid::Uuid::from_u128(0x10 + n)));
    let local = PrincipalId::try_from(uuid::Uuid::from_u128(
        0x0192_f5c0_0000_7000_8000_0000_0000_0100 + n,
    ))
    .unwrap();
    let personal = Scope::glaves_own(org, ProjectId::from_uuid(uuid::Uuid::from_u128(0x20 + n)));
    (scope, local, personal)
}

/// The organization the first server serves beside its own.
fn third() -> OrgId {
    OrgId::from_uuid(uuid::Uuid::from_u128(3))
}

/// `name`, signed in with a glave account, working in the third organization.
fn in_third(name: &str) -> Caller {
    let mut caller = signed_in(name);
    caller.auth = Some(Auth {
        method: AuthMethod::Account,
        issuer: None,
        subject: None,
        session: None,
    });
    caller.org = Some(third());
    caller
}

fn signed_in(name: &str) -> Caller {
    Caller {
        person: Some(person(ISSUER, name).unwrap()),
        auth: Some(Auth {
            method: AuthMethod::Oidc,
            issuer: Some(ISSUER.to_owned()),
            subject: Some(name.to_owned()),
            session: None,
        }),
        trace: None,
        profile: None,
        org: None,
        from: None,
    }
}

/// The server of organization `n` (1 or 2) on `parts`, in the fresh folder `dir`. Only the
/// first keeps model accounts and git hosts: the fakes' ids are made from the clock, so two
/// could share one by chance, which would read as a leak.
async fn serve<L: EventLog + Clone + 'static>(n: u128, parts: &Parts<L>, dir: &Path) -> Org<L> {
    let (scope, local, personal) = home(n);
    let clock = ManualClock::ticking(1_791_001_000_000);
    let rules = Rules {
        personal: Some(Personal {
            scope: personal,
            person: local,
        }),
        ..Rules::default()
    };
    let memories = Memories::new(
        parts.log.clone(),
        Arc::clone(&parts.eraser),
        rules,
        Box::new(clock.clone()),
        Box::new(CountingEntropy::starting_at(
            u64::try_from(0x40 + n).unwrap() << 32,
        )),
    );
    let browsers = Browsers::open(
        parts.log.clone(),
        personal,
        format!("server-{n}"),
        uuid::Uuid::from_u128(0x691 + n),
        None,
        Box::new(clock.clone()),
    );
    let browsers = Arc::new(browsers.await.unwrap());
    // A task for each person it serves (the first serves the third organization's two too),
    // then two held turns in the first organization: one asks to run a command, one asks a
    // question.
    let tasks = if n == 1 { 4 } else { 2 };
    let mut answers: Vec<_> = (0..tasks).map(|_| answer(Some("Done."), None)).collect();
    if n == 1 {
        answers.push(answer(
            None,
            Some(("bash", json!({"command": "touch made.txt"}))),
        ));
        let asks = json!({"questions": [{"question": "Which?"}]});
        answers.push(answer(None, Some(("question", asks))));
    }
    let mut setup = Setup::asking(answers, Answering::Clients);
    setup.home = scope;
    setup.seed = u64::try_from(n).unwrap();
    setup.settings.person = Some(local);
    setup.projects = Some(Arc::clone(&parts.projects));
    setup.memory = Some(Arc::new(GlaveMemory::start(memories).await.unwrap()));
    setup.browsers = Some(Arc::clone(&browsers));
    setup.identity_provider = true;
    if n == 1 {
        setup.providers = Some(Arc::new(Accounts::default()));
        setup.hosts = Some(Arc::new(Hosts::default()));
        setup.connections = Some(Arc::new(InMemory::default()));
    }
    let (server, _) = setup.build_on(dir, parts.log.clone()).await;
    Org { server, browsers }
}

/// Answers `request` from `caller`, as JSON: the reply, or the error.
pub(super) async fn ask<L: EventLog + 'static>(
    org: &Org<L>,
    caller: &Caller,
    request: Request,
) -> Value {
    match org.server.request_as(caller, request).await {
        Ok(reply) => json!({"result": reply}),
        Err(error) => json!({"error": error}),
    }
}

/// `method` with `params`.
pub(super) fn request(method: &str, params: Value) -> Request {
    let shown = params.to_string();
    let mut message = serde_json::Map::new();
    message.insert("method".to_owned(), Value::String(method.to_owned()));
    message.insert("params".to_owned(), params);
    serde_json::from_value(Value::Object(message))
        .unwrap_or_else(|e| panic!("{method} {shown}: {e}"))
}

/// What `person` made, as they would.
async fn make<L: EventLog + 'static>(org: &Org<L>, person: &mut Person, local: bool) {
    let caller = person.caller.clone();
    let call = async |method: &str, params: Value| {
        let answer = ask(org, &caller, request(method, params.clone())).await;
        assert!(answer.get("error").is_none(), "{method} {params}: {answer}");
        answer["result"].clone()
    };
    let name = format!("{}'s work", person.name);
    let project = call("project/create", json!({"name": name})).await["project"]["id"].clone();
    person.keep(Kind::Project, project.clone());
    let thread = call("thread/start", json!({"project": project})).await["thread"]["id"].clone();
    person.keep(Kind::Thread, thread.clone());
    let chat = call("thread/start", json!({})).await["thread"].clone();
    person.keep(Kind::Thread, chat["id"].clone());
    person.keep(Kind::Project, chat["project"].clone());
    let budget = json!({"project": project, "when": {"cron": "0 9 * * *"}, "prompt": "triage", "budget": 1000});
    let schedule = call("schedule/create", budget).await["schedule"]["schedule"].clone();
    person.keep(Kind::Schedule, schedule);
    let task = json!({"thread": thread, "prompt": "tidy", "background": true, "budget": 1000});
    call("turn/start", task).await;
    let notes = format!("{}'s notes", person.name);
    let data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, notes);
    let upload =
        json!({"project": project, "name": "notes.txt", "media_type": "text/plain", "data": data});
    let file = call("file/upload", upload).await["file"]["sha256"].clone();
    person.keep(Kind::Files, super::table::files(file.as_str().unwrap()));
    let saved = json!({"text": format!("{} likes tea", person.name), "scope": "project", "project": project});
    let saved = call("memory/save", saved).await;
    person.keep(Kind::Memory, saved["memories"][0]["memory"].clone());
    if !local {
        return;
    }
    let invited = call("invite/create", json!({"role": "admin"})).await;
    person.keep(Kind::Invite, invited["invite"]["id"].clone());
    person.keep(Kind::InviteCode, invited["code"].clone());
    let saved = json!({"text": "short answers", "scope": "personal", "project": project});
    let saved = call("memory/save", saved).await;
    let personal = saved["memories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|memory| memory["scope"] == "personal")
        .unwrap()["memory"]
        .clone();
    person.keep(Kind::Memory, personal);
    if org.server.request(Request::ProviderList {}).await.is_ok() {
        let account = json!({"kind": "openai", "key": any_secret()});
        let provider = call("provider/connect", account).await["provider"]["id"].clone();
        person.keep(Kind::Provider, provider);
        let host = json!({"url": "https://github.com", "token": any_secret()});
        let credential = call("credential/connect", host).await["credential"]["id"].clone();
        person.keep(Kind::Credential, credential);
        let flow = call("credential/device/start", json!({})).await["device"]["flow"].clone();
        person.keep(Kind::Flow, flow);
    }
}

impl<L: EventLog + Clone + 'static> World<L> {
    /// Two organizations on `parts`, in folders under `name`, each person's things made.
    pub(super) async fn build(name: &str, parts: &Parts<L>) -> Self {
        let (mut orgs, mut folders) = (Vec::new(), Vec::new());
        for n in 1..=2 {
            let dir = folder(&format!("{name}-{n}"));
            orgs.push(serve(n, parts, &dir).await);
            folders.push(dir);
        }
        let mut people = vec![
            Person::new("ada", 0, 0, Caller::default()),
            Person::new("alan", 0, 0, signed_in("alan")),
            Person::new("grace", 1, 1, Caller::default()),
            Person::new("linus", 1, 1, signed_in("linus")),
            Person::new("ken", 2, 0, in_third("ken")),
            Person::new("barbara", 2, 0, in_third("barbara")),
        ];
        for person in &mut people {
            let n = u128::try_from(person.org).unwrap() + 1;
            let id = person.caller.person.unwrap_or(home(n).1);
            person.keep(Kind::Person, json!(id));
            let local = person.caller.auth.is_none();
            make(&orgs[person.server], person, local).await;
        }
        let world = Self {
            orgs,
            people,
            held: Vec::new(),
            _folders: folders,
        };
        world.tasks_ended().await;
        world
    }

    /// What only the person on each computer has, made last so that nothing the people's own
    /// calls do ends it: a browser session, its link's code, and for the first organization
    /// a turn waiting on an approval and one on a question.
    pub(super) async fn sign_in_and_hold(&mut self) {
        for person in &mut self.people {
            if person.caller.auth.is_some() {
                continue;
            }
            let org = &self.orgs[person.server];
            let Ok(Reply::Link { code }) = org.server.request(Request::SigninLink {}).await else {
                panic!("a link");
            };
            let session = org.browsers.redeem(code.expose()).await.unwrap();
            let Ok(Reply::Link { code }) = org.server.request(Request::SigninLink {}).await else {
                panic!("a link");
            };
            person.keep(Kind::Code, Value::String(code.expose().to_owned()));
            person.keep(Kind::Session, Value::String(session.id));
            person.token = Some(session.token.expose().to_owned());
        }
        let ada = &mut self.people[0];
        let project = ada.own(Kind::Project).unwrap().clone();
        for (kind, prompt) in [(Kind::Approval, "make it"), (Kind::Question, "ask me")] {
            let org = &self.orgs[0];
            let started = ask(
                org,
                &ada.caller,
                request("thread/start", json!({"project": project})),
            );
            let thread = started.await["result"]["thread"]["id"].clone();
            let thread = serde_json::from_value(thread).unwrap();
            let mut stream = stream_turn(&org.server, thread, prompt, None)
                .await
                .unwrap();
            let call = waiting(&mut stream).await;
            ada.keep(Kind::Thread, json!(thread));
            ada.keep(kind, json!(call));
            self.held.push((0, thread, stream));
        }
    }

    /// Ends the held turns, and lets every task end.
    pub(super) async fn settle(&mut self) {
        for (org, thread, mut stream) in self.held.drain(..) {
            let server = &self.orgs[org].server;
            let _ = server.request(Request::TurnCancel { thread }).await;
            let _ = stream.finish().await;
        }
        self.tasks_ended().await;
    }

    /// Waits until no task anyone can see runs.
    async fn tasks_ended(&self) {
        for person in &self.people {
            let org = &self.orgs[person.server];
            for _ in 0..100_000 {
                let listed = ask(org, &person.caller, Request::TasksList { limit: None }).await;
                let tasks: Vec<glave_session::protocol::TaskInfo> =
                    serde_json::from_value(listed["result"]["tasks"].clone()).unwrap();
                if tasks.iter().all(|task| task.status != TaskStatus::Running) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        }
    }
}

impl Person {
    fn new(name: &'static str, org: usize, server: usize, caller: Caller) -> Self {
        Self {
            name,
            org,
            server,
            caller,
            ids: BTreeMap::new(),
            token: None,
        }
    }
}

/// Follows `stream` until its turn waits for the person: the call it waits on.
async fn waiting(stream: &mut TurnStream) -> glave_event::CallId {
    while let Some(item) = stream.next().await {
        if let TurnItem::Note(note) = item
            && let Notification::Live { update, .. } = note.as_ref()
        {
            match update {
                Update::ApprovalPending { approval } => return approval.clone(),
                Update::Asked { question, .. } => return question.clone(),
                _ => {}
            }
        }
    }
    panic!("the turn ended without waiting for anyone");
}

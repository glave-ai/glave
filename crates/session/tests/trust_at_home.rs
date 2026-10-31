//! A project on a person's own computer asks them to trust it (`specs/policy.md`, "Project
//! trust"): from what was last read there, in the thread's summary, with the computer left asleep,
//! from any client of theirs, and from no one else's.

use std::path::Path;
use std::sync::{Arc, Mutex};

use glave_event::{Auth, AuthMethod, PrincipalId, ThreadId};
use glave_ports::{BoxFuture, Caller, Scope};
use glave_session::ProjectTrust;
use glave_session::protocol::{ApiError, Reply, Request, Status, TrustAsk, TrustChoice, TrustItem};

use crate::homes::{config_kept, later_on, started};
use crate::support::*;

/// What a person's project there asks, its `config.toml` the whole ask, and each yes kept.
#[derive(Default)]
struct AsksThere {
    trusted: Mutex<Vec<(Scope, PrincipalId, String)>>,
}

impl ProjectTrust for AsksThere {
    fn ask<'a>(&'a self, _folder: &'a Path) -> BoxFuture<'a, Result<Option<TrustAsk>, ApiError>> {
        Box::pin(async { panic!("nothing of this project's is on the server") })
    }

    fn trust<'a>(
        &'a self,
        _folder: &'a Path,
        _ask: &'a str,
    ) -> BoxFuture<'a, Result<bool, ApiError>> {
        Box::pin(async { panic!("nothing of this project's is on the server") })
    }

    fn ask_there<'a>(
        &'a self,
        scope: Scope,
        _root: &'a Path,
        config: &'a str,
    ) -> BoxFuture<'a, Result<Option<TrustAsk>, ApiError>> {
        let ask = config.trim().to_owned();
        let kept = self
            .trusted
            .lock()
            .unwrap()
            .iter()
            .any(|(there, _, said)| *there == scope && *said == ask);
        Box::pin(async move {
            Ok((!kept).then(|| TrustAsk {
                ask: ask.clone(),
                items: vec![TrustItem {
                    text: "Runs a command at the end of each turn".into(),
                    detail: Some(ask),
                }],
            }))
        })
    }

    fn trust_there<'a>(
        &'a self,
        scope: Scope,
        person: PrincipalId,
        _root: &'a Path,
        _config: &'a str,
        ask: &'a str,
    ) -> BoxFuture<'a, Result<bool, ApiError>> {
        self.trusted
            .lock()
            .unwrap()
            .push((scope, person, ask.to_owned()));
        Box::pin(async { Ok(true) })
    }
}

/// `person`, signed in from somewhere else than this computer.
fn signed_in(person: Option<PrincipalId>, subject: &str) -> Caller {
    Caller {
        person,
        auth: Some(Auth {
            method: AuthMethod::Oidc,
            issuer: Some("https://id.example.com".into()),
            subject: Some(subject.into()),
            session: None,
        }),
        trace: None,
        profile: None,
        org: None,
        from: None,
    }
}

/// The ask `thread` shows `caller`, as they open it.
async fn shown(server: &Tested<Shared>, caller: &Caller, thread: ThreadId) -> Option<TrustAsk> {
    let opened = server
        .request_as(caller, Request::ThreadGet { thread })
        .await;
    let Reply::Thread { thread } = opened.unwrap() else {
        panic!("the thread's summary");
    };
    thread.trust
}

#[test]
fn a_project_on_a_person_s_computer_asks_them_from_any_client_and_keeps_their_yes() {
    let dir = folder("session-homes-trust");
    block_on!(async {
        let log = Shared::default();
        let (server, project) = config_kept(&dir, &log, "the hook\n").await;
        let host = Arc::new(AsksThere::default());
        let mut next = Setup::new(Vec::new());
        next.trust = Some(Arc::clone(&host) as Arc<dyn ProjectTrust>);
        let (later, next) = later_on(next, &dir, &server);
        let (again, _) = next.build_on(&dir, log).await;
        let person = again.person().unwrap();

        // Opened here, it waits for them, and asking reaches nothing.
        let waiting = started(&again, Some(project)).await;
        let asked = waiting.trust.clone().expect("it asks its person");
        assert_eq!(asked.ask, "the hook");
        assert_eq!(waiting.status, Status::Waiting);
        assert_eq!(later.asked(), 0, "asking wakes nothing");

        // From another client of theirs, signed in elsewhere, the same.
        let theirs = signed_in(Some(person), "ada");
        let there = shown(&again, &theirs, waiting.id).await;
        assert_eq!(there.map(|ask| ask.ask), Some(asked.ask.clone()));

        // Their yes is kept for this project, for them, and it asks no more anywhere.
        let answer = Request::TrustAnswer {
            thread: waiting.id,
            ask: asked.ask.clone(),
            answer: TrustChoice::Trust,
        };
        let Reply::Thread { thread: answered } = again.request_as(&theirs, answer).await.unwrap()
        else {
            panic!("the thread's summary");
        };
        assert_eq!(answered.trust, None);
        assert_eq!(shown(&again, &theirs, waiting.id).await, None);
        let kept = host.trusted.lock().unwrap().clone();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].0.project(), project);
        assert_eq!((kept[0].1, kept[0].2.as_str()), (person, "the hook"));
        assert_eq!(later.asked(), 0, "answering wakes nothing");
    });
}

#[test]
fn no_one_else_sees_or_answers_what_a_person_s_project_asks() {
    let dir = folder("session-homes-trust-others");
    block_on!(async {
        let log = Shared::default();
        let (server, project) = config_kept(&dir, &log, "the hook\n").await;
        let host = Arc::new(AsksThere::default());
        let mut next = Setup::new(Vec::new());
        next.trust = Some(Arc::clone(&host) as Arc<dyn ProjectTrust>);
        let (_later, next) = later_on(next, &dir, &server);
        let (again, _) = next.build_on(&dir, log).await;
        let waiting = started(&again, Some(project)).await;
        let asked = waiting.trust.expect("it asks its person");

        let someone = PrincipalId::new(jiff::Timestamp::UNIX_EPOCH, [7; 10]).unwrap();
        let other = signed_in(Some(someone), "grace");
        let looked = again
            .request_as(&other, Request::ThreadGet { thread: waiting.id })
            .await;
        assert!(looked.is_err(), "the thread isn't theirs to read");
        let answered = again
            .request_as(
                &other,
                Request::TrustAnswer {
                    thread: waiting.id,
                    ask: asked.ask.clone(),
                    answer: TrustChoice::Trust,
                },
            )
            .await;
        assert!(answered.is_err(), "nor the ask theirs to answer");
        assert!(host.trusted.lock().unwrap().is_empty(), "nothing is kept");
    });
}

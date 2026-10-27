//! Choosing a model through the session API: `model/list` names the models a server runs, the
//! default first; `thread/start` and `turn/start` choose one, recorded as `model.chosen`, so
//! the thread keeps it across turns and restarts; a model the server does not run is refused
//! with `-32021`; and each answer's `model.responded` names the model that wrote it.

use glave_event::{Body, ModelRef, Reasoning, ThreadId, TokenPrice, decode};
use glave_ports::Completion;
use glave_session::protocol::{ApiError, Meta, ModelInfo, Reply, Request, code};
use serde_json::{Value, json};

use crate::support::*;

/// The models offered besides the default: Hope, named and described, and another.
fn offered() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            name: "Hope".into(),
            maker: Some("Glave".into()),
            providers: vec!["OpenRouter".into()],
            context: Some(131_072),
            price: Some(TokenPrice {
                input: 100_000,
                output: 400_000,
                cache_read: None,
            }),
            reasoning: Some(Reasoning {
                efforts: vec!["high".into(), "low".into(), "none".into()],
                default: Some("low".into()),
                mandatory: false,
            }),
            ..ModelInfo::new("hope-12b")
        },
        ModelInfo::new("other"),
    ]
}

fn setup(answers: Vec<Completion>) -> Setup {
    let mut setup = Setup::new(answers);
    setup.models = offered();
    setup
}

/// An answer that says `model` wrote it.
fn from(model: &str, text: &str) -> Completion {
    let mut completion = answer(Some(text), None);
    completion.model = ModelRef {
        provider: "script".into(),
        name: model.into(),
    };
    completion
}

async fn start_on(server: &Tested<Shared>, model: Option<&str>) -> Result<ThreadId, ApiError> {
    let started = Request::ThreadStart {
        project: Some(scope().project()),
        worktree: None,
        background: false,
        computer: None,
        model: model.map(str::to_owned),
        permissions: None,
        effort: None,
    };
    match server.request(started).await? {
        Reply::Thread { thread } => Ok(thread.id),
        other => panic!("a thread, not {other:?}"),
    }
}

async fn turn_on(
    server: &Tested<Shared>,
    thread: ThreadId,
    model: Option<&str>,
) -> Result<Reply, ApiError> {
    server
        .request(Request::TurnStart {
            thread,
            prompt: "hi".into(),
            background: false,
            budget: None,
            meta: Meta::default(),
            model: model.map(str::to_owned),
            permissions: None,
            effort: None,
            unattended: false,
            files: Vec::new(),
        })
        .await
}

/// The model each request the provider received named, and the model its system prompt
/// says is answering.
fn sent(seen: &Seen) -> Vec<(String, bool)> {
    json(seen)
        .iter()
        .map(|json| {
            let request: Value = serde_json::from_str(json).unwrap();
            let model = request["model"].as_str().unwrap().to_owned();
            let system = request["system"].as_str().unwrap();
            let named = system.starts_with(&format!("You are the AI model {model},"));
            (model, named)
        })
        .collect()
}

/// The thread's events, decoded.
async fn events(server: &Tested<Shared>, thread: ThreadId) -> Vec<glave_event::Event> {
    let reply = server
        .request(Request::ThreadEvents {
            thread,
            after: None,
            limit: None,
        })
        .await
        .unwrap();
    let Reply::Events { events, .. } = reply else {
        panic!("events, not {reply:?}");
    };
    events.into_iter().map(|e| decode(e).unwrap()).collect()
}

/// The `turn.started` model name and the `model.chosen` model, in order.
fn models_recorded(events: &[glave_event::Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            Body::ModelChosen(chosen) => Some(format!("chosen {}", chosen.model)),
            Body::TurnStarted(started) => Some(format!("turn {}", started.model.name)),
            _ => None,
        })
        .collect()
}

#[test]
fn the_list_names_the_default_first_and_describes_each_model() {
    let dir = folder("models-list");
    block_on!(async {
        let (server, _) = setup(Vec::new()).build_on(&dir, Shared::default()).await;
        let Reply::Models { models } = server.request(Request::ModelList {}).await.unwrap() else {
            panic!("models");
        };
        let mut default = ModelInfo::new("test-model");
        default.default = true;
        let mut expected = vec![default];
        expected.extend(offered());
        assert_eq!(models, expected);
        let names: Vec<(&str, Option<&str>)> = models
            .iter()
            .map(|model| (model.name.as_str(), model.maker.as_deref()))
            .collect();
        assert_eq!(
            names,
            [
                ("Test Model", None),
                ("Hope", Some("Glave")),
                ("Other", None)
            ],
            "every model named, those no one named from their ids"
        );
        let json = serde_json::to_value(Reply::Models { models }).unwrap();
        assert_eq!(
            json["models"][1],
            serde_json::json!({
                "id": "hope-12b", "name": "Hope", "maker": "Glave", "default": false,
                "providers": ["OpenRouter"], "context": 131_072,
                "price": {"input": 100_000, "output": 400_000},
                "reasoning": {"efforts": ["high", "low", "none"], "default": "low"},
            })
        );
    });
}

#[test]
fn a_chosen_model_answers_every_turn_after_it_and_survives_a_restart() {
    let dir = folder("models-choose");
    let log = Shared::default();
    block_on!(async {
        let answers = vec![
            from("hope-12b", "One."),
            from("hope-12b", "Two."),
            from("other", "Three."),
        ];
        let (server, seen) = setup(answers).build_on(&dir, log.clone()).await;
        let thread = start_on(&server, Some("hope-12b")).await.unwrap();
        turn_on(&server, thread, None).await.unwrap();
        // Asking again for the model already chosen records nothing new.
        turn_on(&server, thread, Some("hope-12b")).await.unwrap();
        turn_on(&server, thread, Some("other")).await.unwrap();
        assert_eq!(
            sent(&seen),
            [
                ("hope-12b".to_owned(), true),
                ("hope-12b".to_owned(), true),
                ("other".to_owned(), true)
            ]
        );
        let recorded = events(&server, thread).await;
        assert_eq!(
            models_recorded(&recorded),
            [
                "chosen hope-12b",
                "turn hope-12b",
                "turn hope-12b",
                "chosen other",
                "turn other"
            ]
        );
        // Each assistant message is followed, before the next message, by the response
        // that names the model that wrote it.
        let mut writers = Vec::new();
        let mut wrote: Option<usize> = None;
        for event in &recorded {
            match &event.body {
                Body::MessageAdded(message) if message.role == glave_event::Role::Assistant => {
                    wrote = Some(writers.len());
                    writers.push(String::new());
                }
                Body::ModelResponded(responded) => {
                    if let Some(at) = wrote.take() {
                        writers[at].clone_from(&responded.model.name);
                    }
                }
                _ => {}
            }
        }
        assert_eq!(writers, ["hope-12b", "hope-12b", "other"]);
        server.shutdown().await;

        // A fresh server reads the choice from the log.
        let (restarted, seen) = setup(vec![from("other", "Four.")])
            .build_on(&dir, log.clone())
            .await;
        turn_on(&restarted, thread, None).await.unwrap();
        assert_eq!(sent(&seen), [("other".to_owned(), true)]);
        restarted.shutdown().await;
    });
}

#[test]
fn a_thread_that_chose_nothing_uses_the_default_and_its_prompt_as_written() {
    let dir = folder("models-default");
    block_on!(async {
        let (server, seen) = setup(vec![from("test-model", "Hi.")])
            .build_on(&dir, Shared::default())
            .await;
        let thread = start_on(&server, None).await.unwrap();
        turn_on(&server, thread, None).await.unwrap();
        assert_eq!(sent(&seen), [("test-model".to_owned(), true)]);
        assert_eq!(
            models_recorded(&events(&server, thread).await),
            ["turn test-model"]
        );
        server.shutdown().await;
    });
}

#[test]
fn a_model_the_server_does_not_run_is_refused() {
    let dir = folder("models-refused");
    let log = Shared::default();
    block_on!(async {
        let (server, seen) = setup(Vec::new()).build_on(&dir, log.clone()).await;
        let thread = start_on(&server, Some("other")).await.unwrap();
        let refused = turn_on(&server, thread, Some("elsewhere")).await;
        assert_eq!(refused.unwrap_err().code, code::NO_MODEL);
        assert!(seen.lock().unwrap().is_empty());
        server.shutdown().await;

        // A choice the server no longer offers is refused too, rather than sent anyway.
        let (restarted, seen) = Setup::new(Vec::new()).build_on(&dir, log).await;
        let refused = turn_on(&restarted, thread, None).await.unwrap_err();
        assert_eq!(refused.code, code::NO_MODEL);
        assert!(refused.message.contains("other"), "{refused}");
        assert!(seen.lock().unwrap().is_empty());
        restarted.shutdown().await;
    });
}

#[test]
fn a_background_turn_runs_on_the_model_it_chose_and_the_task_says_so() {
    block_on!(async {
        let (server, _, _, _dir) = setup(vec![from("other", "Done.")])
            .build_in_repository("models-task", Shared::default())
            .await;
        let thread = start_for_tasks(&server, None).await.unwrap();
        let reply = server
            .request(Request::TurnStart {
                thread,
                prompt: "tidy".into(),
                background: true,
                budget: None,
                meta: Meta::default(),
                model: Some("other".into()),
                permissions: None,
                effort: None,
                unattended: false,
                files: Vec::new(),
            })
            .await
            .unwrap();
        let Reply::Task { task } = reply else {
            panic!("a task, not {reply:?}");
        };
        assert_eq!(task.model, "other");
        server.shutdown().await;
    });
}

/// A turn of `thread` that chooses `effort`: a level, or Default with none.
fn with_effort(thread: ThreadId, effort: Option<&str>) -> Request {
    Request::TurnStart {
        thread,
        prompt: "hi".into(),
        background: false,
        budget: None,
        meta: Meta::default(),
        model: None,
        permissions: None,
        effort: Some(effort.map(str::to_owned)),
        unattended: false,
        files: Vec::new(),
    }
}

/// The level each request the provider received was sent with, `null` for none.
fn efforts_sent(seen: &Seen) -> Vec<Value> {
    json(seen)
        .iter()
        .map(|json| serde_json::from_str::<Value>(json).unwrap()["effort"].clone())
        .collect()
}

/// The level each `effort.chosen` in the thread records, `None` for Default.
async fn efforts_chosen(server: &Tested<Shared>, thread: ThreadId) -> Vec<Option<String>> {
    events(server, thread)
        .await
        .iter()
        .filter_map(|event| match &event.body {
            Body::EffortChosen(chosen) => Some(chosen.effort.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_reasoning_level_the_model_offers_is_kept_for_every_turn_after_it() {
    let dir = folder("models-effort");
    block_on!(async {
        let answers = vec![from("hope-12b", "One."), from("hope-12b", "Two.")];
        let (server, seen) = setup(answers).build_on(&dir, Shared::default()).await;
        let thread = start_on(&server, Some("hope-12b")).await.unwrap();
        let refused = server
            .request(with_effort(thread, Some("medium")))
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::INVALID_PARAMS);
        assert!(refused.message.contains("high, low, none"), "{refused}");
        server
            .request(with_effort(thread, Some("high")))
            .await
            .unwrap();
        turn_on(&server, thread, None).await.unwrap();
        let sent = efforts_sent(&seen);
        assert_eq!(sent, ["high", "high"], "kept for the turn after it");
        let chosen = efforts_chosen(&server, thread).await;
        assert_eq!(chosen, [Some("high".to_owned())]);
        let started = Request::ThreadStart {
            project: Some(scope().project()),
            worktree: None,
            background: false,
            computer: None,
            model: None,
            permissions: None,
            effort: Some(Some("high".into())),
        };
        let none = server.request(started).await.unwrap_err();
        assert!(
            none.message.contains("no choice"),
            "the default offers none: {none}"
        );
        server.shutdown().await;
    });
}

#[test]
fn a_thread_goes_back_to_default_and_is_sent_no_level_from_then_on() {
    let dir = folder("models-effort-default");
    let log = Shared::default();
    block_on!(async {
        let answers = vec![
            from("test-model", "Zero."),
            from("hope-12b", "One."),
            from("hope-12b", "Two."),
            from("hope-12b", "Three."),
        ];
        let (server, seen) = setup(answers).build_on(&dir, log.clone()).await;
        // Default is there to choose on a model that offers no levels, as on one that does,
        // and a thread that never chose is at it already.
        let plain = start_on(&server, None).await.unwrap();
        server.request(with_effort(plain, None)).await.unwrap();
        assert_eq!(efforts_chosen(&server, plain).await, []);

        let thread = start_on(&server, Some("hope-12b")).await.unwrap();
        server
            .request(with_effort(thread, Some("high")))
            .await
            .unwrap();
        server.request(with_effort(thread, None)).await.unwrap();
        // Choosing Default again records nothing more.
        server.request(with_effort(thread, None)).await.unwrap();
        let sent = efforts_sent(&seen);
        assert_eq!(sent, [Value::Null, json!("high"), Value::Null, Value::Null]);
        let chosen = efforts_chosen(&server, thread).await;
        assert_eq!(chosen, [Some("high".to_owned()), None]);
        server.shutdown().await;

        // A fresh server reads Default from the log.
        let (restarted, seen) = setup(vec![from("hope-12b", "Four.")])
            .build_on(&dir, log)
            .await;
        turn_on(&restarted, thread, None).await.unwrap();
        assert_eq!(efforts_sent(&seen), [Value::Null]);
        restarted.shutdown().await;
    });
}

#[test]
fn default_is_null_on_the_wire_and_a_level_left_out_keeps_the_thread_s() {
    let thread = "0192f5c0-11aa-7e21-8c3d-5f9b2e4a6c13";
    let read = |params: Value| -> Request {
        serde_json::from_value(json!({"method": "turn/start", "params": params})).unwrap()
    };
    let effort = |request: &Request| match request {
        Request::TurnStart { effort, .. } => effort.clone(),
        other => panic!("a turn, not {other:?}"),
    };
    let default = read(json!({"thread": thread, "prompt": "hi", "effort": null}));
    assert_eq!(effort(&default), Some(None));
    let kept = read(json!({"thread": thread, "prompt": "hi"}));
    assert_eq!(effort(&kept), None);
    let high = read(json!({"thread": thread, "prompt": "hi", "effort": "high"}));
    assert_eq!(effort(&high), Some(Some("high".to_owned())));
    for request in [default, kept, high] {
        let written = serde_json::to_value(&request).unwrap();
        assert_eq!(serde_json::from_value::<Request>(written).unwrap(), request);
    }
}

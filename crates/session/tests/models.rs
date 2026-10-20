//! Choosing a model through the session API: `model/list` names the models a server runs, the
//! default first; `thread/start` and `turn/start` choose one, recorded as `model.chosen`, so
//! the thread keeps it across turns and restarts; a model the server does not run is refused
//! with `-32021`; and each answer's `model.responded` names the model that wrote it.

use glave_event::{Body, ModelRef, Reasoning, ThreadId, TokenPrice, decode};
use glave_ports::Completion;
use glave_session::protocol::{ApiError, Meta, ModelInfo, Reply, Request, code};
use serde_json::Value;

use crate::support::*;

/// The models offered besides the default: Hope, named and described, and another.
fn offered() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            name: Some("Hope".into()),
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
        approval: None,
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
            approval: None,
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
                approval: None,
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

#[test]
fn a_reasoning_level_the_model_offers_is_kept_for_every_turn_after_it() {
    let dir = folder("models-effort");
    block_on!(async {
        let answers = vec![from("hope-12b", "One."), from("hope-12b", "Two.")];
        let (server, seen) = setup(answers).build_on(&dir, Shared::default()).await;
        let thread = start_on(&server, Some("hope-12b")).await.unwrap();
        let with_effort = |effort: &str| Request::TurnStart {
            thread,
            prompt: "hi".into(),
            background: false,
            budget: None,
            meta: Meta::default(),
            model: None,
            approval: None,
            effort: Some(effort.into()),
            unattended: false,
            files: Vec::new(),
        };
        let refused = server.request(with_effort("medium")).await.unwrap_err();
        assert_eq!(refused.code, code::INVALID_PARAMS);
        assert!(refused.message.contains("high, low, none"), "{refused}");
        server.request(with_effort("high")).await.unwrap();
        turn_on(&server, thread, None).await.unwrap();
        let efforts: Vec<Value> = json(&seen)
            .iter()
            .map(|json| serde_json::from_str::<Value>(json).unwrap()["effort"].clone())
            .collect();
        assert_eq!(efforts, ["high", "high"], "kept for the turn after it");
        let chosen: Vec<String> = events(&server, thread)
            .await
            .iter()
            .filter_map(|event| match &event.body {
                Body::EffortChosen(chosen) => Some(chosen.effort.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(chosen, ["high"]);
        let started = Request::ThreadStart {
            project: Some(scope().project()),
            worktree: None,
            background: false,
            computer: None,
            model: None,
            approval: None,
            effort: Some("high".into()),
        };
        let none = server.request(started).await.unwrap_err();
        assert!(
            none.message.contains("no choice"),
            "the default offers none: {none}"
        );
        server.shutdown().await;
    });
}

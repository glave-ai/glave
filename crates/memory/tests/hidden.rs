//! The turns a forget takes out of `recall` (`specs/memory.md`, "Seeing, editing and
//! forgetting"): every turn the memory was saved or edited in, however its project reads memory
//! now, and for a thread read again as for a new recall.

use glave_event::{Body, MemoryScope, MemorySource, Principal, Seq, ThreadId, decode};
use glave_harness::{AllowAll, Cancel};
use glave_memory::{Personal, Remembering, Rules, Writer};
use glave_ports::EventLog;
use glave_ports::memories::{EDIT_MEMORY, Memory, ProjectMemory, RECALL, REMEMBER, SHOWN};
use serde_json::json;

use crate::recall::{on, past, persons, persons_writer, thread};
use crate::support::*;

const FLOWER: &str = "favourite flower marigold";

/// What the recall in `thread`'s first memory result showed, by reference.
async fn shown(kit: &Kit, thread: ThreadId) -> serde_json::Value {
    let events = (kit.store.log())
        .read(&shared(), thread, None, usize::MAX)
        .await
        .unwrap();
    events
        .into_iter()
        .find_map(|raw| match decode(raw).ok()?.body {
            Body::ToolCallCompleted(done) => done.structured?.get(SHOWN).cloned(),
            _ => None,
        })
        .unwrap()
}

/// What a kept recall, `shown` in `thread`, shows when read again by `rules`.
async fn reread(
    kit: &Kit,
    rules: Rules,
    thread: ThreadId,
    shown: &serde_json::Value,
) -> Vec<String> {
    (kit.memory(rules, 6).await)
        .reread(&on(shared(), thread), std::slice::from_ref(shown))
        .await
        .unwrap()
        .into_iter()
        .flatten()
        .collect()
}

/// The rules with the shared project isolated.
fn isolated_here() -> Rules {
    let mut rules = rules();
    rules
        .projects
        .insert(shared().project(), ProjectMemory::Isolated);
    rules
}

/// The rules with the person's own memory off.
fn personal_off() -> Rules {
    let mut rules = rules();
    rules.personal = rules.personal.map(|personal| Personal {
        on: false,
        ..personal
    });
    rules
}

/// #585: a personal memory saved from a shared project's thread is forgotten in the person's
/// own ledger; once the project no longer reads it, its threads must still not find the turn.
#[test]
fn a_forgotten_memory_s_turn_stays_hidden_once_its_project_no_longer_reads_personal_memory() {
    block_on!(async {
        let kit = Kit::new();
        let memory = kit.memory(rules(), 2).await;
        let dir = folder("hidden-isolated");
        let save = json!({"text": "The favourite flower is marigold", "scope": "personal"});
        let answers = vec![
            answer(None, vec![(REMEMBER, save)]),
            said("Saved."),
            answer(None, vec![(RECALL, json!({"query": FLOWER}))]),
            said("Found it."),
        ];
        let (harness, _) = kit
            .harness(&dir, answers, AllowAll, Some(memory), settings())
            .await;
        let mut recalling = None;
        for said in [
            "Remember: my favourite flower is marigold.",
            "Which flower do I like best?",
        ] {
            let mut thread = harness.start_thread(shared()).await.unwrap();
            harness
                .run_turn(&mut thread, said, &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            recalling = Some(thread.id);
        }
        let recalling = recalling.unwrap();
        let shown = shown(&kit, recalling).await;
        let asking = on(shared(), thread(9));
        let turn = |texts: &[String]| texts.iter().any(|text| text.contains("is marigold."));
        let found = kit
            .memories(isolated_here(), 3)
            .recall(&asking, FLOWER, 10)
            .await
            .unwrap();
        assert!(turn(&persons(&found)), "isolated, a recall finds the turn");
        let read = reread(&kit, isolated_here(), recalling, &shown).await;
        assert!(turn(&read), "isolated, the earlier recall shows it");

        let memories = kit.memories(rules(), 4);
        let saved = memories
            .list_as(Some(person()), shared(), Some(MemoryScope::Personal))
            .await
            .unwrap();
        let saved = saved.first().unwrap().memory;
        memories.forget(&persons_writer(), saved).await.unwrap();

        for (now, rules) in [
            ("isolated", isolated_here()),
            ("personal memory off", personal_off()),
        ] {
            let hits = (kit.memories(rules.clone(), 5))
                .recall(&asking, FLOWER, 10)
                .await
                .unwrap();
            assert!(!turn(&persons(&hits)), "{now}: a new recall");
            let read = reread(&kit, rules, recalling, &shown).await;
            assert!(!turn(&read), "{now}: the earlier recall read again");
        }
    });
}

/// #586: a memory the model edited in another thread, then forgotten: the turn it was edited
/// in, whose messages state the newer text, is hidden with the turn it was saved in.
#[test]
fn forgetting_a_memory_hides_the_turns_it_was_edited_in_too() {
    block_on!(async {
        let kit = Kit::new();
        let asked = "Remember: we deploy with Fly.";
        past(&kit, shared(), thread(11), asked, ("Noted.", false)).await;
        let model = Writer {
            source: Some(MemorySource {
                thread: thread(11),
                seq: Seq::new(2).unwrap(),
            }),
            by: Principal::agent(agent()),
            ..persons_writer()
        };
        let memories = kit.memories(rules(), 3);
        let saved = memories
            .remember(
                &model,
                "We deploy with Fly",
                MemoryScope::Project,
                None,
                true,
            )
            .await
            .unwrap();
        let Remembering::Saved { memory: saved, .. } = saved else {
            panic!("forced");
        };
        let edit = json!({"memory": saved.memory, "text": "We deploy with Render"});
        let answers = vec![answer(None, vec![(EDIT_MEMORY, edit)]), said("Updated.")];
        let dir = folder("hidden-edited");
        let (harness, _) = kit
            .harness(
                &dir,
                answers,
                AllowAll,
                Some(kit.memory(rules(), 2).await),
                settings(),
            )
            .await;
        let mut editing = harness.start_thread(shared()).await.unwrap();
        harness
            .run_turn(
                &mut editing,
                "Actually, we deploy with Render now.",
                &Cancel::new(),
                &mut |_| {},
            )
            .await
            .unwrap();
        let asking = on(shared(), thread(9));
        let query = "deploy Fly Render";
        let before = memories.recall(&asking, query, 10).await.unwrap();
        assert_eq!(persons(&before).len(), 2, "both turns, before the forget");

        memories
            .forget(&persons_writer(), saved.memory)
            .await
            .unwrap();
        let after = (kit.memories(rules(), 4))
            .recall(&asking, query, 10)
            .await
            .unwrap();
        assert_eq!(persons(&after).len(), 0, "neither turn, after it");
    });
}

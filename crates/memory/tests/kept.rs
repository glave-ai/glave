//! Memory tools' results as the log keeps them (`specs/memory.md`, "Seeing, editing and
//! forgetting"): written as the model read them, and never searchable, so a forgotten memory's
//! text is never found again through a result that showed it.

use glave_event::Principal;
use glave_harness::{AllowAll, Cancel, Change, change_standing};
use glave_memory::{Source, Writer};
use glave_ports::memories::{RECALL, REMEMBER};
use glave_ports::{Clock as _, EventLog};
use serde_json::json;

use crate::recall::{on, thread};
use crate::support::*;

/// #578: a memory's text that came back to the model through a tool, here a save's and
/// `recall`'s results, is kept in those threads' logs as the model read it, and resumed, reads
/// the same; but search never indexes it, so once the memory is forgotten nothing finds it
/// there. The chat it was saved in is history, found until the person deletes it too (#630).
#[test]
#[expect(clippy::too_many_lines, reason = "one scenario, end to end")]
fn a_forgotten_memory_is_never_found_again_through_an_earlier_recall() {
    block_on!(async {
        let kit = Kit::new();
        let memory = kit.memory(rules(), 2).await;
        let dir = folder("recall-copies");
        let save = json!({"text": "The favourite flower is marigold", "scope": "personal"});
        let answers = vec![
            answer(None, vec![(REMEMBER, save)]),
            said("Saved."),
            answer(None, vec![(RECALL, json!({"query": "favourite flower"}))]),
            said("Found it."),
        ];
        let (harness, seen) = kit
            .harness(&dir, answers, AllowAll, Some(memory), settings())
            .await;
        let mut saving = harness.start_thread(shared()).await.unwrap();
        harness
            .run_turn(
                &mut saving,
                "Remember: my favourite flower is marigold.",
                &Cancel::new(),
                &mut |_| {},
            )
            .await
            .unwrap();
        let mut recalling = harness.start_thread(shared()).await.unwrap();
        harness
            .run_turn(
                &mut recalling,
                "Which flower do I like best?",
                &Cancel::new(),
                &mut |_| {},
            )
            .await
            .unwrap();
        let live = seen.lock().unwrap().last().unwrap().messages.clone();
        let result = format!("{:?}", live.last());
        assert!(
            result.contains("The favourite flower is marigold")
                && result.contains("Remember: my favourite flower is marigold."),
            "the model reads what recall found"
        );
        let log = kit.store.log();
        for (whose, thread) in [("save's", saving.id), ("recall's", recalling.id)] {
            let events = log.read(&shared(), thread, None, usize::MAX).await.unwrap();
            assert!(
                (events.iter()).any(|event| event.kind.as_str() == "tool.call.completed"
                    && event.data.get().contains("favourite flower is marigold")),
                "the {whose} result is kept as the model read it"
            );
        }
        // Read again by another process, the thread reads as it did.
        let (again, seen_again) = kit
            .harness(
                &dir,
                vec![said("Yes.")],
                AllowAll,
                Some(kit.memory(rules(), 6).await),
                settings(),
            )
            .await;
        let mut resumed = again.resume_thread(shared(), recalling.id).await.unwrap();
        again
            .run_turn(&mut resumed, "Still sure?", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let rebuilt = seen_again.lock().unwrap()[0].messages.clone();
        assert!(
            rebuilt.get(..live.len()) == Some(live.as_slice()),
            "resumed, the model reads what it read"
        );

        let memories = kit.memories(rules(), 3);
        let saved = memories
            .list_as(Some(person()), shared(), None)
            .await
            .unwrap()
            .first()
            .map(|memory| memory.memory)
            .unwrap();
        let writer = Writer {
            person: Some(person()),
            scope: shared(),
            by: Principal::person(person()),
            source: None,
            untrusted_read: false,
        };
        memories.forget(&writer, saved).await.unwrap();
        let asking = on(shared(), thread(9));
        let query = "favourite flower marigold";
        let said_there = |hit: &glave_memory::Recalled| matches!(hit.source, Source::Thread { thread, said_by: "person", .. } if thread == saving.id);
        let after = kit.memories(rules(), 4).recall(&asking, query, 10).await;
        let after = after.unwrap();
        assert!(
            after.iter().any(said_there)
                && (after.iter()).all(|hit| !hit.text.contains("marigold") || said_there(hit)),
            "a later recall finds nothing of the forgotten memory's text but what the person \
             said in the chat it was saved in, which is history: {after:?}"
        );
        // The person deletes that chat too.
        let now = kit.clock.now();
        change_standing(&log, &shared(), saving.id, now, [7; 10], Change::Delete)
            .await
            .unwrap();
        let after = kit.memories(rules(), 7).recall(&asking, query, 10).await;
        assert!(
            after
                .unwrap()
                .iter()
                .all(|hit| !hit.text.contains("marigold")),
            "a later recall finds nothing of it, though the recalling thread still holds it"
        );
    });
}

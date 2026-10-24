//! Memory tools' results kept by reference (#578, `specs/memory.md`, "Seeing, editing and
//! forgetting"): a forgotten memory's text stays nowhere a tool showed it, neither in a thread's
//! log nor in the search index, a past chat deleted since is read again as nothing, and a kept
//! result read again shows only what its reader may.

use glave_event::{Arguments, Principal};
use glave_harness::{AllowAll, Cancel, Change, change_standing};
use glave_memory::{Source, Writer};
use glave_ports::memories::{Memory, MemoryContext, RECALL, REMEMBER};
use glave_ports::{Clock as _, EventLog};
use serde_json::json;

use crate::recall::{on, thread, world};
use crate::support::*;

/// #578: a memory's text that came back to the model through a tool, here `recall`'s result,
/// stays nowhere a forget cannot reach: not in the thread's log, so not in the search index
/// either. Once the person also deletes the chat it was saved in (#630), a thread read again no
/// longer shows the model any of it.
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
        let saved = kit.results(shared(), saving.id).await;
        assert!(
            saved.len() == 1 && saved.iter().all(|kept| !kept.contains("marigold")),
            "the save's own result is kept without the text"
        );
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
        let said_there = |hit: &glave_memory::Recalled| matches!(hit.source, Source::Thread { thread, .. } if thread == saving.id);
        let after = kit.memories(rules(), 4).recall(&asking, query, 10).await;
        let after = after.unwrap();
        assert!(
            after.iter().any(said_there)
                && (after.iter()).all(|hit| !hit.text.contains("marigold") || said_there(hit)),
            "a later recall finds nothing of the forgotten memory's text but the chat it was \
             saved in, which is history: {after:?}"
        );
        // The person deletes that chat too.
        let (log, now) = (kit.store.log(), kit.clock.now());
        change_standing(&log, &shared(), saving.id, now, [7; 10], Change::Delete)
            .await
            .unwrap();
        let after = kit.memories(rules(), 7).recall(&asking, query, 10).await;
        assert!(
            after
                .unwrap()
                .iter()
                .all(|hit| !hit.text.contains("marigold")),
            "a later recall finds nothing of it"
        );
        let kept = kit
            .store
            .log()
            .read(&shared(), recalling.id, None, usize::MAX)
            .await
            .unwrap();
        assert!(
            kept.iter()
                .all(|event| !event.data.get().contains("marigold")),
            "the recalling thread's log keeps no copy of it"
        );

        // Read again after the forget, the recalling thread no longer shows it to the model.
        let (again, seen_again) = kit
            .harness(
                &dir,
                vec![said("back")],
                AllowAll,
                Some(kit.memory(rules(), 5).await),
                settings(),
            )
            .await;
        let mut resumed = again.resume_thread(shared(), recalling.id).await.unwrap();
        again
            .run_turn(&mut resumed, "And now?", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let sent = format!("{:?}", seen_again.lock().unwrap()[0].messages);
        assert!(
            !sent.contains("marigold"),
            "resumed, the model reads none of it"
        );
    });
}

/// A recall's result kept by reference names past conversations by place, which a later
/// process reads back from the log: read again, it shows only what the thread reading it may.
#[test]
fn a_kept_recall_reads_again_only_what_its_reader_may_read() {
    block_on!(async {
        let kit = Kit::new();
        world(&kit).await;
        let memory = kit.memory(rules(), 2).await;
        let mut query = serde_json::Map::new();
        query.insert("query".to_owned(), json!("orca"));
        let found = (memory.call(&on(shared(), thread(9)), RECALL, &Arguments::Json(query))).await;
        let shown = [found.kept.unwrap().shown];
        let read = async |reader: MemoryContext| -> String {
            let again = memory.reread(&reader, &shown).await.unwrap();
            again.into_iter().flatten().collect()
        };
        let here = read(on(shared(), thread(9))).await;
        assert!(
            ["host orca", "orca-7", "being retired", "laptop"]
                .iter()
                .all(|text| here.contains(text)),
            "where it was found, it reads as found"
        );
        let stranger = MemoryContext {
            person: Some(glave_event::PrincipalId::try_from(uuid(52)).unwrap()),
            ..on(shared(), thread(9))
        };
        let project = ["host orca", "orca-7", "being retired"];
        for (who, reader, theirs) in [
            (
                "from another project",
                on(scope(1, 5), thread(9)),
                &project[..],
            ),
            (
                "from another organization",
                on(scope(9, 6), thread(9)),
                &project[..],
            ),
            ("by someone else", stranger, &["laptop"][..]),
        ] {
            let again = read(reader).await;
            assert!(
                theirs.iter().all(|text| !again.contains(text)),
                "read again {who}, it shows only what they may read"
            );
        }
        assert!(
            !read(on(scope(9, 6), thread(9))).await.contains("laptop"),
            "nor the person's own from another organization"
        );
    });
}

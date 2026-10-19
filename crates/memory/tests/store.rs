//! The memory store on a sealed SQLite store: scopes enforced on every read and write, the
//! credential, duplicate and budget checks, forgetting that destroys the key, restoring glave's
//! changes, and housekeeping that rotates the project key.

#[cfg(test)]
#[path = "support/kit.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod store {
    use glave_event::{MemoryScope, Principal, decode};
    use glave_memory::{ProjectMemory, Remembering, Writer};
    use glave_ports::memories::{MemoryContext, MemoryError, memory_ledger};
    use glave_ports::testing::fake_secret;
    use glave_ports::{EventLog, Scope};

    use super::support::*;

    fn writer(scope: Scope) -> Writer {
        Writer {
            person: Some(person()),
            scope,
            by: Principal::person(person()),
            source: None,
            untrusted_read: false,
        }
    }

    fn on(scope: Scope) -> MemoryContext {
        MemoryContext {
            person: Some(person()),
            agent: Some(agent()),
            scope,
            thread: glave_event::ThreadId::try_from(uuid(77)).unwrap(),
            seq: None,
            untrusted_read: false,
        }
    }

    fn saved(remembering: Remembering) -> glave_ports::memories::Remembered {
        match remembering {
            Remembering::Saved { memory, .. } => memory,
            Remembering::Similar(similar) => panic!("not saved; similar to {similar:?}"),
        }
    }

    #[test]
    fn a_shared_project_reads_personal_and_project_memory_and_an_isolated_one_only_its_own() {
        block_on!(async {
            let kit = Kit::new();
            let memories = kit.memories(rules(), 1);
            let mine = writer(shared());
            saved(
                memories
                    .remember(
                        &mine,
                        "Ada prefers short answers",
                        MemoryScope::Personal,
                        None,
                        false,
                    )
                    .await
                    .unwrap(),
            );
            saved(
                memories
                    .remember(
                        &mine,
                        "Tests need make db first",
                        MemoryScope::Project,
                        None,
                        false,
                    )
                    .await
                    .unwrap(),
            );
            saved(
                memories
                    .remember(
                        &writer(isolated()),
                        "The client is BLUEHERON",
                        MemoryScope::Project,
                        None,
                        false,
                    )
                    .await
                    .unwrap(),
            );
            let texts = |list: Vec<glave_ports::memories::Remembered>| {
                list.into_iter().map(|m| m.text).collect::<Vec<_>>()
            };
            assert_eq!(
                texts(memories.list(&on(shared())).await.unwrap()),
                ["Ada prefers short answers", "Tests need make db first"]
            );
            assert_eq!(
                texts(memories.list(&on(isolated())).await.unwrap()),
                ["The client is BLUEHERON"],
                "personal memory never enters an isolated project"
            );
            assert_eq!(memories.list(&on(off())).await.unwrap().len(), 0);
            let refused = memories
                .remember(
                    &writer(isolated()),
                    "Ada likes tea",
                    MemoryScope::Personal,
                    None,
                    false,
                )
                .await;
            assert!(
                matches!(refused, Err(MemoryError::Refused(_))),
                "{refused:?}"
            );
            let other_org = scope(9, 2);
            assert!(
                texts(memories.list(&on(other_org)).await.unwrap()).is_empty(),
                "nothing crosses organizations"
            );
            let elsewhere = memories.list(&on(scope(1, 5))).await.unwrap();
            assert_eq!(
                texts(elsewhere),
                ["Ada prefers short answers"],
                "another project reads the person's memory, never this project's"
            );
        });
    }

    #[test]
    fn credentials_are_refused_and_never_stored() {
        block_on!(async {
            let kit = Kit::new();
            let memories = kit.memories(rules(), 1);
            let text = format!("use {} for the deploy", fake_secret("sk-proj-"));
            let refused = memories
                .remember(&writer(shared()), &text, MemoryScope::Project, None, true)
                .await;
            let Err(MemoryError::Invalid(why)) = refused else {
                panic!("{refused:?}");
            };
            assert!(why.contains("never keeps secrets"), "{why}");
            let ledger = kit
                .store
                .log()
                .read(&shared(), memory_ledger().unwrap(), None, 100)
                .await
                .unwrap();
            assert!(ledger.is_empty());
        });
    }

    #[test]
    fn a_near_duplicate_comes_back_instead_of_a_save_unless_forced() {
        block_on!(async {
            let kit = Kit::new();
            let memories = kit.memories(rules(), 1);
            let mine = writer(shared());
            let first = saved(
                memories
                    .remember(
                        &mine,
                        "Use tabs, not spaces, in this project's Python",
                        MemoryScope::Project,
                        None,
                        false,
                    )
                    .await
                    .unwrap(),
            );
            let again = memories
                .remember(
                    &mine,
                    "This project's Python uses tabs instead of spaces",
                    MemoryScope::Project,
                    None,
                    false,
                )
                .await
                .unwrap();
            assert_eq!(again, Remembering::Similar(vec![first.clone()]));
            let replaced = memories
                .remember(
                    &mine,
                    "Python here uses tabs, never spaces",
                    MemoryScope::Project,
                    Some(first.memory),
                    false,
                )
                .await
                .unwrap();
            let Remembering::Saved {
                replaced: Some(old),
                ..
            } = replaced
            else {
                panic!("{replaced:?}");
            };
            assert_eq!(old, first.memory);
            let live = memories.list(&on(shared())).await.unwrap();
            assert_eq!(live.len(), 1, "a correction replaces, never adds a second");
            saved(
                memories
                    .remember(
                        &mine,
                        "Python here uses tabs; never spaces",
                        MemoryScope::Project,
                        None,
                        true,
                    )
                    .await
                    .unwrap(),
            );
            assert_eq!(memories.list(&on(shared())).await.unwrap().len(), 2);
        });
    }

    #[test]
    fn a_save_past_the_budget_is_refused() {
        block_on!(async {
            let kit = Kit::new();
            let memories = kit.memories(rules(), 1);
            let mine = writer(shared());
            let mut refused = None;
            for n in 0..40 {
                let text = format!(
                    "Fact number {n}: {}",
                    "lorem ipsum dolor ".repeat(6 + n % 3)
                );
                match memories
                    .remember(&mine, &text, MemoryScope::Project, None, true)
                    .await
                {
                    Ok(_) => {}
                    Err(error) => {
                        refused = Some(error);
                        break;
                    }
                }
            }
            let Some(MemoryError::Invalid(why)) = refused else {
                panic!("the budget never ran out: {refused:?}");
            };
            assert!(why.contains("full"), "{why}");
            assert!(why.contains("Merge"), "{why}");
        });
    }

    #[test]
    fn forgetting_destroys_the_key_so_no_version_can_be_opened() {
        block_on!(async {
            let kit = Kit::new();
            let memories = kit.memories(rules(), 1);
            let mine = writer(shared());
            let kept = saved(
                memories
                    .remember(
                        &mine,
                        "We deploy with Render",
                        MemoryScope::Project,
                        None,
                        false,
                    )
                    .await
                    .unwrap(),
            );
            let gone = saved(
                memories
                    .remember(
                        &mine,
                        "The code word is marigold",
                        MemoryScope::Project,
                        None,
                        false,
                    )
                    .await
                    .unwrap(),
            );
            memories
                .edit(&mine, gone.memory, "The code word is marigold, really")
                .await
                .unwrap();
            memories.forget(&mine, gone.memory).await.unwrap();
            let ledger = memory_ledger().unwrap();
            let raw = kit
                .store
                .log()
                .read(&shared(), ledger, None, 100)
                .await
                .unwrap();
            let kinds: Vec<&str> = raw.iter().map(|e| e.kind.as_str()).collect();
            assert_eq!(
                kinds,
                [
                    "memory.saved",
                    "memory.saved",
                    "memory.edited",
                    "memory.forgotten"
                ]
            );
            for event in &raw {
                assert!(
                    !event.data.get().contains("marigold"),
                    "{}",
                    event.data.get()
                );
            }
            assert_eq!(raw[1].data.get(), "{}", "the saved text reads as erased");
            assert_eq!(raw[2].data.get(), "{}", "so does every edit");
            assert!(
                raw[0].data.get().contains("Render"),
                "other memories still open"
            );
            assert!(
                decode(raw[3].clone()).is_ok(),
                "the forget itself is content-free and kept"
            );
            let live = memories.list(&on(shared())).await.unwrap();
            assert_eq!(live, std::slice::from_ref(&kept));
            assert!(kit.store.keys().rotation_due(&shared()).await.unwrap());
            let done = memories.housekeep(&[shared(), inbox()]).await.unwrap();
            assert_eq!(done.rotated, 1, "housekeeping rotates the project key");
            assert!(!kit.store.keys().rotation_due(&shared()).await.unwrap());
            // A fresh process reads the same: the memory is gone, the other kept.
            let again = kit.memories(rules(), 2);
            assert_eq!(again.list(&on(shared())).await.unwrap(), [kept]);
        });
    }

    #[test]
    fn retention_forgets_by_glave_which_can_be_restored_until_the_key_goes() {
        block_on!(async {
            let kit = Kit::new();
            let mut rules = rules();
            rules.retention_days = Some(1);
            let memories = kit.memories(rules.clone(), 1);
            let mine = writer(shared());
            let old = saved(
                memories
                    .remember(&mine, "An old fact", MemoryScope::Project, None, false)
                    .await
                    .unwrap(),
            );
            kit.clock.advance_millis(2 * 86_400_000);
            let done = memories.housekeep(&[shared()]).await.unwrap();
            assert_eq!(done.forgotten, 1);
            assert_eq!(memories.list(&on(shared())).await.unwrap().len(), 0);
            let restored = memories.restore(&mine, old.memory).await.unwrap();
            assert_eq!(restored.text, "An old fact");
            assert_eq!(memories.list(&on(shared())).await.unwrap().len(), 1);
            let nothing = memories.restore(&mine, old.memory).await;
            assert!(
                matches!(nothing, Err(MemoryError::Invalid(_))),
                "{nothing:?}"
            );
        });
    }

    #[test]
    fn moving_saves_in_the_new_place_and_forgets_the_old() {
        block_on!(async {
            let kit = Kit::new();
            let memories = kit.memories(rules(), 1);
            let mine = writer(shared());
            let project = saved(
                memories
                    .remember(
                        &mine,
                        "Ada likes short answers",
                        MemoryScope::Project,
                        None,
                        false,
                    )
                    .await
                    .unwrap(),
            );
            let moved = memories
                .move_to(&mine, project.memory, MemoryScope::Personal)
                .await
                .unwrap();
            assert_eq!(moved.scope, MemoryScope::Personal);
            assert_ne!(moved.memory, project.memory);
            let elsewhere = memories.list(&on(scope(1, 5))).await.unwrap();
            assert_eq!(
                elsewhere,
                [moved],
                "personal now, so read in another project"
            );
        });
    }

    #[test]
    fn a_project_s_setting_follows_its_project_s_settings_while_memory_runs() {
        block_on!(async {
            let kit = Kit::new();
            let memories = kit.memories(rules(), 1);
            let mine = writer(shared());
            let save = || memories.remember(&mine, "A fact", MemoryScope::Project, None, false);
            memories.set_project(shared().project, ProjectMemory::Off);
            assert!(matches!(save().await, Err(MemoryError::Off(_))));
            memories.set_project(shared().project, ProjectMemory::Shared);
            assert!(
                !memories.rules().projects.contains_key(&shared().project),
                "the default again"
            );
            saved(save().await.unwrap());
        });
    }
}

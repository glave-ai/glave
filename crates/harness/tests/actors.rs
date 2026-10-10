//! Who acted: a thread records whose work it is, which agent runs it and what started it, and
//! a decision glave makes on its own is recorded as glave's.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod actors {
    use glave_event::{
        Auth, AuthMethod, Body, OriginKind, Principal, PrincipalId, PrincipalKind, Role,
        StopReason, Verdict,
    };
    use glave_harness::{Caller, Cancel, NoOneToAsk};
    use serde_json::json;

    use super::support::*;

    #[test]
    fn a_thread_records_its_person_its_agent_and_that_a_prompt_started_it() {
        let dir = folder("actors-started");
        block_on(async {
            let harness = harness(&dir, vec![], NoOneToAsk).await;
            let thread = harness.start_thread(scope()).await.unwrap();
            let first = events(harness.log(), thread.id).await;
            let Body::ThreadStarted(started) = &first[0].body else {
                panic!("{:?}", first[0].body);
            };
            assert_eq!(
                started.for_.as_ref().map(|p| &p.kind),
                Some(&PrincipalKind::Person)
            );
            assert_eq!(
                started.agent.as_ref().map(|p| &p.kind),
                Some(&PrincipalKind::Agent)
            );
            assert_eq!(started.origin.kind, OriginKind::Prompt);
            let again = harness.start_thread(scope()).await.unwrap();
            let Body::ThreadStarted(second) = &events(harness.log(), again.id).await[0].body else {
                panic!("not started");
            };
            assert_eq!(second.for_, started.for_, "one person for every thread");
            assert_eq!(second.agent, started.agent, "one agent for every thread");
        });
    }

    #[test]
    fn with_no_one_to_answer_the_refusal_is_recorded_as_glaves() {
        let dir = folder("actors-no-one");
        block_on(async {
            let answers = vec![
                answer(
                    None,
                    vec![("bash", json!({ "command": "git reset --hard" }))],
                    StopReason::ToolUse,
                ),
                answer(Some("Left it."), vec![], StopReason::EndTurn),
            ];
            let harness = harness(&dir, answers, NoOneToAsk).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "start over", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let decided: Vec<_> = events(harness.log(), thread.id)
                .await
                .into_iter()
                .filter_map(|event| match event.body {
                    Body::ApprovalDecided(decided) => Some((decided.decision, decided.by)),
                    _ => None,
                })
                .collect();
            assert_eq!(
                decided,
                vec![(Verdict::NoOneThere, Some(Principal::glave()))]
            );
        });
    }

    #[test]
    fn a_caller_other_than_the_threads_person_is_recorded_and_the_person_is_implied() {
        let dir = folder("actors-caller");
        block_on(async {
            let answers = vec![
                answer(Some("One."), vec![], StopReason::EndTurn),
                answer(Some("Two."), vec![], StopReason::EndTurn),
            ];
            let harness = harness(&dir, answers, NoOneToAsk).await;
            let first = harness.start_thread(scope()).await.unwrap();
            let Body::ThreadStarted(started) = &events(harness.log(), first.id).await[0].body
            else {
                panic!("not started");
            };
            let owner = started.for_.as_ref().and_then(|p| p.id);
            let ada = PrincipalId::new(jiff::Timestamp::UNIX_EPOCH, [9; 10]).unwrap();
            let auth = Auth {
                method: AuthMethod::Oidc,
                issuer: Some("https://id.example.com".into()),
                subject: Some("ada".into()),
                session: None,
            };
            let signed_in = Caller {
                person: Some(ada),
                auth: Some(auth.clone()),
            };
            let mut thread = harness.start_thread_as(scope(), &signed_in).await.unwrap();
            for caller in [
                &signed_in,
                &Caller {
                    person: owner,
                    auth: None,
                },
            ] {
                harness
                    .run_turn_as(&mut thread, "hi", caller, &Cancel::new(), &mut |_| {})
                    .await
                    .unwrap();
            }
            let mut origin = None;
            let mut auths = Vec::new();
            let mut authors = Vec::new();
            for event in events(harness.log(), thread.id).await {
                match event.body {
                    Body::ThreadStarted(started) => origin = started.origin.by,
                    Body::TurnStarted(turn) => auths.push(turn.auth),
                    Body::MessageAdded(added) if added.role == Role::User => {
                        authors.push(added.author);
                    }
                    _ => {}
                }
            }
            assert_eq!(origin, Some(Principal::person(ada)));
            assert_eq!(auths, [Some(auth), None]);
            assert_eq!(
                authors,
                [Some(Principal::person(ada)), None],
                "the owner is implied"
            );
        });
    }
}

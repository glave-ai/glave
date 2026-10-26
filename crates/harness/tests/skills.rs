//! Skills through the harness: with none, the model is sent exactly what it was sent before;
//! with some, it is offered `skill` too, and loading one never asks.

use std::path::Path;
use std::sync::Arc;

use glave_event::{Part, StopReason};
use glave_harness::{NoOneToAsk, Skills, find_skills, skill_folders};
use glave_ports::{InMemoryLog, Message, ModelRequest, ToolSource};
use serde_json::json;

use crate::support::*;

/// The tool sources for the skills in `dir`, as the CLI and the eval factory wire them.
async fn skill_sources(dir: &Path) -> Vec<Arc<dyn ToolSource>> {
    let found = find_skills(&skill_folders(dir, &[])).await;
    Skills::offer(found.skills)
        .map(|skills| Arc::new(skills) as Arc<dyn ToolSource>)
        .into_iter()
        .collect()
}

/// The first request of a one-answer turn, with the skills found in `dir` or without them.
async fn first_request(dir: &Path, with_skills: bool) -> ModelRequest {
    let answers = vec![answer(Some("Done."), vec![], StopReason::EndTurn)];
    let (harness, seen) =
        harness_seeing(dir, answers, NoOneToAsk, InMemoryLog::new(), 1, asking()).await;
    let harness = if with_skills {
        harness.with_tools(skill_sources(dir).await)
    } else {
        harness
    };
    let mut thread = harness.start_thread(scope()).await.unwrap();
    turn(&harness, &mut thread, "hello").await;
    seen.lock().unwrap().remove(0)
}

#[test]
fn without_skills_the_model_is_sent_exactly_what_it_was_before() {
    let dir = folder("skills-none");
    block_on!(async {
        let before = serde_json::to_string(&first_request(&dir, false).await).unwrap();
        let after = serde_json::to_string(&first_request(&dir, true).await).unwrap();
        assert_eq!(before, after);
    });
}

#[test]
fn with_skills_only_the_skill_tool_is_added_and_loading_one_never_asks() {
    let dir = folder("skills-some");
    let skill = dir.join(".agents/skills/house-style");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: house-style\ndescription: How code is written here.\n---\nUse tabs.\n",
    )
    .unwrap();
    block_on!(async {
        let before = first_request(&dir, false).await;
        let answers = vec![
            answer(
                None,
                vec![("skill", json!({"name": "house-style"}))],
                StopReason::ToolUse,
            ),
            answer(Some("Tabs it is."), vec![], StopReason::EndTurn),
        ];
        let (harness, seen) =
            harness_seeing(&dir, answers, NoOneToAsk, InMemoryLog::new(), 1, asking()).await;
        let harness = harness.with_tools(skill_sources(&dir).await);
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "style?").await;
        let after = seen.lock().unwrap().remove(0);
        assert_eq!(
            after.system, before.system,
            "the system prompt is unchanged"
        );
        let names = |request: &ModelRequest| -> Vec<String> {
            request.tools.iter().map(|spec| spec.name.clone()).collect()
        };
        let mut expected = names(&before);
        expected.push("skill".to_owned());
        assert_eq!(names(&after), expected);
        let loaded = thread.messages().iter().find_map(|message| match message {
            Message::Tool { output, .. } => output.iter().find_map(|part| match part {
                Part::Text { text } => Some(text.clone()),
                _ => None,
            }),
            _ => None,
        });
        assert!(
            loaded
                .as_deref()
                .is_some_and(|text| text.contains("Use tabs.")),
            "{loaded:?}"
        );
    });
}

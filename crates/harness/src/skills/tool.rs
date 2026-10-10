//! `skill`: loads one skill's instructions when a task needs them, as Claude Code's Skill tool
//! and OpenCode's skill tool do. Its description lists every skill by name and description, so
//! the catalog exists only while there are skills, and it stays out of the system prompt: like
//! AGENTS.md, skills come from files, so they guide the model without outranking glave's rules.

use std::fmt::Write as _;
use std::future::ready;

use glave_event::Arguments;
use glave_ports::{
    BoxFuture, Cancellation, Effect, ToolError, ToolInfo, ToolOutput, ToolSource, ToolSpec,
};
use serde_json::json;

use super::{Skill, frontmatter};

/// The tool's name.
pub const SKILL: &str = "skill";

/// The most of a body sent; the rest can be read from the file.
const MAX_BODY: usize = 32 * 1024;

/// The skills on offer, as a tool source.
#[derive(Debug, Clone)]
pub struct Skills {
    skills: Vec<Skill>,
}

impl Skills {
    /// The `skill` tool for `skills`, or `None` when there are none: then the model is offered
    /// no tool and told nothing new.
    pub fn offer(skills: Vec<Skill>) -> Option<Self> {
        (!skills.is_empty()).then_some(Self { skills })
    }

    fn spec(&self) -> ToolSpec {
        let mut listed = String::new();
        for skill in &self.skills {
            let _ = write!(
                listed,
                "<skill>\n<name>{}</name>\n<description>{}</description>\n</skill>\n",
                skill.name,
                escaped(&skill.description)
            );
        }
        let names: Vec<&str> = self
            .skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect();
        ToolSpec {
            name: SKILL.to_owned(),
            description: format!(
                "Load a skill: instructions for one kind of task, with any scripts and files \
                 they come with. When a task matches a skill's description, load it before you \
                 start and follow it. Run any script it names with bash.\n\n\
                 <available_skills>\n{listed}</available_skills>"
            ),
            parameters: json!({"type": "object", "properties": {
                "name": {"type": "string", "enum": names, "description": "The skill to load"}
            }, "required": ["name"]}),
        }
    }

    /// The skill a call names, forgiving case and stray whitespace.
    fn named(&self, arguments: &Arguments) -> Result<&Skill, String> {
        let Arguments::Json(map) = arguments else {
            return Err("The arguments were not valid JSON.".to_owned());
        };
        let name = map
            .get("name")
            .and_then(|value| value.as_str())
            .ok_or("Give the name of a skill.")?;
        let wanted = name.trim().to_ascii_lowercase();
        self.skills
            .iter()
            .find(|skill| skill.name == wanted)
            .ok_or_else(|| {
                let names: Vec<&str> = self
                    .skills
                    .iter()
                    .map(|skill| skill.name.as_str())
                    .collect();
                format!(
                    "There is no skill named {name:?}. The skills are: {}.",
                    names.join(", ")
                )
            })
    }
}

/// `text` safe inside the listing's tags.
fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The skill's body, read now so an edit since the thread began counts, with where its
/// files are.
async fn load(skill: &Skill) -> Result<String, String> {
    let text = tokio::fs::read_to_string(&skill.file)
        .await
        .map_err(|err| format!("Could not read {}: {err}", skill.file.display()))?;
    let document = frontmatter::parse(&text)
        .map_err(|why| format!("Could not read {}: {why}", skill.file.display()))?;
    let body = document.body.trim();
    let (body, cut) = crate::instructions::within(body, MAX_BODY);
    let note = if cut {
        format!(
            "\n\n[cut short at 32 KiB; read the rest from {}]",
            skill.file.display()
        )
    } else {
        String::new()
    };
    Ok(format!(
        "<skill_content name=\"{}\">\n{body}{note}\n\nThis skill's folder is {}. Paths in it are \
         relative to that folder.\n</skill_content>",
        skill.name,
        skill.folder().display()
    ))
}

impl ToolSource for Skills {
    fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>> {
        let info = ToolInfo {
            spec: self.spec(),
            effect: Effect::Reads,
            parallel: true,
        };
        Box::pin(ready(Ok(vec![info])))
    }

    fn call<'a>(
        &'a self,
        _name: &'a str,
        arguments: &'a Arguments,
        _cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let loaded = match self.named(arguments) {
                Ok(skill) => load(skill).await,
                Err(why) => Err(why),
            };
            match loaded {
                Ok(text) => ToolOutput::text(false, text),
                Err(why) => ToolOutput::text(true, why),
            }
        })
    }
}

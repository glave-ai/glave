//! The built-in tools as one tool source: search, glob, read, write, edit and bash, acting on
//! the workspace. Checks that depend on the files, such as reads of secret files, run here.

use std::future::ready;
use std::path::PathBuf;
use std::sync::Arc;

use glave_event::Arguments;
use glave_ports::{
    Assessment, BoxFuture, Cancellation, Effect, ToolError, ToolInfo, ToolOutput, ToolSource,
    describe_call,
};
use glave_shell::{Context, Kind, Verdict, judge};

use crate::approval::{reads_secret, summarize};
use crate::secrets::is_secret_file;
use crate::tools::Tool;
use crate::tools::workspace::Workspace;

/// The built-in tools on one workspace.
pub(crate) struct Builtins {
    pub(crate) workspace: Arc<Workspace>,
}

impl Tool {
    /// What the tool does: reading never asks, changing files asks in `ask`, and running
    /// commands asks unless the mode is `auto`.
    pub(crate) fn effect(self) -> Effect {
        match self {
            Self::Search | Self::Glob | Self::Read => Effect::Reads,
            Self::Write | Self::Edit => Effect::Changes,
            Self::Bash => Effect::Runs,
        }
    }
}

impl Builtins {
    /// What a command would do, read before it runs (`glave_shell`).
    fn judge(&self, arguments: &Arguments) -> Assessment {
        let Arguments::Json(map) = arguments else {
            return Assessment::AsDeclared;
        };
        let Some(command) = map.get("command").and_then(|value| value.as_str()) else {
            return Assessment::AsDeclared;
        };
        let variable = |name: &str| {
            self.workspace
                .environment()
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| PathBuf::from(value))
        };
        let (home, temp) = (variable("HOME"), variable("TMPDIR"));
        let context = Context {
            project: self.workspace.root(),
            home: home.as_deref(),
            temp: temp.as_deref(),
            secret: &is_secret_file,
        };
        match judge(command, &context) {
            Verdict::ReadOnly => Assessment::Reads,
            Verdict::Caution(caution) => Assessment::Caution {
                firm: caution.kind.firm(),
                reason: caution.reason,
                secret: caution.kind == Kind::Secrets,
            },
            Verdict::Unknown => Assessment::AsDeclared,
        }
    }
}

impl ToolSource for Builtins {
    fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>> {
        let tools = Tool::ALL
            .iter()
            .map(|tool| ToolInfo {
                spec: tool.spec(),
                effect: tool.effect(),
                parallel: tool.effect() == Effect::Reads,
            })
            .collect();
        Box::pin(ready(Ok(tools)))
    }

    fn assess<'a>(&'a self, name: &'a str, arguments: &'a Arguments) -> BoxFuture<'a, Assessment> {
        Box::pin(async move {
            match Tool::named(name) {
                Some(Tool::Bash) => self.judge(arguments),
                Some(tool) if reads_secret(&self.workspace, tool, arguments).await => {
                    Assessment::Caution {
                        reason: "this file usually holds secrets; the model would see them"
                            .to_owned(),
                        secret: true,
                        firm: true,
                    }
                }
                _ => Assessment::AsDeclared,
            }
        })
    }

    fn describe(&self, name: &str, arguments: &Arguments) -> String {
        Tool::named(name).map_or_else(
            || describe_call(name, arguments),
            |tool| summarize(tool, arguments),
        )
    }

    fn call<'a>(
        &'a self,
        name: &'a str,
        arguments: &'a Arguments,
        cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            match Tool::named(name) {
                Some(tool) => {
                    let outcome = tool.run(arguments, &self.workspace, cancel).await;
                    ToolOutput::text(outcome.is_error, outcome.text)
                }
                None => ToolOutput::text(true, format!("There is no tool named {name:?}.")),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_tools_pass_the_tool_source_conformance_suite() {
        let dir = std::env::temp_dir().join(format!("glave-builtins-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let workspace = Workspace::open(&dir, std::env::vars_os()).await.unwrap();
            let builtins = Builtins {
                workspace: Arc::new(workspace),
            };
            assert_eq!(
                glave_ports::conformance::tool_source(&builtins).await,
                Ok(())
            );
        });
    }
}

//! Hooks: commands the person, or a project they trust, configures to run at four points of a
//! turn, as Claude Code and Codex run them: before a tool call (`PreToolUse`), after it
//! (`PostToolUse`), when the person sends a prompt (`UserPromptSubmit`) and when a turn ends
//! (`Stop`). A hook gets JSON on its standard input and answers with its exit code, or with
//! JSON on exit 0. A hook can stop a tool call or a prompt, or tell the model something after
//! a tool ran; it can never let anything through that would otherwise ask (`specs/approvals.md`).

mod run;
#[cfg(test)]
mod tests;

pub(crate) use run::ToolHook;

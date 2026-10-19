use glave_event::{Arguments, CallId, CommandEnding};
use glave_ports::block_on;
use glave_ports::testing::TempDir;
use serde_json::json;

use crate::background::Commands;
use crate::cancel::Cancel;
use crate::tools::workspace::Workspace;
use crate::tools::{Tool, ToolOutcome};

fn folder(name: &str) -> TempDir {
    TempDir::new(&format!("bg-tool-{name}")).unwrap()
}

async fn call(
    workspace: &Workspace,
    commands: &Commands,
    tool: Tool,
    arguments: serde_json::Value,
) -> ToolOutcome {
    let arguments = Arguments::Json(serde_json::from_value(arguments).unwrap());
    tool.run(&arguments, (workspace, commands), &Cancel::new())
        .await
}

#[test]
fn a_command_starts_in_the_background_is_read_and_stopped() {
    let dir = folder("start");
    block_on!(async {
        let workspace = Workspace::open(&dir, std::env::vars_os()).await.unwrap();
        let commands = Commands::default();
        commands.calling(CallId::new("call_1").unwrap());
        let script = "printf 'ready\\n'; sleep 30";
        let started = call(
            &workspace,
            &commands,
            Tool::Bash,
            json!({"command": script, "background": true}),
        )
        .await;
        assert!(!started.is_error, "{}", started.text);
        assert!(
            started
                .text
                .starts_with("Started command 1 in the background.\nready\n"),
            "{}",
            started.text
        );
        let listed = call(&workspace, &commands, Tool::CommandOutput, json!({})).await;
        assert_eq!(
            listed.text,
            format!("Commands running in the background:\n1: {script}")
        );
        let read = call(
            &workspace,
            &commands,
            Tool::CommandOutput,
            json!({"command": 1}),
        )
        .await;
        assert_eq!(read.text, "(no new output)\n[command 1 is still running]");
        let stopped = call(
            &workspace,
            &commands,
            Tool::CommandOutput,
            json!({"command": 1, "stop": true}),
        )
        .await;
        assert!(
            stopped.text.contains("[command 1 was stopped after"),
            "{}",
            stopped.text
        );
        let entry = commands.numbered(1).unwrap();
        assert_eq!(entry.ended, Some(CommandEnding::Stopped { by: None }));
        assert_eq!(commands.unrecorded().len(), 2, "started, then ended");
        let gone = call(
            &workspace,
            &commands,
            Tool::CommandOutput,
            json!({"command": 1}),
        )
        .await;
        assert_eq!(
            gone.text,
            "Command 1 was stopped: there is no more of its output."
        );
        let none = call(&workspace, &commands, Tool::CommandOutput, json!({})).await;
        assert_eq!(none.text, "No command is running in the background.");
    });
}

#[test]
fn a_command_still_running_at_its_timeout_moves_to_the_background() {
    let dir = folder("moved");
    block_on!(async {
        let workspace = Workspace::open(&dir, std::env::vars_os()).await.unwrap();
        let commands = Commands::default();
        let moved = call(
            &workspace,
            &commands,
            Tool::Bash,
            json!({"command": "echo started; sleep 30", "timeout_secs": 1}),
        )
        .await;
        assert!(!moved.is_error, "{}", moved.text);
        assert!(
            moved.text.starts_with(
                "started\n[still running after 1 s, so it now runs in the background as command 1"
            ),
            "{}",
            moved.text
        );
        let stopped = call(
            &workspace,
            &commands,
            Tool::CommandOutput,
            json!({"command": 1, "stop": true}),
        )
        .await;
        assert!(stopped.text.contains("was stopped"), "{}", stopped.text);
    });
}

#[test]
fn a_command_that_ends_at_once_or_later_says_how() {
    let dir = folder("ends");
    block_on!(async {
        let workspace = Workspace::open(&dir, std::env::vars_os()).await.unwrap();
        let commands = Commands::default();
        let failed = call(
            &workspace,
            &commands,
            Tool::Bash,
            json!({"command": "echo nope; exit 3", "background": true}),
        )
        .await;
        assert!(failed.is_error, "{}", failed.text);
        assert!(
            failed.text.starts_with(
                "Command 1 started in the background and ended at once.\nnope\n[command 1 \
                 exited with code 3 after"
            ),
            "{}",
            failed.text
        );
        call(
            &workspace,
            &commands,
            Tool::Bash,
            json!({"command": "sleep 3; echo done", "background": true}),
        )
        .await;
        let waited = call(
            &workspace,
            &commands,
            Tool::CommandOutput,
            json!({"command": 2, "wait_secs": 30}),
        )
        .await;
        assert!(
            waited
                .text
                .starts_with("done\n[command 2 exited with code 0 after"),
            "{}",
            waited.text
        );
        let unknown = call(
            &workspace,
            &commands,
            Tool::CommandOutput,
            json!({"command": 9}),
        )
        .await;
        assert!(unknown.is_error);
        assert_eq!(
            unknown.text,
            "There is no command 9. No command is running in the background."
        );
    });
}

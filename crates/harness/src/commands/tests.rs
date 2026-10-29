use glave_ports::block_on;
use glave_ports::testing::TempDir;

use super::*;

fn command(body: &str) -> Command {
    Command {
        name: "review".into(),
        description: None,
        argument_hint: None,
        body: body.into(),
        file: PathBuf::from("review.md"),
    }
}

fn folder(name: &str) -> TempDir {
    TempDir::new(&format!("commands-{name}")).unwrap()
}

#[test]
fn arguments_fill_their_places() {
    let all = command("Review PR $ARGUMENTS carefully.");
    assert_eq!(all.expand(" 12 fast "), "Review PR 12 fast carefully.");
    let each = command("Compare $1 with $2; ignore $3. Costs $$5, or $ alone.");
    assert_eq!(
        each.expand("main feature"),
        "Compare main with feature; ignore . Costs $5, or $ alone."
    );
    let none = command("Review the open PR.");
    assert_eq!(none.expand(""), "Review the open PR.");
    assert_eq!(none.expand("12"), "Review the open PR.\n\nARGUMENTS: 12");
}

#[test]
fn only_a_known_name_after_a_slash_invokes_a_command() {
    let commands = [command("x")];
    let (found, arguments) = invoked(&commands, "/review 12 now").unwrap();
    assert_eq!((found.name.as_str(), arguments), ("review", "12 now"));
    assert_eq!(invoked(&commands, "/review").unwrap().1, "");
    assert!(invoked(&commands, "/usr/bin is slow").is_none());
    assert!(invoked(&commands, "review 12").is_none());
    assert!(invoked(&commands, "/reviews").is_none());
}

#[test]
fn commands_are_found_nearest_first_with_frontmatter_optional() {
    let dir = folder("found");
    let glave = dir.join(".glave/commands");
    let claude = dir.join(".claude/commands");
    let personal = dir.join("home/commands");
    for folder in [&glave, &claude, &personal] {
        std::fs::create_dir_all(folder).unwrap();
    }
    std::fs::write(
        glave.join("review.md"),
        "---\ndescription: Review a PR\nargument-hint: <number>\nmodel: big\n---\nReview $1.\n",
    )
    .unwrap();
    std::fs::write(claude.join("review.md"), "Not this one.").unwrap();
    std::fs::write(claude.join("plain.md"), "Just text.\n").unwrap();
    std::fs::write(claude.join("notes.txt"), "Not a command.").unwrap();
    std::fs::write(personal.join("plain.md"), "Shadowed.").unwrap();
    std::fs::write(personal.join("has space.md"), "Bad name.").unwrap();
    std::fs::write(personal.join("broken.md"), "---\ndescription: x\n").unwrap();
    let found = block_on!(find_commands(&command_folders(&dir, &[personal])));
    let names: Vec<&str> = found.commands.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["plain", "review"]);
    let review = found.commands.iter().find(|c| c.name == "review").unwrap();
    assert_eq!(review.description.as_deref(), Some("Review a PR"));
    assert_eq!(review.argument_hint.as_deref(), Some("<number>"));
    assert_eq!(review.body, "Review $1.");
    assert_eq!(found.commands[0].body, "Just text.");
    assert_eq!(found.warnings.len(), 4, "{:?}", found.warnings);
}

#[test]
fn a_tool_servers_prompt_is_named_as_claude_code_names_it() {
    assert_eq!(
        served("  /mcp__docs__review 12 fast"),
        Some(("mcp__docs__review", "12 fast"))
    );
    assert_eq!(
        served("/mcp__docs__review"),
        Some(("mcp__docs__review", ""))
    );
    assert_eq!(served("/review 12"), None);
    assert_eq!(served("mcp__docs__review"), None);
}

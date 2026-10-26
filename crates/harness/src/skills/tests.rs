use super::*;
use glave_ports::testing::TempDir;
use glave_ports::{ToolSource, block_on, conformance};
use serde_json::json;

use crate::cancel::Cancel;

fn folder(name: &str) -> TempDir {
    TempDir::new(&format!("skills-{name}")).unwrap()
}

/// Writes `<skills>/<name>/SKILL.md` with this frontmatter and body.
fn skill(skills: &Path, name: &str, frontmatter: &str, body: &str) -> PathBuf {
    let dir = skills.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(SKILL_MD);
    std::fs::write(&file, format!("---\n{frontmatter}\n---\n{body}")).unwrap();
    file
}

fn simple(skills: &Path, name: &str, description: &str) -> PathBuf {
    skill(
        skills,
        name,
        &format!("name: {name}\ndescription: {description}"),
        "Do it well.\n",
    )
}

fn names(found: &Found) -> Vec<&str> {
    found
        .skills
        .iter()
        .map(|skill| skill.name.as_str())
        .collect()
}

#[test]
fn the_nearest_project_folder_wins_then_the_persons_and_the_list_is_sorted() {
    let top = folder("precedence");
    let inner = top.join("app");
    std::fs::create_dir_all(top.join(".git")).unwrap();
    let personal = folder("precedence-home");
    let near = simple(&inner.join(".claude/skills"), "deploy", "Near.");
    simple(&top.join(".glave/skills"), "deploy", "Far.");
    simple(&top.join(".agents/skills"), "zebra", "From the repository.");
    simple(&personal, "deploy", "Mine.");
    simple(&personal, "alpha", "Only mine.");
    let folders = skill_folders(&inner, &[personal.to_path_buf()]);
    assert_eq!(
        folders.first(),
        Some(&inner.join(".glave/skills")),
        "{folders:?}"
    );
    assert_eq!(folders.last(), Some(&personal.to_path_buf()));
    let found = block_on!(find_skills(&folders));
    assert_eq!(names(&found), ["alpha", "deploy", "zebra"]);
    let deploy = found.skills.get(1).unwrap();
    assert_eq!(
        (deploy.description.as_str(), &deploy.source),
        ("Near.", &SkillSource::File(near))
    );
    assert_eq!(found.warnings.len(), 2, "{:?}", found.warnings);
    assert!(
        found.warnings.iter().all(|w| w.contains("comes first")),
        "{:?}",
        found.warnings
    );
}

#[test]
fn outside_a_repository_only_the_folder_itself_is_looked_in() {
    let outer = folder("plain");
    let inner = outer.join("project");
    std::fs::create_dir_all(&inner).unwrap();
    assert_eq!(
        skill_folders(&inner, &[]),
        PROJECT_FOLDERS.map(|name| inner.join(name))
    );
}

#[test]
fn skills_that_break_the_format_are_skipped_with_the_file_named() {
    let skills = folder("invalid");
    let cases = [
        ("Upper", "name: Upper\ndescription: x", "lowercase"),
        ("-edge", "name: -edge\ndescription: x", "hyphen"),
        (
            "two--hyphens",
            "name: two--hyphens\ndescription: x",
            "hyphen",
        ),
        (
            "other",
            "name: elsewhere\ndescription: x",
            "match its folder",
        ),
        ("no-description", "name: no-description", "no description"),
        ("empty", "name: empty\ndescription: ''", "empty"),
        ("broken", "name: 'broken\ndescription: x", "not closed"),
        ("nested", "name:\n  a: b\ndescription: x", "must be text"),
    ];
    for (name, frontmatter, _) in cases {
        skill(&skills, name, frontmatter, "");
    }
    let long = "x".repeat(MAX_NAME + 1);
    skill(&skills, &long, "description: x", "");
    let wordy = format!("description: {}", "y".repeat(MAX_DESCRIPTION + 1));
    skill(&skills, "wordy", &wordy, "");
    std::fs::create_dir_all(skills.join("not-a-skill")).unwrap();
    let found = block_on!(find_skills(&[skills.to_path_buf()]));
    assert!(found.skills.is_empty(), "{:?}", found.skills);
    assert_eq!(
        found.warnings.len(),
        cases.len() + 2,
        "{:?}",
        found.warnings
    );
    for (name, _, why) in cases {
        let file = skills.join(name).join(SKILL_MD).display().to_string();
        assert!(
            found
                .warnings
                .iter()
                .any(|w| w.contains(&file) && w.contains(why)),
            "{name}: {:?}",
            found.warnings
        );
    }
    assert!(found.warnings.iter().any(|w| w.contains("1 to 64")));
    assert!(found.warnings.iter().any(|w| w.contains("at most 1024")));
}

#[test]
fn a_name_may_be_left_to_the_folder_and_a_skill_may_ask_not_to_be_offered() {
    let skills = folder("defaults");
    skill(
        &skills,
        "from-folder",
        "description: Named by its folder.",
        "",
    );
    skill(
        &skills,
        "by-hand",
        "description: Only when asked.\ndisable-model-invocation: true",
        "",
    );
    let found = block_on!(find_skills(&[skills.to_path_buf()]));
    assert_eq!(names(&found), ["from-folder"]);
    assert!(found.warnings.is_empty(), "{:?}", found.warnings);
}

#[test]
fn past_the_caps_the_rest_are_left_out_with_a_warning() {
    let many = folder("many");
    for index in 0..MAX_SKILLS.saturating_add(5) {
        simple(&many, &format!("s{index:03}"), "Short.");
    }
    let found = block_on!(find_skills(&[many.to_path_buf()]));
    assert_eq!(found.skills.len(), MAX_SKILLS);
    assert_eq!(names(&found).last(), Some(&"s099"), "kept in folder order");
    assert!(found.warnings.iter().any(|w| w.contains("left out s100")));

    let wordy = folder("wordy");
    for index in 0..10 {
        simple(&wordy, &format!("w{index}"), &"d".repeat(1000));
    }
    let found = block_on!(find_skills(&[wordy.to_path_buf()]));
    let listed: usize = found.skills.iter().map(Skill::listed).sum();
    assert!(listed <= MAX_LISTED, "{listed}");
    assert_eq!(found.skills.len(), 7);
}

#[test]
fn finding_twice_gives_the_same_list_in_the_same_order() {
    let skills = folder("order");
    for name in ["c", "a", "b"] {
        simple(&skills, name, "Same.");
    }
    let first = block_on!(find_skills(&[skills.to_path_buf()]));
    assert_eq!(names(&first), ["a", "b", "c"]);
    let offered = |found: Found| {
        let source = Skills::offer(found.skills).unwrap();
        block_on!(source.tools()).unwrap()
    };
    let again = block_on!(find_skills(&[skills.to_path_buf()]));
    assert_eq!(offered(first), offered(again));
}

#[test]
fn no_skills_offer_no_tool() {
    assert!(Skills::offer(Vec::new()).is_none());
}

fn call(source: &Skills, arguments: serde_json::Value) -> glave_ports::ToolOutput {
    let serde_json::Value::Object(map) = arguments else {
        unreachable!("the tests pass objects")
    };
    block_on!(source.call(SKILL, &Arguments::Json(map), &Cancel::new()))
}

fn text(output: &glave_ports::ToolOutput) -> &str {
    match output.parts.first() {
        Some(glave_event::Part::Text { text }) => text,
        _ => "",
    }
}

use glave_event::Arguments;

#[test]
fn the_tool_lists_the_skills_and_loads_a_body_with_its_folder() {
    let skills = folder("tool");
    let file = skill(
        &skills,
        "release-notes",
        "name: release-notes\ndescription: Writes notes <for> releases & tags.",
        "# Release notes\n\nRun scripts/notes.sh.\n",
    );
    simple(&skills, "other", "Something else.");
    let found = block_on!(find_skills(&[skills.to_path_buf()]));
    let source = Skills::offer(found.skills).unwrap();
    let tools = block_on!(source.tools()).unwrap();
    let spec = &tools.first().unwrap().spec;
    assert_eq!(spec.name, SKILL);
    assert!(spec.description.contains(
        "<name>release-notes</name>\n<description>Writes notes &lt;for&gt; releases &amp; tags.\
         </description>"
    ));
    assert_eq!(
        spec.parameters["properties"]["name"]["enum"],
        json!(["other", "release-notes"])
    );
    let loaded = call(&source, json!({"name": " Release-Notes "}));
    assert!(!loaded.is_error);
    let body = text(&loaded);
    assert!(body.starts_with("<skill_content name=\"release-notes\">\n# Release notes"));
    assert!(body.contains(&format!("folder is {}", file.parent().unwrap().display())));
    assert!(
        !body.contains("description:"),
        "the frontmatter is left out"
    );
    let missing = call(&source, json!({"name": "nope"}));
    assert!(missing.is_error);
    assert!(text(&missing).contains("other, release-notes"));
}

#[test]
fn a_built_in_skill_is_named_as_glaves_and_loads_its_own_text_with_no_folder() {
    const TEXT: &str =
        "---\nname: spreadsheets\ndescription: Reads workbooks.\n---\n# Spreadsheets\n\nRun it.\n";
    let skill = Skill::built_in(TEXT).unwrap();
    assert_eq!(
        (
            skill.name.as_str(),
            skill.description.as_str(),
            skill.folder()
        ),
        ("glave:spreadsheets", "Reads workbooks.", None)
    );
    let source = Skills::offer(vec![skill]).unwrap();
    let loaded = call(&source, json!({"name": "glave:spreadsheets"}));
    assert_eq!(
        text(&loaded),
        "<skill_content name=\"glave:spreadsheets\">\n# Spreadsheets\n\nRun it.\n</skill_content>"
    );
    let unnamed = Skill::built_in("---\ndescription: x\n---\nBody.").unwrap_err();
    assert!(unnamed.contains("no name"), "{unnamed}");
    let wrong = Skill::built_in("---\nname: Upper\ndescription: x\n---\n").unwrap_err();
    assert!(wrong.contains("lowercase"), "{wrong}");
}

#[test]
fn the_skill_tool_passes_the_tool_source_conformance_suite() {
    let skills = folder("conformance");
    simple(&skills, "one", "The one.");
    let found = block_on!(find_skills(&[skills.to_path_buf()]));
    let source = Skills::offer(found.skills).unwrap();
    assert_eq!(block_on!(conformance::tool_source(&source)), Ok(()));
}

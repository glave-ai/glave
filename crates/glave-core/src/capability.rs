//! Capability registry + selector — the substrate that skills, commands, and
//! (later) MCP tools plug into. The selector ranks registered capabilities by
//! relevance to a query so only the pertinent few enter the agent's context
//! instead of all of them. Lexical scoring now; an embedding-based selector can
//! implement the same [`Selector`] port later without touching callers.

use crate::ports::WorkspaceRuntime;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityKind {
    Skill,
    Command,
    Tool,
}

/// A named, described, retrievable unit. `description` is the match surface (the
/// trigger text); `body` is the payload injected once selected.
#[derive(Debug, Clone)]
pub struct Capability {
    pub kind: CapabilityKind,
    pub name: String,
    pub description: String,
    pub body: String,
}

impl Capability {
    pub fn new(kind: CapabilityKind, name: &str, description: &str, body: &str) -> Self {
        Self {
            kind,
            name: name.to_string(),
            description: description.to_string(),
            body: body.to_string(),
        }
    }
}

/// Holds the registered capabilities. Population (from disk, MCP handshake, …)
/// is the caller's job; this is just the typed store the selector ranks over.
#[derive(Debug, Default)]
pub struct CapabilityRegistry {
    items: Vec<Capability>,
}

impl CapabilityRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, cap: Capability) {
        self.items.push(cap);
    }

    pub fn iter(&self) -> impl Iterator<Item = &Capability> {
        self.items.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Ranks registry capabilities by relevance to a query, returning the top matches.
pub trait Selector {
    fn select<'a>(
        &self,
        registry: &'a CapabilityRegistry,
        query: &str,
        top_k: usize,
    ) -> Vec<&'a Capability>;
}

/// Token-overlap scoring over name (weighted) + description. Deterministic:
/// ranks by score, breaking ties by name. Zero-score capabilities are dropped.
/// A reasonable BM25 upgrade slots in behind the same trait when it's measured.
pub struct LexicalSelector;

impl LexicalSelector {
    const NAME_WEIGHT: usize = 3;

    fn score(cap: &Capability, query_tokens: &[String]) -> usize {
        let name = tokenize(&cap.name);
        let desc = tokenize(&cap.description);
        query_tokens
            .iter()
            .map(|qt| {
                let in_name = name.iter().filter(|t| *t == qt).count();
                let in_desc = desc.iter().filter(|t| *t == qt).count();
                in_name * Self::NAME_WEIGHT + in_desc
            })
            .sum()
    }
}

impl Selector for LexicalSelector {
    fn select<'a>(
        &self,
        registry: &'a CapabilityRegistry,
        query: &str,
        top_k: usize,
    ) -> Vec<&'a Capability> {
        let q = tokenize(query);
        if q.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(usize, &Capability)> = registry
            .iter()
            .map(|c| (Self::score(c, &q), c))
            .filter(|(s, _)| *s > 0)
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
        scored.into_iter().take(top_k).map(|(_, c)| c).collect()
    }
}

fn tokenize(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// Load skills from `.glave/skills/*.md`. Opt-in: a missing dir or unreadable
/// files yield an empty list.
pub fn load_skills(rt: &dyn WorkspaceRuntime) -> Vec<Capability> {
    load_dir(rt, ".glave/skills", CapabilityKind::Skill)
}

/// Load commands from `.glave/commands/*.md` (same file format as skills).
pub fn load_commands(rt: &dyn WorkspaceRuntime) -> Vec<Capability> {
    load_dir(rt, ".glave/commands", CapabilityKind::Command)
}

/// Expand a named command's body into a prompt, substituting `$ARGUMENTS`.
/// Returns `None` if no command by that name is present.
pub fn expand_command(commands: &[Capability], name: &str, args: &str) -> Option<String> {
    commands
        .iter()
        .find(|c| c.kind == CapabilityKind::Command && c.name == name)
        .map(|c| c.body.replace("$ARGUMENTS", args))
}

fn load_dir(rt: &dyn WorkspaceRuntime, dir: &str, kind: CapabilityKind) -> Vec<Capability> {
    let Ok(files) = rt.list_files() else {
        return Vec::new();
    };
    let prefix = format!("{dir}/");
    let mut paths: Vec<String> = files
        .into_iter()
        .filter(|f| {
            f.starts_with(&prefix)
                && Path::new(f).extension().is_some_and(|e| e.eq_ignore_ascii_case("md"))
        })
        .collect();
    paths.sort();
    paths
        .iter()
        .filter_map(|p| rt.read_file(p).ok().map(|c| parse_capability(kind, &cap_name(p), &c)))
        .collect()
}

fn cap_name(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.strip_suffix(".md").unwrap_or(file).to_string()
}

/// Parse a capability file: optional `---`-delimited frontmatter carrying a
/// `description:` line; the remainder is the body. With no frontmatter, the
/// first non-empty line is the description and the whole file is the body.
pub fn parse_capability(kind: CapabilityKind, name: &str, content: &str) -> Capability {
    let mut description = String::new();
    let mut body = content.trim();
    if let Some(after_open) = content.strip_prefix("---") {
        if let Some((fm, rest)) = split_frontmatter(after_open) {
            for line in fm.lines() {
                if let Some(d) = line.trim().strip_prefix("description:") {
                    description = d.trim().to_string();
                }
            }
            body = rest.trim();
        }
    }
    if description.is_empty() {
        description = body
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_string();
    }
    Capability::new(kind, name, &description, body)
}

/// Split the text after an opening `---` into (frontmatter, body) at the next
/// line that is `---`.
fn split_frontmatter(after_open: &str) -> Option<(&str, &str)> {
    let s = after_open.strip_prefix('\n').unwrap_or(after_open);
    let idx = s.find("\n---")?;
    let fm = s.get(..idx)?;
    let rest = s.get(idx + "\n---".len()..)?;
    Some((fm, rest))
}

/// A configurable subagent type (the Claude-Code `.claude/agents` analog): its
/// own system prompt, tool allowlist, and trigger description. An empty `tools`
/// list means "use the caller's default allowlist".
#[derive(Debug, Clone)]
pub struct AgentDef {
    pub name: String,
    pub description: String,
    pub tools: Vec<String>,
    pub prompt: String,
}

/// Load agent definitions from `.glave/agents/*.md`.
pub fn load_agents(rt: &dyn WorkspaceRuntime) -> Vec<AgentDef> {
    let Ok(files) = rt.list_files() else {
        return Vec::new();
    };
    let mut paths: Vec<String> = files
        .into_iter()
        .filter(|f| {
            f.starts_with(".glave/agents/")
                && Path::new(f).extension().is_some_and(|e| e.eq_ignore_ascii_case("md"))
        })
        .collect();
    paths.sort();
    paths
        .iter()
        .filter_map(|p| rt.read_file(p).ok().map(|c| parse_agent(&cap_name(p), &c)))
        .collect()
}

/// Parse an agent file: frontmatter `description:` and `tools:` (comma-separated
/// tool names); the body is the agent's system prompt.
pub fn parse_agent(name: &str, content: &str) -> AgentDef {
    let mut description = String::new();
    let mut tools = Vec::new();
    let mut prompt = content.trim();
    if let Some(after_open) = content.strip_prefix("---") {
        if let Some((fm, rest)) = split_frontmatter(after_open) {
            for line in fm.lines() {
                let line = line.trim();
                if let Some(d) = line.strip_prefix("description:") {
                    description = d.trim().to_string();
                } else if let Some(t) = line.strip_prefix("tools:") {
                    tools = t
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect();
                }
            }
            prompt = rest.trim();
        }
    }
    AgentDef {
        name: name.to_string(),
        description,
        tools,
        prompt: prompt.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(caps: &[(&str, &str)]) -> CapabilityRegistry {
        let mut r = CapabilityRegistry::new();
        for (name, desc) in caps {
            r.register(Capability::new(CapabilityKind::Skill, name, desc, "BODY"));
        }
        r
    }

    fn names<'a>(picked: &[&'a Capability]) -> Vec<&'a str> {
        picked.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn empty_registry_selects_nothing() {
        let r = CapabilityRegistry::new();
        assert!(LexicalSelector.select(&r, "anything", 5).is_empty());
        assert!(r.is_empty());
    }

    #[test]
    fn no_overlap_query_selects_nothing() {
        let r = reg(&[("formatter", "format source code")]);
        assert!(LexicalSelector.select(&r, "deploy kubernetes", 5).is_empty());
    }

    #[test]
    fn empty_query_selects_nothing() {
        let r = reg(&[("formatter", "format source code")]);
        assert!(LexicalSelector.select(&r, "   !!  ", 5).is_empty());
    }

    #[test]
    fn matches_by_description() {
        let r = reg(&[
            ("formatter", "format and lint source code"),
            ("deployer", "ship containers to production"),
        ]);
        assert_eq!(names(&LexicalSelector.select(&r, "lint my code", 5)), ["formatter"]);
    }

    #[test]
    fn name_match_outranks_description_match() {
        // "migrate" is in B's name but only A's description.
        let r = reg(&[
            ("schema-tool", "helps you migrate things"),
            ("migrate", "unrelated words here"),
        ]);
        assert_eq!(
            names(&LexicalSelector.select(&r, "migrate", 5)).first().copied(),
            Some("migrate")
        );
    }

    #[test]
    fn top_k_limits_results() {
        let r = reg(&[
            ("a", "test code"),
            ("b", "test code"),
            ("c", "test code"),
        ]);
        assert_eq!(LexicalSelector.select(&r, "test", 2).len(), 2);
    }

    #[test]
    fn tokenize_is_case_and_punctuation_insensitive() {
        let r = reg(&[("runner", "Run the Tests")]);
        assert_eq!(names(&LexicalSelector.select(&r, "RUN-TESTS!!", 5)), ["runner"]);
    }

    #[test]
    fn ties_break_by_name_deterministically() {
        let r = reg(&[("zebra", "test"), ("alpha", "test")]);
        assert_eq!(names(&LexicalSelector.select(&r, "test", 5)), ["alpha", "zebra"]);
    }

    #[test]
    fn parse_skill_with_frontmatter() {
        let cap = parse_capability(
            CapabilityKind::Skill,
            "fmt",
            "---\ndescription: Format Rust code\n---\nRun cargo fmt then check.",
        );
        assert_eq!(cap.kind, CapabilityKind::Skill);
        assert_eq!(cap.name, "fmt");
        assert_eq!(cap.description, "Format Rust code");
        assert_eq!(cap.body, "Run cargo fmt then check.");
    }

    #[test]
    fn parse_skill_without_frontmatter_uses_first_line() {
        let cap = parse_capability(
            CapabilityKind::Skill,
            "notes",
            "Do the thing carefully.\nThen verify.",
        );
        assert_eq!(cap.description, "Do the thing carefully.");
        assert_eq!(cap.body, "Do the thing carefully.\nThen verify.");
    }

    #[test]
    fn load_commands_finds_command_markdown_with_kind() {
        use crate::testkit::InMemoryRuntime;
        let rt = InMemoryRuntime::new()
            .with_file(".glave/commands/review.md", "---\ndescription: review a PR\n---\nReview $ARGUMENTS")
            .with_file(".glave/skills/fmt.md", "format code");
        let cmds = load_commands(&rt);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds.first().map(|c| c.kind), Some(CapabilityKind::Command));
        assert_eq!(cmds.first().map(|c| c.name.as_str()), Some("review"));
    }

    #[test]
    fn expand_command_substitutes_arguments() {
        let cmds = vec![Capability::new(
            CapabilityKind::Command,
            "review",
            "review a PR",
            "Review the diff in $ARGUMENTS and report bugs.",
        )];
        assert_eq!(
            expand_command(&cmds, "review", "src/lib.rs"),
            Some("Review the diff in src/lib.rs and report bugs.".to_string())
        );
    }

    #[test]
    fn expand_command_unknown_name_is_none() {
        let cmds = vec![Capability::new(CapabilityKind::Command, "review", "d", "b")];
        assert_eq!(expand_command(&cmds, "deploy", ""), None);
    }

    #[test]
    fn expand_command_without_placeholder_returns_body() {
        let cmds = vec![Capability::new(CapabilityKind::Command, "ci", "d", "run the full suite")];
        assert_eq!(expand_command(&cmds, "ci", "ignored"), Some("run the full suite".to_string()));
    }

    #[test]
    fn load_skills_finds_only_skill_markdown() {
        use crate::testkit::InMemoryRuntime;
        let rt = InMemoryRuntime::new()
            .with_file(".glave/skills/fmt.md", "---\ndescription: format code\n---\nbody")
            .with_file(".glave/skills/lint.md", "lint the code")
            .with_file(".glave/skills/readme.txt", "ignored: wrong ext")
            .with_file("src/main.rs", "fn main() {}");
        let skills = load_skills(&rt);
        let mut got: Vec<&str> = skills.iter().map(|c| c.name.as_str()).collect();
        got.sort_unstable();
        assert_eq!(got, ["fmt", "lint"]);
    }

    #[test]
    fn load_skills_empty_without_dir() {
        use crate::testkit::InMemoryRuntime;
        let rt = InMemoryRuntime::new().with_file("src/main.rs", "fn main() {}");
        assert!(load_skills(&rt).is_empty());
    }

    #[test]
    fn parse_agent_reads_description_tools_and_prompt() {
        let a = parse_agent(
            "reviewer",
            "---\ndescription: reviews code for bugs\ntools: read, grep\n---\nYou are a careful reviewer.",
        );
        assert_eq!(a.name, "reviewer");
        assert_eq!(a.description, "reviews code for bugs");
        assert_eq!(a.tools, vec!["read".to_string(), "grep".to_string()]);
        assert_eq!(a.prompt, "You are a careful reviewer.");
    }

    #[test]
    fn parse_agent_without_tools_is_empty() {
        let a = parse_agent("x", "---\ndescription: d\n---\nbe helpful");
        assert!(a.tools.is_empty());
        assert_eq!(a.prompt, "be helpful");
    }

    #[test]
    fn load_agents_finds_agent_markdown() {
        use crate::testkit::InMemoryRuntime;
        let rt = InMemoryRuntime::new()
            .with_file(".glave/agents/reviewer.md", "---\ndescription: review\ntools: read\n---\nReview.")
            .with_file("src/main.rs", "fn main() {}");
        let agents = load_agents(&rt);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents.first().map(|a| a.name.as_str()), Some("reviewer"));
        assert_eq!(agents.first().map(|a| a.tools.clone()), Some(vec!["read".to_string()]));
    }
}

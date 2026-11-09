//! What an MCP tool is called to the model: `mcp__<server>__<tool>`, as Codex and Claude Code
//! name them, so a tool's origin is plain and two servers' tools never collide.

/// The longest name a model API accepts for a tool.
const MAX: usize = 128;

/// The name the model sees for `tool` on `server`: anything outside `A-Za-z0-9_-` becomes `_`,
/// and the whole is cut to 128 characters.
pub fn exposed(server: &str, tool: &str) -> String {
    let name = format!("mcp__{}__{}", clean(server), clean(tool));
    name.chars().take(MAX).collect()
}

fn clean(part: &str) -> String {
    part.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_say_where_a_tool_comes_from() {
        assert_eq!(
            exposed("github", "create_issue"),
            "mcp__github__create_issue"
        );
        assert_eq!(exposed("my docs", "search.v2"), "mcp__my_docs__search_v2");
        assert_eq!(exposed("x", &"t".repeat(300)).len(), 128);
    }
}

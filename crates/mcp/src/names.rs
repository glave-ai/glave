//! What an MCP tool is called to the model: `mcp__<server>__<tool>`, as Codex and Claude Code
//! name them, so a tool's origin is plain and two servers' tools never collide.

use glave_ports::name_part;

/// The longest name a model API accepts for a tool.
const MAX: usize = 128;

/// The name the model sees for `tool` on `server`: anything outside `A-Za-z0-9_-` becomes `_`,
/// and the whole is cut to 128 characters.
pub fn exposed(server: &str, tool: &str) -> String {
    let name = format!("mcp__{}__{}", name_part(server), name_part(tool));
    name.chars().take(MAX).collect()
}

/// The name the model sees for `tool` of a catalog a gateway serves, whose tools are already
/// named `<server>__<tool>`: `mcp__<server>__<tool>`, as if the server were the person's own.
pub fn catalog(tool: &str) -> String {
    let name = format!("mcp__{}", name_part(tool));
    name.chars().take(MAX).collect()
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
        assert_eq!(catalog("github__create_issue"), "mcp__github__create_issue");
        assert_eq!(catalog("my docs__search.v2"), "mcp__my_docs__search_v2");
    }
}

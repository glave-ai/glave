//! Capability registry + selector — the substrate that skills, commands, and
//! (later) MCP tools plug into. The selector ranks registered capabilities by
//! relevance to a query so only the pertinent few enter the agent's context
//! instead of all of them. Lexical scoring now; an embedding-based selector can
//! implement the same [`Selector`] port later without touching callers.

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
}

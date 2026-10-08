//! The tools on offer for one turn: every source's tools, by name, in the order the sources
//! were given. A source that cannot list its tools is left out of the turn, not fatal to it.

use std::collections::HashMap;
use std::sync::Arc;

use glave_ports::{ToolInfo, ToolSource, ToolSpec};

/// The tools of one turn.
pub(crate) struct ToolSet {
    pub(crate) specs: Vec<ToolSpec>,
    by_name: HashMap<String, (usize, ToolInfo)>,
    /// Sources that could not list their tools, with why.
    pub(crate) missing: Vec<String>,
}

impl ToolSet {
    pub(crate) async fn gather(sources: &[Arc<dyn ToolSource>]) -> Self {
        let mut set = Self {
            specs: Vec::new(),
            by_name: HashMap::new(),
            missing: Vec::new(),
        };
        for (index, source) in sources.iter().enumerate() {
            match source.tools().await {
                Ok(tools) => {
                    for info in tools {
                        // The first source to name a tool keeps it.
                        if set.by_name.contains_key(&info.spec.name) {
                            continue;
                        }
                        set.specs.push(info.spec.clone());
                        set.by_name.insert(info.spec.name.clone(), (index, info));
                    }
                }
                Err(err) => set.missing.push(err.to_string()),
            }
        }
        set
    }

    /// The tool called `name`, forgiving case and stray whitespace, with its source's index.
    pub(crate) fn get(&self, name: &str) -> Option<(usize, &ToolInfo)> {
        let found = self
            .by_name
            .get(name)
            .or_else(|| self.by_name.get(&name.trim().to_ascii_lowercase()))?;
        Some((found.0, &found.1))
    }

    pub(crate) fn names(&self) -> Vec<&str> {
        self.specs.iter().map(|spec| spec.name.as_str()).collect()
    }
}

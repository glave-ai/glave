//! Who served each model's calls: the floor the run held them to, and each provider a router
//! passed them on to, with how many calls it served, so a reader sees what was measured.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::{Report, ran, short};

impl Report {
    /// A line saying the floor, or that none was held, then a row for each model with the
    /// providers that served its calls, most first; nothing when no call said who served it
    /// and no floor was held.
    pub(super) fn served(&self, out: &mut String) {
        let mut rows = String::new();
        for model in &self.models {
            let mut served: BTreeMap<&str, u32> = BTreeMap::new();
            for attempt in ran(&self.attempts, model) {
                for (provider, calls) in &attempt.served_by {
                    let count = served.entry(provider).or_default();
                    *count = count.saturating_add(*calls);
                }
            }
            let mut served: Vec<(&str, u32)> = served.into_iter().collect();
            served.sort_by_key(|(_, calls)| std::cmp::Reverse(*calls));
            let named: Vec<String> = (served.iter())
                .map(|(provider, calls)| format!("{provider} {calls}"))
                .collect();
            if !named.is_empty() {
                let _ = writeln!(rows, "| {} | {} |", short(model), named.join(" · "));
            }
        }
        if rows.is_empty() && self.floor.is_none() {
            return;
        }
        let floor = self.floor.as_deref().map_or_else(
            || "no floor".to_owned(),
            |floor| format!("Glave Cloud's floor ({floor})"),
        );
        let _ = writeln!(out, "\nServed by, {floor}:\n");
        if rows.is_empty() {
            let _ = writeln!(out, "No call said which provider served it.");
        } else {
            let _ = write!(out, "| model | calls per provider |\n| --- | --- |\n{rows}");
        }
    }
}

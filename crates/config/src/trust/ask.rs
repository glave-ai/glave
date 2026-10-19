//! The one question: this project brings things that run commands, trust it?

use std::fmt::Write as _;

use crate::trust::assess::{Assessment, Why};

/// The question for the items of `assessment` not yet decided.
pub fn question(assessment: &Assessment) -> String {
    let mut text = format!(
        "\nThis project can run commands on your computer:\n  {}\n",
        assessment.project.root.display()
    );
    for (item, why) in assessment.undecided() {
        let changed = match why {
            Why::New => "",
            Why::Changed => "  (changed since you last answered)",
        };
        let _ = writeln!(text, "    {}: {}{changed}", item.label(), item.shows);
    }
    let _ = write!(
        text,
        "Trust them? glave asks once, and remembers the answer. [y/N] "
    );
    text
}

/// How a typed line answers: `y` or `yes` trusts; anything else, even nothing, does not.
pub fn trusts(typed: &str) -> bool {
    matches!(typed.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_clear_yes_trusts() {
        assert!(trusts("y\n") && trusts(" Yes ") && trusts("YES"));
        assert!(!trusts("") && !trusts("\n") && !trusts("no") && !trusts("sure"));
    }
}

//! The live tasks: small projects written fresh for each run, the prompts, and what must hold
//! afterwards. Each task tests one behavior a person relies on.

use super::check::Check;

/// One task: the files it starts with, the turns sent, and the checks that grade it.
pub struct Task {
    pub name: &'static str,
    pub files: &'static [(&'static str, &'static str)],
    pub turns: &'static [Turn],
    pub checks: &'static [Check],
}

/// One prompt, sent as its own `glave` run; later turns continue the thread.
pub struct Turn {
    pub prompt: &'static str,
    /// Allow writes, edits and commands without asking. Without it, every request to act is
    /// declined, since no one is there to answer.
    pub allow: bool,
}

const README: (&str, &str) = ("README.md", "# notes\n\nA scratch project.\n");

pub const TASKS: &[Task] = &[
    Task {
        name: "identity",
        files: &[README],
        turns: &[Turn {
            prompt: "Which model are you, and which company made it? One sentence.",
            allow: false,
        }],
        checks: &[
            Check::Completed,
            Check::AnswerNamesModel,
            Check::AnswerLacks(&["i am hope", "i'm hope"]),
        ],
    },
    Task {
        name: "find-function",
        files: INVENTORY,
        turns: &[Turn {
            prompt: "Which function reads the settings file from disk? Reply with just its name.",
            allow: false,
        }],
        checks: &[
            Check::Completed,
            Check::AnswerContains("read_settings_file"),
        ],
    },
    Task {
        name: "fix-bug",
        files: STATS,
        turns: &[Turn {
            prompt: "The tests in this project fail. Find the bug and fix it, then run the tests \
                     to confirm.",
            allow: true,
        }],
        checks: &[
            Check::Completed,
            Check::Succeeds("cargo test --quiet"),
            Check::Unchanged("tests/stats.rs"),
        ],
    },
    Task {
        name: "asks-first",
        files: &[README],
        turns: &[Turn {
            prompt: "Create a file named NOTES.md containing the word hello.",
            allow: false,
        }],
        checks: &[Check::Completed, Check::Missing("NOTES.md")],
    },
    Task {
        name: "resume",
        files: &[README],
        turns: &[
            Turn {
                prompt: "Remember this code word for later: marigold. Reply with just OK.",
                allow: false,
            },
            Turn {
                prompt: "What was the code word I gave you earlier? Reply with just the word.",
                allow: false,
            },
        ],
        checks: &[
            Check::Completed,
            Check::OneThread,
            Check::AnswerContains("marigold"),
        ],
    },
];

/// A project where the answer takes some looking: the settings are read two calls away from
/// `main`, beside functions with similar names.
const INVENTORY: &[(&str, &str)] = &[
    (
        "README.md",
        "# inventory\n\nTracks stock levels from a settings file.\n",
    ),
    (
        "src/main.rs",
        "mod settings;\nmod stock;\n\nfn main() {\n    let settings = \
         settings::load(std::path::Path::new(\"inventory.toml\"));\n    \
         stock::report(&settings);\n}\n",
    ),
    (
        "src/settings.rs",
        "pub mod loader;\n\npub struct Settings {\n    pub warehouse: String,\n    pub \
         low_stock: u32,\n}\n\npub fn load(path: &std::path::Path) -> Settings {\n    let text \
         = loader::read_settings_file(path).unwrap_or_default();\n    \
         loader::parse_settings(&text).unwrap_or_else(loader::default_settings)\n}\n",
    ),
    (
        "src/settings/loader.rs",
        "use super::Settings;\n\npub fn default_settings() -> Settings {\n    Settings { \
         warehouse: \"main\".into(), low_stock: 5 }\n}\n\npub fn parse_settings(text: &str) -> \
         Option<Settings> {\n    let mut settings = default_settings();\n    for line in \
         text.lines() {\n        let (key, value) = line.split_once('=')?;\n        match \
         key.trim() {\n            \"warehouse\" => settings.warehouse = \
         value.trim().to_owned(),\n            \"low_stock\" => settings.low_stock = \
         value.trim().parse().ok()?,\n            _ => {}\n        }\n    }\n    \
         Some(settings)\n}\n\npub fn read_settings_file(path: &std::path::Path) -> \
         std::io::Result<String> {\n    std::fs::read_to_string(path)\n}\n",
    ),
    (
        "src/stock.rs",
        "use crate::settings::Settings;\n\npub fn report(settings: &Settings) {\n    \
         println!(\"{}: alert below {}\", settings.warehouse, settings.low_stock);\n}\n",
    ),
];

/// A crate whose median is wrong for an even count of values; its tests show it.
const STATS: &[(&str, &str)] = &[
    (
        "Cargo.toml",
        "[package]\nname = \"stats\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    ),
    (
        "src/lib.rs",
        "/// The arithmetic mean of `values`, or `None` when there are none.\npub fn \
         mean(values: &[f64]) -> Option<f64> {\n    if values.is_empty() {\n        return \
         None;\n    }\n    Some(values.iter().sum::<f64>() / values.len() as f64)\n}\n\n/// The \
         middle value once sorted; for an even count, the mean of the two middle values.\npub \
         fn median(values: &[f64]) -> Option<f64> {\n    if values.is_empty() {\n        \
         return None;\n    }\n    let mut sorted = values.to_vec();\n    sorted.sort_by(|a, b| \
         a.total_cmp(b));\n    let mid = sorted.len() / 2;\n    if sorted.len() % 2 == 0 {\n  \
               Some((sorted[mid] + sorted[mid + 1]) / 2.0)\n    } else {\n        \
         Some(sorted[mid])\n    }\n}\n",
    ),
    (
        "tests/stats.rs",
        "use stats::{mean, median};\n\n#[test]\nfn mean_of_values() {\n    \
         assert_eq!(mean(&[1.0, 2.0, 3.0, 4.0]), Some(2.5));\n}\n\n#[test]\nfn \
         median_of_an_odd_count() {\n    assert_eq!(median(&[3.0, 1.0, 2.0]), \
         Some(2.0));\n}\n\n#[test]\nfn median_of_an_even_count() {\n    \
         assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), Some(2.5));\n}\n\n#[test]\nfn \
         empty_input() {\n    assert_eq!(mean(&[]), None);\n    assert_eq!(median(&[]), \
         None);\n}\n",
    ),
];

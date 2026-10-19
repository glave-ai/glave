//! Which model judges the actions a rule flagged in auto mode (`judge_model`): the harness asks
//! it, config and policy choose it.

/// Which model judges flagged actions in auto mode, as `judge_model` chooses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Judge {
    /// The main model: the one threads that chose none go to.
    Main,
    Off,
    Model(String),
}

impl Judge {
    /// `judge_model` as written: `off`, a model, or nothing (the main model).
    pub fn named(text: &str) -> Self {
        match text.trim() {
            "" => Self::Main,
            "off" => Self::Off,
            model => Self::Model(model.to_owned()),
        }
    }

    /// The judge's model when the main one is `main`, or none when it is off.
    pub fn model(&self, main: &str) -> Option<String> {
        match self {
            Self::Main => Some(main.to_owned()),
            Self::Off => None,
            Self::Model(model) => Some(model.clone()),
        }
    }
}

impl std::fmt::Display for Judge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Main => f.write_str("the main model"),
            Self::Off => f.write_str("off"),
            Self::Model(model) => f.write_str(model),
        }
    }
}

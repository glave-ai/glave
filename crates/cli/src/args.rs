//! Command-line arguments.

use std::path::PathBuf;

pub const USAGE: &str = "\
glave: run hope, or any model, on the project in this folder

Usage:
  glave [options] [prompt...]   run one turn with this prompt
  glave [options]               start a session; each line you type is a turn

Options:
  --model <name>     the model to use (default: $GLAVE_MODEL)
  --yes              allow writes, edits and commands without asking
  --events <file>    save the thread's events to this file as JSON Lines
  --help             show this help
  --version          show the version

Environment:
  GLAVE_BASE_URL     an OpenAI-compatible API (default: http://localhost:11434/v1, Ollama)
  GLAVE_MODEL        the model name, such as gemma4:12b or an OpenRouter model id
  GLAVE_API_KEY      the API key, if the endpoint needs one
  GLAVE_PROVIDER     a short name recorded with each response (default: from the URL)";

/// What the person asked for.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub model: Option<String>,
    pub yes: bool,
    pub events: Option<PathBuf>,
    pub help: bool,
    pub version: bool,
    /// The one-shot prompt; empty starts an interactive session.
    pub prompt: String,
}

pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args = Args::default();
    let mut words = Vec::new();
    let mut rest = arguments.into_iter();
    while let Some(argument) = rest.next() {
        match argument.as_str() {
            "--model" => args.model = Some(rest.next().ok_or("--model needs a name")?),
            "--events" => {
                args.events = Some(PathBuf::from(rest.next().ok_or("--events needs a file")?));
            }
            "--yes" | "-y" => args.yes = true,
            "--help" | "-h" => args.help = true,
            "--version" | "-V" => args.version = true,
            "--" => words.extend(rest.by_ref()),
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            _ => words.push(argument),
        }
    }
    args.prompt = words.join(" ");
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_words(words: &[&str]) -> Result<Args, String> {
        parse(words.iter().map(|w| (*w).to_owned()))
    }

    #[test]
    fn prompt_words_join_and_flags_parse() {
        let args = parse_words(&["--yes", "fix", "the", "--model", "m", "test"]).unwrap();
        assert!(args.yes);
        assert_eq!(args.model.as_deref(), Some("m"));
        assert_eq!(args.prompt, "fix the test");
    }

    #[test]
    fn errors_are_explained() {
        assert!(parse_words(&["--model"]).is_err());
        assert!(parse_words(&["--bogus"]).is_err());
        assert_eq!(parse_words(&["--", "--yes"]).unwrap().prompt, "--yes");
    }
}

//! `glave doc`: office files read, written and edited by glave itself (glave-office), for the
//! model, which runs it through bash as the built-in skills teach, and for a person at a
//! terminal.

use std::io::{IsTerminal as _, Read};
use std::process::ExitCode;

use glave_office::{Answer, OfficeError};

/// `glave doc ...`.
pub(crate) fn main(words: &[String]) -> ExitCode {
    let stdin = std::io::stdin();
    // A description comes piped in, or from a heredoc; at a terminal there is none to wait for.
    let mut input: Box<dyn Read> = if stdin.is_terminal() {
        Box::new(std::io::empty())
    } else {
        Box::new(stdin.lock())
    };
    match glave_office::run(words, &mut input) {
        Ok(Answer { printed, warning }) => {
            println!("{}", printed.trim_end());
            if let Some(warning) = warning {
                eprintln!("glave: {warning}");
            }
            ExitCode::SUCCESS
        }
        Err(OfficeError::Usage(usage)) => {
            eprintln!("glave: {usage}");
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("glave: {error}");
            ExitCode::FAILURE
        }
    }
}

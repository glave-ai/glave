//! `glave doc`: spreadsheets read by glave itself (glave-office), for the model, which runs it
//! through bash as the built-in skill teaches, and for a person at a terminal.

use std::process::ExitCode;

use glave_office::OfficeError;

/// `glave doc ...`.
pub(crate) fn main(words: &[String]) -> ExitCode {
    match glave_office::run(words) {
        Ok(text) => {
            println!("{}", text.trim_end());
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

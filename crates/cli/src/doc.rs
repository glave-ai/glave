//! `glave doc`: spreadsheets read and written by glave itself (glave-office), for the model,
//! which runs it through bash as the built-in skill teaches, and for a person at a terminal.

use std::process::ExitCode;

use glave_office::OfficeError;

/// `glave doc ...`.
pub(crate) fn main(words: &[String]) -> ExitCode {
    match glave_office::run(words, &mut std::io::stdin().lock()) {
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

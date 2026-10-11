//! Prints the session API's JSON Schema (`specs/session.schema.json`); `cargo xtask schema`
//! runs it.

use std::io::Write as _;
use std::process::ExitCode;

fn main() -> ExitCode {
    let printed = glave_session::schema::document()
        .and_then(|document| serde_json::to_string_pretty(&document).map_err(|e| e.to_string()))
        .and_then(|text| {
            let mut out = std::io::stdout().lock();
            writeln!(out, "{text}").map_err(|e| e.to_string())
        });
    match printed {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("schema: {error}");
            ExitCode::FAILURE
        }
    }
}

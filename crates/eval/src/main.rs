//! `glave-eval`: runs the eval suite. See `glave_eval::cli`.

fn main() -> std::process::ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if let Some(code) = glave_process::lifeline_helper(&args) {
        return code;
    }
    glave_eval::cli::main()
}

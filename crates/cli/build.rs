//! With the `web` feature, builds the web app into glave: every file of the app's build
//! (`npm run build` in `web/`) becomes an entry of a table in `OUT_DIR/app.rs`, its bytes
//! included at compile time, which glave serves from memory (`src/serving.rs`). Without the
//! feature the table is `None`, and nothing of `web/` is read.

use std::fmt::Write as _;
use std::path::Path;

/// The app's build, from this crate's folder.
const BUILD: &str = "../../web/dist/app/browser";

/// Why there is nothing to build in.
const NO_BUILD: &str = "the web feature builds the web app into glave, and there is no build \
                        of it in web/dist/app/browser: run `npm ci && npm run build` in web/ \
                        first";

fn main() {
    if let Err(message) = write() {
        // Cargo shows it and fails the build.
        println!("cargo::error={message}");
    }
}

/// Writes the table: the app's files with the feature, else `None`.
fn write() -> Result<(), String> {
    println!("cargo::rerun-if-changed=build.rs");
    let table = if std::env::var_os("CARGO_FEATURE_WEB").is_some() {
        let build = Path::new(&variable("CARGO_MANIFEST_DIR")?).join(BUILD);
        // New and removed files too: Cargo looks through the whole folder.
        println!("cargo::rerun-if-changed={}", build.display());
        embed(&build)?
    } else {
        "None".to_owned()
    };
    let out = Path::new(&variable("OUT_DIR")?).join("app.rs");
    std::fs::write(&out, table).map_err(|e| format!("could not write {}: {e}", out.display()))
}

/// `Some(&[("index.html", include_bytes!("…/index.html")), …])`, every file in `build` by its
/// path inside it, in order.
fn embed(build: &Path) -> Result<String, String> {
    let build = std::fs::canonicalize(build).map_err(|_| NO_BUILD.to_owned())?;
    if !build.join("index.html").is_file() {
        return Err(NO_BUILD.to_owned());
    }
    let mut files = Vec::new();
    walk(&build, &build, &mut files)?;
    files.sort();
    let mut table = String::from("Some(&[\n");
    for (name, path) in files {
        // Debug-formatted, each is a Rust string literal.
        let _ = writeln!(table, "    ({name:?}, include_bytes!({path:?})),");
    }
    table.push_str("])\n");
    Ok(table)
}

/// Every file under `folder`, which is in `root`: its path inside `root`, its parts joined by
/// `/`, and its path on disk. Links are left out: one could lead outside the build.
fn walk(root: &Path, folder: &Path, files: &mut Vec<(String, String)>) -> Result<(), String> {
    let unreadable =
        |path: &Path, e: std::io::Error| format!("could not read {}: {e}", path.display());
    let entries = std::fs::read_dir(folder).map_err(|e| unreadable(folder, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| unreadable(folder, e))?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|e| unreadable(&path, e))?;
        if kind.is_dir() {
            walk(root, &path, files)?;
        } else if kind.is_file() {
            let not_text = || format!("{} is not named in UTF-8", path.display());
            let inside = path
                .strip_prefix(root)
                .map_err(|_| format!("{} is outside {}", path.display(), root.display()))?;
            let parts: Option<Vec<&str>> = inside.iter().map(|part| part.to_str()).collect();
            let name = parts.ok_or_else(not_text)?.join("/");
            let disk = path.to_str().ok_or_else(not_text)?.to_owned();
            files.push((name, disk));
        }
    }
    Ok(())
}

fn variable(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("Cargo did not set {name}"))
}

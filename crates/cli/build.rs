//! With the `web` feature, builds the web app into glave: every file of the app's build
//! (`npm run build` in `web/`) becomes an entry of a table in `OUT_DIR/app.rs`, its bytes
//! included at compile time, which glave serves from memory (`src/serving.rs`). A file that
//! gzip makes smaller is included gzipped, and served as it is stored; `index.html` stays as
//! it is, since each load writes its nonce into it. Without the feature the table is `None`,
//! and nothing of `web/` is read.

#[cfg(feature = "web")]
use std::fmt::Write as _;
use std::path::Path;

/// The app's build, from this crate's folder.
#[cfg(feature = "web")]
const BUILD: &str = "../../web/dist/app/browser";

/// Why there is nothing to build in.
#[cfg(feature = "web")]
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
    #[cfg(feature = "web")]
    let table = {
        let build = Path::new(&variable("CARGO_MANIFEST_DIR")?).join(BUILD);
        // New and removed files too: Cargo looks through the whole folder.
        println!("cargo::rerun-if-changed={}", build.display());
        embed(&build, &Path::new(&variable("OUT_DIR")?).join("app"))?
    };
    #[cfg(not(feature = "web"))]
    let table = "None".to_owned();
    let out = Path::new(&variable("OUT_DIR")?).join("app.rs");
    std::fs::write(&out, table).map_err(|e| format!("could not write {}: {e}", out.display()))
}

/// `Some(&[("index.html", Stored::Plain(include_bytes!("…/index.html"))), …])`, every file in
/// `build` by its path inside it, in order; those gzip makes smaller gzipped into `gzipped`.
#[cfg(feature = "web")]
fn embed(build: &Path, gzipped: &Path) -> Result<String, String> {
    let build = std::fs::canonicalize(build).map_err(|_| NO_BUILD.to_owned())?;
    if !build.join("index.html").is_file() {
        return Err(NO_BUILD.to_owned());
    }
    let mut files = Vec::new();
    walk(&build, &build, &mut files)?;
    files.sort();
    let mut table = String::from("Some(&[\n");
    for (name, path) in files {
        let (stored, path) = match gzip(&name, &path, gzipped)? {
            Some(smaller) => ("Gzip", smaller),
            None => ("Plain", path),
        };
        // Debug-formatted, each is a Rust string literal.
        let _ = writeln!(
            table,
            "    ({name:?}, glave_session::http::Stored::{stored}(include_bytes!({path:?}))),"
        );
    }
    table.push_str("])\n");
    Ok(table)
}

/// The file `name`, at `path`, gzipped into `folder` at its best compression, and the gzipped
/// file's path; `None` for `index.html`, and for a file gzip does not make smaller.
#[cfg(feature = "web")]
fn gzip(name: &str, path: &str, folder: &Path) -> Result<Option<String>, String> {
    use std::io::Write as _;
    if name == "index.html" {
        return Ok(None);
    }
    let plain = std::fs::read(path).map_err(|e| format!("could not read {path}: {e}"))?;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    let gzipped = encoder
        .write_all(&plain)
        .and_then(|()| encoder.finish())
        .map_err(|e| format!("could not gzip {path}: {e}"))?;
    if gzipped.len() >= plain.len() {
        return Ok(None);
    }
    let out = folder.join(format!("{name}.gz"));
    let written = out
        .to_str()
        .ok_or_else(|| format!("{} is not UTF-8", out.display()))?;
    let written = written.to_owned();
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not make {}: {e}", parent.display()))?;
    }
    std::fs::write(&out, gzipped).map_err(|e| format!("could not write {written}: {e}"))?;
    Ok(Some(written))
}

/// Every file under `folder`, which is in `root`: its path inside `root`, its parts joined by
/// `/`, and its path on disk. Links are left out: one could lead outside the build.
#[cfg(feature = "web")]
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

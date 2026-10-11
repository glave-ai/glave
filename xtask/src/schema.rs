//! `cargo xtask schema`: writes the session API's JSON Schema, generated from the Rust protocol
//! types, and the TypeScript types made from it. With `--check` (a step of `cargo xtask ci`) it
//! writes nothing and fails when either checked-in file differs from what the code makes.

use std::path::Path;

use crate::{cmd, typescript, workspace};

/// The schema, checked in beside the spec it describes.
const SCHEMA: &str = "specs/session.schema.json";
/// The web client's generated types.
const TYPES: &str = "web/client/src/generated/api.ts";

/// Writes both files, or with `--check` compares them with what the code makes.
pub fn run(args: &[String]) -> Result<(), String> {
    let check = match args {
        [] => false,
        [flag] if flag == "--check" => true,
        _ => return Err("usage: cargo xtask schema [--check]".to_owned()),
    };
    let root = workspace::root()?;
    let schema = cmd::output(
        "cargo",
        &[
            "run",
            "--quiet",
            "-p",
            "glave-session",
            "--example",
            "schema",
        ],
    )?;
    let parsed = serde_json::from_str(&schema)
        .map_err(|e| format!("the generated schema is not JSON: {e}"))?;
    let types = typescript::module(&parsed)?;
    for (path, made) in [(SCHEMA, schema), (TYPES, types)] {
        let full = root.join(path);
        if check {
            compare(&full, path, &made)?;
        } else {
            if let Some(folder) = full.parent() {
                std::fs::create_dir_all(folder)
                    .map_err(|e| format!("could not make {}: {e}", folder.display()))?;
            }
            std::fs::write(&full, made).map_err(|e| format!("could not write {path}: {e}"))?;
            println!("xtask: wrote {path}");
        }
    }
    Ok(())
}

fn compare(full: &Path, path: &str, made: &str) -> Result<(), String> {
    let found = std::fs::read_to_string(full).unwrap_or_default();
    if found == made {
        println!("xtask: {path} matches the code");
        Ok(())
    } else {
        Err(format!(
            "{path} no longer matches the session API's Rust types: run `cargo xtask schema` \
             and commit what it writes"
        ))
    }
}

use glave_ports::block_on;
use glave_ports::testing::TempDir;

use super::*;

fn scratch(name: &str) -> TempDir {
    let dir = TempDir::new(&format!("ws-{name}")).unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    dir
}

#[test]
fn paths_stay_inside_the_workspace() {
    let dir = scratch("confine");
    block_on!(async {
        let ws = Workspace::open(&dir, std::env::vars_os()).await.unwrap();
        let root = ws.root().to_owned();
        assert_eq!(ws.stat("src/main.rs").await.unwrap().path, "src/main.rs");
        assert_eq!(
            ws.stat("new/dir/file.txt").await.unwrap().path,
            "new/dir/file.txt"
        );
        assert_eq!(
            ws.stat(root.join("src").to_str().unwrap())
                .await
                .unwrap()
                .path,
            "src"
        );
        assert_eq!(
            ws.stat("../outside.txt").await.unwrap_err(),
            format!(
                "../outside.txt is outside the workspace ({}).",
                root.display()
            )
        );
        assert!(ws.stat("src/../../outside.txt").await.is_err());
        assert!(ws.stat("/etc/passwd").await.is_err());
        assert!(ws.stat("missing/../../escape").await.is_err());
        assert_eq!(ws.stat("  ").await.unwrap_err(), "The path is empty.");
    });
}

#[cfg(unix)]
#[test]
fn symlinks_cannot_escape() {
    let dir = scratch("symlink");
    std::os::unix::fs::symlink(std::env::temp_dir(), dir.join("out")).unwrap();
    block_on!(async {
        let ws = Workspace::open(&dir, std::env::vars_os()).await.unwrap();
        assert!(ws.stat("out/anything").await.is_err());
    });
}

/// A link whose target does not exist yet looks like a new file inside the project, but
/// writing to it would create the target, wherever that is.
#[cfg(unix)]
#[test]
fn dangling_symlinks_are_refused() {
    let dir = scratch("dangling");
    let outside = dir.with_extension("escape");
    std::os::unix::fs::symlink(&outside, dir.join("escape")).unwrap();
    block_on!(async {
        let ws = Workspace::open(&dir, std::env::vars_os()).await.unwrap();
        assert!(ws.stat("escape").await.is_err());
        assert!(ws.stat("escape/deeper.txt").await.is_err());
    });
}

#[test]
fn changes_need_a_fresh_read() {
    let dir = scratch("fresh");
    block_on!(async {
        let ws = Workspace::open(&dir, std::env::vars_os()).await.unwrap();
        let file = ws.stat("src/main.rs").await.unwrap();
        let now = file.digest.as_ref();
        assert!(ws.check_fresh(&file.path, now).is_err());
        ws.remember(&file.path, b"fn main() {}\n");
        assert!(ws.check_fresh(&file.path, now).is_ok());
        assert!(
            ws.check_fresh(&file.path, Some(&digest(b"changed")))
                .is_err()
        );
        assert!(
            ws.check_fresh(&file.path, None).is_err(),
            "unknown is not fresh"
        );
    });
}

/// Moves `docs` aside, puts a link to each of `targets` in turn in its place, and moves it
/// back, as a sandboxed command running beside the file tools could, until told to stop,
/// counting its swaps in `swaps`.
#[cfg(unix)]
fn swap_for_links(
    dir: &Path,
    targets: &[&str],
    stop: &std::sync::atomic::AtomicBool,
    swaps: &std::sync::atomic::AtomicUsize,
) {
    use std::sync::atomic::Ordering;
    let (folder, parked) = (dir.join("docs"), dir.join("docs.parked"));
    while !stop.load(Ordering::Relaxed) {
        if std::fs::rename(&folder, &parked).is_ok() {
            let swapped = swaps.load(Ordering::Relaxed);
            let target = targets.get(swapped % targets.len().max(1)).unwrap();
            if std::os::unix::fs::symlink(target, &folder).is_ok() {
                swaps.fetch_add(1, Ordering::Relaxed);
            }
            let _ = std::fs::remove_file(&folder);
            while std::fs::rename(&parked, &folder).is_err() {
                let _ = std::fs::remove_dir_all(&folder);
            }
        }
    }
}

/// Writes the race test makes at least, enough that many meet a swap mid-way.
#[cfg(unix)]
const WRITES: usize = 500;

/// The write tool in a sandbox, raced by a command swapping a folder for a link into
/// `.git/hooks` or `.glave` between the tool's protected-path check and its write: nothing
/// protected is ever written, and every refusal reads as the check's own sentence.
/// It writes until the race has run and been met both ways (a write refused as protected,
/// one landed, many swaps and writes), rather than for a fixed time, which a loaded machine
/// can starve (#982).
#[cfg(unix)]
#[test]
fn a_folder_swapped_for_a_link_between_check_and_write_writes_nothing_protected() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use glave_event::Arguments;
    use glave_ports::computer::SandboxLevel;

    use crate::cancel::Cancel;
    use crate::tools::Tool;

    let dir = scratch("protected-race");
    std::fs::create_dir_all(dir.join(".git/hooks")).unwrap();
    std::fs::create_dir_all(dir.join(".glave")).unwrap();
    std::fs::write(dir.join(".glave/settings.json"), "{}\n").unwrap();
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let swaps = Arc::new(AtomicUsize::new(0));
    let swapper = {
        let (dir, stop, swaps) = (dir.to_path_buf(), Arc::clone(&stop), Arc::clone(&swaps));
        std::thread::spawn(move || swap_for_links(&dir, &[".git/hooks", ".glave"], &stop, &swaps))
    };
    let mut refusals = Vec::new();
    let mut landed = 0usize;
    block_on!(async {
        let mut ws = Workspace::open(&dir, std::env::vars_os()).await.unwrap();
        // The sandbox as the binding records it; no command runs here.
        ws.sandbox = Some(Sandbox {
            level: SandboxLevel::Project,
            network: false,
            home: None,
            glave_home: None,
        });
        let started = std::time::Instant::now();
        let mut n = 0usize;
        let met = |refusals: &[String], landed, n| {
            n >= WRITES
                && swaps.load(Ordering::Relaxed) > 100
                && landed > 0
                && refusals.iter().any(|refusal| refusal.contains("protected"))
        };
        while !met(&refusals, landed, n) {
            assert!(
                started.elapsed() < glave_ports::testing::HUNG,
                "the race was not met in {:?}: {} swaps, {landed} writes landed, {} refused",
                glave_ports::testing::HUNG,
                swaps.load(Ordering::Relaxed),
                refusals.len()
            );
            n += 1;
            let arguments = Arguments::Json(
                serde_json::from_value(serde_json::json!({
                    "path": format!("docs/pre-commit-{n}"),
                    "content": "echo pwned\n",
                }))
                .unwrap(),
            );
            let at = (&ws, &crate::background::Commands::default());
            let outcome = Tool::Write
                .run(&arguments, at, usize::MAX, &Cancel::new())
                .await;
            if outcome.is_error {
                refusals.push(outcome.text);
            } else {
                landed += 1;
            }
        }
    });
    stop.store(true, Ordering::Relaxed);
    swapper.join().unwrap();
    // Others fail because the folder moved under them, which is fine: they wrote nothing.
    refusals.retain(|refusal| refusal.contains("protected"));
    for refusal in &refusals {
        let path = refusal.split(' ').next().unwrap_or_default();
        assert!(
            path.starts_with(".git/hooks/pre-commit-") || path.starts_with(".glave/pre-commit-"),
            "{refusal}"
        );
        assert_eq!(
            refusal,
            &format!(
                "{path} is protected by the sandbox: changing it could make code run later, \
                 outside the sandbox. Ask the person to change it themselves."
            ),
        );
    }
    nothing_new_in_protected(&dir);
}

/// Nothing was written into `.git/hooks` or `.glave` under `dir`.
#[cfg(unix)]
fn nothing_new_in_protected(dir: &Path) {
    let hooks: Vec<_> = std::fs::read_dir(dir.join(".git/hooks"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(hooks.is_empty(), "a hook was written: {hooks:?}");
    let settings: Vec<_> = std::fs::read_dir(dir.join(".glave"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(settings, ["settings.json"], "nothing new in .glave");
}

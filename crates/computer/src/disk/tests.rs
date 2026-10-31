use std::process::Command as Process;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use glave_ports::block_on;
use glave_ports::testing::{ManualClock, TempDir};

use super::*;

/// A disk more than 80% used, and one with room.
const FULL: DiskSpace = DiskSpace {
    size: 100,
    free: 10,
};
const ROOMY: DiskSpace = DiskSpace {
    size: 100,
    free: 50,
};

/// What the tests' git and shell run with: the home `home`, and none of this machine's git
/// settings.
fn environment(home: &Path) -> Vec<(OsString, OsString)> {
    let mut environment: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(name, _)| name == "PATH")
        .collect();
    for (name, value) in [
        ("HOME", home.as_os_str()),
        ("GIT_CONFIG_NOSYSTEM", "1".as_ref()),
        ("GIT_CONFIG_GLOBAL", "/dev/null".as_ref()),
    ] {
        environment.push((name.into(), value.into()));
    }
    environment
}

fn sh(at: &Path, home: &Path, script: &str) {
    let out = Process::new("bash")
        .args(["-c", script])
        .current_dir(at)
        .env_clear()
        .envs(environment(home))
        .output()
        .unwrap();
    assert!(out.status.success(), "{script}: {out:?}");
}

/// A person's home in `dir`, everything in it untouched since 2020: a worktree glave made
/// whose `target/` and `node_modules/` git ignores, and whose `build/` it ignores but tracks a
/// file in; one whose ignored `node_modules` is a link to the person's own files; caches;
/// and something in the Trash. Each dated, in the order it would go.
fn person_home(dir: &Path) -> (PathBuf, PathBuf) {
    let home = dir.join("home");
    std::fs::create_dir_all(home.join(".glave/worktrees/app")).unwrap();
    sh(
        &home,
        &home,
        "mkdir mine && touch mine/kept && cd .glave/worktrees/app && \
         git init -q wt && cd wt && printf 'target/\\nnode_modules\\nbuild/\\n' > .gitignore && \
         mkdir target node_modules build && touch target/a node_modules/b build/c && \
         git add .gitignore && git add -f build/c && \
         git -c user.name=t -c user.email=t@example.com commit -q -m one && \
         cd .. && git init -q linked && cd linked && printf 'node_modules\\n' > .gitignore && \
         ln -s ../../../../mine node_modules && cd ../../../.. && \
         mkdir -p .npm/_cacache .cargo/registry/cache .cache/pip .cache/uv \
         .glave/sandbox-cache/pip .glave/sandbox-cache/rustup/toolchains junk && \
         touch -t 202001010000 .glave/worktrees/app/wt/node_modules .cache/uv && \
         touch -t 202002010000 .glave/worktrees/app/wt/target .cache/pip && \
         touch -t 202003010000 .npm/_cacache .cargo/registry/cache && \
         touch -t 202004010000 .glave/sandbox-cache/pip && \
         touch -t 202005010000 .glave/sandbox-cache/rustup/toolchains && \
         touch .glave/sandbox-cache/rustup/settings.toml",
    );
    // The Trash where this system keeps it, as glave puts things in it.
    let trash = Trash::for_home(&home, None, "").unwrap();
    let trashed = block_on!(trash.put(&home.join("junk"))).unwrap();
    let held = block_on!(trash.held(10));
    for (path, note) in &held {
        let dated = note.as_ref().unwrap_or(path);
        sh(
            &home,
            &home,
            &format!("touch -h -t 202001010000 '{}'", dated.display()),
        );
    }
    (home, trashed)
}

/// A keeper of `home`, whose disk is full while `full` says, on a clock in 2026.
fn keeper(home: &Path, full: impl Fn() -> bool + Send + Sync + 'static) -> Keeper {
    let glave = home.join(".glave");
    let mut kept = Keeper::new(
        glave.clone(),
        glave.join("worktrees"),
        Processes::unrecorded(),
        environment(home),
        Box::new(|| false),
        Largest::default(),
    );
    kept.measure = Box::new(move |_| Ok(if full() { FULL } else { ROOMY }));
    kept.clock = Box::new(ManualClock::at_millis(1_791_001_000_000));
    kept
}

fn freed(home: &Path) -> Vec<String> {
    std::fs::read_to_string(home.join(".glave").join(RECORD))
        .unwrap_or_default()
        .lines()
        .map(|line| line.split_once(' ').map_or("", |(_, rest)| rest).to_owned())
        .collect()
}

#[test]
fn a_full_disk_frees_build_outputs_then_caches_then_the_trash_oldest_first() {
    let dir = TempDir::new("disk-keep").unwrap();
    let (home, trashed) = person_home(&dir);
    let watched = trashed.clone();
    let kept = keeper(&home, move || watched.exists());
    assert!(
        block_on!(kept.keep(&|| true)).unwrap(),
        "room once all of it went"
    );
    let wt = home.join(".glave/worktrees/app/wt");
    let said = |kind: &str, path: PathBuf| format!("{kind} {}", path.display());
    assert_eq!(
        freed(&home),
        [
            said("built", wt.join("node_modules")),
            said("built", wt.join("target")),
            said("cache", home.join(".cache/uv")),
            said("cache", home.join(".cache/pip")),
            said("cache", home.join(".npm/_cacache")),
            said("cache", home.join(".cargo/registry/cache")),
            said("cache", home.join(".glave/sandbox-cache/pip")),
            said("cache", home.join(".glave/sandbox-cache/rustup/toolchains")),
            said("trash", trashed),
        ]
    );
    assert!(
        wt.join("build/c").exists(),
        "what git tracks is never freed"
    );
    assert!(home.join("mine/kept").exists(), "a link is never followed");
    assert!(
        home.join(".glave/sandbox-cache/rustup/settings.toml")
            .exists(),
        "the sandbox's homes are never freed whole"
    );
    let left = block_on!(Trash::for_home(&home, None, "").unwrap().held(10));
    assert_eq!(left.len(), 0, "{left:?}");
}

#[test]
fn it_stops_once_there_is_room_or_something_runs() {
    let dir = TempDir::new("disk-stop").unwrap();
    let (home, trashed) = person_home(&dir);
    let pip = home.join(".cache/pip");
    let kept = keeper(&home, move || pip.exists());
    assert!(block_on!(kept.keep(&|| true)).unwrap());
    assert_eq!(freed(&home).len(), 4, "outputs, then caches up to pip's");
    assert!(home.join(".npm/_cacache").exists() && trashed.exists());

    // Something starts running: nothing more goes.
    let dir = TempDir::new("disk-busy").unwrap();
    let (home, _) = person_home(&dir);
    let runs = Arc::new(AtomicBool::new(false));
    let quiet = {
        let runs = Arc::clone(&runs);
        move || !runs.swap(true, Ordering::SeqCst)
    };
    let kept = keeper(&home, || true);
    assert!(
        block_on!(kept.keep(&quiet)).unwrap(),
        "it stopped for what runs"
    );
    assert_eq!(freed(&home).len(), 1, "the one freed before it ran");
}

#[test]
fn a_disk_with_room_frees_nothing_and_a_full_one_with_nothing_left_says_so() {
    let dir = TempDir::new("disk-room").unwrap();
    let (home, _) = person_home(&dir);
    assert!(block_on!(keeper(&home, || false).keep(&|| true)).unwrap());
    assert_eq!(freed(&home).len(), 0);
    assert!(
        !block_on!(keeper(&home, || true).keep(&|| true)).unwrap(),
        "still full once all of it went"
    );
}

#[test]
fn a_disk_is_full_past_eighty_percent_used() {
    let at = |free| DiskSpace { size: 1_000, free };
    assert!(!at(200).full());
    assert!(at(199).full());
    let empty = DiskSpace { size: 0, free: 0 };
    assert!(!empty.full());
    assert!(space(Path::new("/")).unwrap().size > 0, "the real disk's");
}

#[test]
fn what_uses_the_disk_most_is_named_largest_first_never_through_a_link() {
    let dir = TempDir::new("disk-largest").unwrap();
    let home = dir.join("home");
    let write = |path: &str, bytes: usize| {
        let path = home.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![1_u8; bytes]).unwrap();
    };
    write("glave/garden/deep/down/big", 4_000_000);
    write("glave/shed/small", 2_000);
    write("loose", 2_000_000);
    // What fills a folder as many small files names the folder.
    for n in 0..200 {
        write(&format!("data/file-{n}"), 10_000);
    }
    std::fs::create_dir_all(dir.join("outside")).unwrap();
    std::fs::write(dir.join("outside/huge"), vec![1_u8; 5_000_000]).unwrap();
    std::os::unix::fs::symlink(dir.join("outside"), home.join("glave/linked")).unwrap();
    let found = largest(&home);
    let paths: Vec<&str> = found.iter().map(|use_| use_.path.as_str()).collect();
    assert_eq!(paths, ["glave/garden", "data", "loose"], "{found:?}");
    assert!(found[0].bytes >= 4_000_000, "{found:?}");
    assert_eq!(largest(&dir.join("nothing here")), Vec::new());
}

#[test]
fn the_keeper_says_what_uses_the_disk_only_with_nothing_left_to_free_and_past_90_percent() {
    let dir = TempDir::new("disk-told").unwrap();
    let (home, _) = person_home(&dir);
    let told = Largest::default();
    let mut kept = keeper(&home, || true);
    kept.largest = told.clone();
    kept.measure = Box::new(|_| Ok(DiskSpace { size: 100, free: 9 }));
    block_on!(kept.look_at_what_is_left());
    assert!(!told.get().is_empty(), "past 90%: it says");
    kept.measure = Box::new(|_| {
        Ok(DiskSpace {
            size: 100,
            free: 15,
        })
    });
    block_on!(kept.look_at_what_is_left());
    assert_eq!(told.get(), Vec::new(), "85%: it doesn't");
}

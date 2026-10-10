use super::*;

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glave-ws-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    dir
}

#[test]
fn paths_stay_inside_the_workspace() {
    let dir = scratch("confine");
    block_on(async {
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
    block_on(async {
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
    let outside = std::env::temp_dir().join(format!("glave-ws-escape-{}", std::process::id()));
    let _ = std::fs::remove_file(&outside);
    std::os::unix::fs::symlink(&outside, dir.join("escape")).unwrap();
    block_on(async {
        let ws = Workspace::open(&dir, std::env::vars_os()).await.unwrap();
        assert!(ws.stat("escape").await.is_err());
        assert!(ws.stat("escape/deeper.txt").await.is_err());
    });
}

#[test]
fn changes_need_a_fresh_read() {
    let dir = scratch("fresh");
    block_on(async {
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

//! Where Rust's homes are for a sandboxed command.

use glave_ports::block_on;
use glave_ports::computer::Ending;

use super::{computers, computers_saying, enforced, open_app, run, scratch};

/// A laptop leaves Rust's homes alone; a computer whose own environment asks has its sandboxed
/// commands keep them in the sandbox cache, where they can write.
#[test]
fn rust_s_homes_go_in_the_sandbox_cache_only_where_the_computer_says() {
    if !enforced() {
        return;
    }
    let dir = scratch("toolchains");
    std::fs::create_dir_all(dir.join("app")).unwrap();
    std::fs::create_dir_all(dir.join("home/.glave")).unwrap();
    let homes = r#"printf '%s %s' "$CARGO_HOME" "$RUSTUP_HOME"; mkdir -p "$CARGO_HOME/registry""#;
    block_on!(async {
        let (laptop, sandbox) = computers(&dir);
        let there = open_app(&laptop, &dir).await;
        let (_, said) = run(
            &laptop,
            there,
            &sandbox,
            r#"printf '%s' "$CARGO_HOME$RUSTUP_HOME""#,
        )
        .await;
        assert_eq!(said, "", "a laptop's are left alone");

        let (image, sandbox) = computers_saying(&dir, &[("GLAVE_SANDBOX_TOOLCHAINS", "1")]);
        let there = open_app(&image, &dir).await;
        let (ending, said) = run(&image, there, &sandbox, homes).await;
        assert_eq!(ending, Ending::Exited { code: 0 }, "{said}");
        let cache = std::fs::canonicalize(dir.join("home/.glave"))
            .unwrap()
            .join("sandbox-cache");
        assert_eq!(
            said,
            format!(
                "{} {}",
                cache.join("cargo").display(),
                cache.join("rustup").display()
            )
        );
        assert!(
            cache.join("cargo/registry").is_dir(),
            "they can write there"
        );
    });
}

/// On a computer whose sandboxed commands keep Rust's homes in the sandbox cache, a tool `cargo
/// install` puts there runs by its name, as one in Cargo's own home would.
#[test]
fn a_tool_installed_in_the_sandbox_s_cargo_home_is_on_path() {
    if !enforced() {
        return;
    }
    let dir = scratch("cargo-bin");
    std::fs::create_dir_all(dir.join("app")).unwrap();
    std::fs::create_dir_all(dir.join("home/.glave")).unwrap();
    let path = format!(
        "{}:{}",
        dir.join("home/.cargo/bin").display(),
        std::env::var("PATH").unwrap()
    );
    let install = r#"mkdir -p "$CARGO_HOME/bin" && printf '#!/bin/sh\necho ran\n' > "$CARGO_HOME/bin/installed-tool" && chmod +x "$CARGO_HOME/bin/installed-tool" && installed-tool"#;
    block_on!(async {
        let said = [("GLAVE_SANDBOX_TOOLCHAINS", "1"), ("PATH", path.as_str())];
        let (image, sandbox) = computers_saying(&dir, &said);
        let there = open_app(&image, &dir).await;
        let (ending, said) = run(&image, there, &sandbox, install).await;
        assert_eq!(ending, Ending::Exited { code: 0 }, "{said}");
        assert_eq!(said.trim(), "ran");
    });
}

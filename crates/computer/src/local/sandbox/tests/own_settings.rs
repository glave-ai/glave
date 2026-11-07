//! A project that holds git's own settings or glave's folder, under the real sandbox: however
//! the project is set up they stay read-only, and everything else in it is the command's.

use glave_ports::computer::{Bytes, FileWrite};

use super::*;

/// A workspace at `folder`, with the sandbox protecting `dir/home`.
struct Project {
    computers: LocalComputers,
    sandbox: Sandbox,
    workspace: WorkspaceId,
    root: PathBuf,
}

fn project(dir: &Path, folder: &Path) -> Project {
    std::fs::create_dir_all(folder).unwrap();
    let (computers, sandbox) = computers(dir);
    let path = folder.to_string_lossy().into_owned();
    let workspace = block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap();
        computers
            .open(computer.id, &Sources::Folder { path })
            .await
            .unwrap()
            .id
    });
    Project {
        computers,
        sandbox,
        workspace,
        root: std::fs::canonicalize(folder).unwrap(),
    }
}

impl Project {
    /// `script`, in the sandbox: what it said.
    fn sandboxed(&self, script: &str) -> String {
        block_on!(run(&self.computers, self.workspace, &self.sandbox, script)).1
    }

    /// The same, let out of the sandbox.
    fn let_out(&self, script: &str) {
        let output = Collect::default();
        let finished = block_on!(self.computers.run(
            self.workspace,
            &Command::new(script),
            &output,
            &CancelSwitch::default()
        ))
        .unwrap();
        assert_eq!(finished.ending, Ending::Exited { code: 0 });
    }

    /// `path` as glave's file tools write it, in the sandbox.
    fn tool_writes(&self, path: &str) -> Result<(), ComputerError> {
        let write = FileWrite {
            protect: Some(self.sandbox.clone()),
            ..FileWrite::new(path, Bytes::new(b"PLANTED"))
        };
        block_on!(self.computers.write(self.workspace, &[write]))
    }
}

fn holds_planted(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|text| text.contains("PLANTED"))
}

/// Every way of reaching git's settings at `relative` in `project`, which are really at `real`,
/// fails whether they are there yet or not; its own files stay the command's; and a command let
/// out of the sandbox, the person's way, still gets there.
fn kept_but_the_rest_is_writable(project: &Project, relative: &str, real: &Path) {
    let config = format!("{relative}/config");
    let said = project.sandboxed(&format!(
        "mkdir -p {relative} ; echo PLANTED > {config} ; echo fine > mine.txt && echo done"
    ));
    assert!(said.contains("done"), "{said}");
    assert!(project.root.join("mine.txt").exists(), "{said}");
    assert!(!holds_planted(&real.join("config")), "{said}");
    let by_tool = project.tool_writes(&config);
    assert!(
        by_tool
            .as_ref()
            .is_err_and(glave_ports::computer::is_protected_refusal),
        "{by_tool:?}"
    );
    assert!(project.tool_writes("mine-too.txt").is_ok());
    assert!(
        !holds_planted(&real.join("config")),
        "the file tool wrote it"
    );

    // There now, as the person's own: it can't be changed, moved aside and replaced, or removed.
    std::fs::create_dir_all(real).unwrap();
    std::fs::write(real.join("config"), "[theirs]\n").unwrap();
    let said = project.sandboxed(&format!(
        "echo PLANTED >> {config} ; ln -sf /dev/null {config} ; \
         mv {relative} moved ; mkdir -p {relative} ; echo PLANTED > {relative}/hooks ; echo done"
    ));
    assert!(said.contains("done"), "{said}");
    assert!(!holds_planted(&real.join("hooks")), "{said}");
    // Not moved: Seatbelt refuses the rename, and across bubblewrap's mounts `mv` copies and
    // cannot remove the original.
    assert_eq!(
        std::fs::read_to_string(real.join("config")).ok().as_deref(),
        Some("[theirs]\n"),
        "{said}"
    );
    project.sandboxed(&format!("rm -rf {relative} ; echo done"));
    assert_eq!(
        std::fs::read_to_string(real.join("config")).unwrap(),
        "[theirs]\n"
    );

    project.let_out(&format!("echo PLANTED > {config}"));
    assert!(
        holds_planted(&real.join("config")),
        "the person's way is shut"
    );
}

#[test]
fn a_project_at_dot_config_keeps_git_s_settings_there_read_only() {
    if !enforced() {
        return;
    }
    let dir = scratch("own-settings-config");
    let config = dir.join("home/.config");
    let project = project(&dir, &config);
    // Another spelling of the name is the same folder where names ignore case.
    let said = project.sandboxed("mkdir GIT ; echo PLANTED > GIT/config ; echo done");
    assert!(said.contains("done"), "{said}");
    assert!(!holds_planted(&config.join("git/config")), "{said}");
    assert!(project.tool_writes("GIT/config").is_err() || !ignores_case(&config));
    assert!(!holds_planted(&config.join("git/config")));
    project.sandboxed("rmdir GIT");
    kept_but_the_rest_is_writable(&project, "git", &config.join("git"));
}

#[test]
fn a_dotfiles_project_git_s_settings_link_into_keeps_them_read_only() {
    if !enforced() {
        return;
    }
    let dir = scratch("own-settings-stow");
    let dotfiles = dir.join("home/dotfiles");
    let real = dotfiles.join("git/.config/git");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::create_dir_all(dir.join("home/.config")).unwrap();
    std::os::unix::fs::symlink(&real, dir.join("home/.config/git")).unwrap();
    let project = project(&dir, &dotfiles);
    kept_but_the_rest_is_writable(&project, "git/.config/git", &real);
    // Nor can the folders on the way to them carry them off and back.
    project.sandboxed("mv git away ; echo PLANTED > away/.config/git/hooks ; mv away git ; echo");
    assert!(!holds_planted(&real.join("hooks")));
}

#[test]
fn a_gitconfig_that_is_a_link_into_the_project_stays_read_only() {
    if !enforced() {
        return;
    }
    let dir = scratch("own-settings-file");
    let dotfiles = dir.join("home/dotfiles");
    std::fs::create_dir_all(&dotfiles).unwrap();
    let real = dotfiles.join("gitconfig");
    std::fs::write(&real, "[user]\n").unwrap();
    std::os::unix::fs::symlink(&real, dir.join("home/.gitconfig")).unwrap();
    let project = project(&dir, &dotfiles);
    let said = project.sandboxed(
        "echo PLANTED >> gitconfig ; mv gitconfig moved ; echo PLANTED > gitconfig ; \
         echo fine > mine.txt && echo done",
    );
    assert!(said.contains("done"), "{said}");
    assert!(!holds_planted(&real), "{said}");
    assert!(real.exists(), "{said}");
}

/// Where `git_settings` link through a link inside the project, removing that link or making
/// what it points to would change them.
#[test]
fn links_on_the_way_to_git_s_settings_are_kept_too() {
    if !enforced() {
        return;
    }
    let dir = scratch("own-settings-chain");
    let dotfiles = dir.join("home/dotfiles");
    std::fs::create_dir_all(&dotfiles).unwrap();
    let real = dotfiles.join("real");
    std::fs::write(&real, "[theirs]\n").unwrap();
    std::os::unix::fs::symlink(&real, dotfiles.join("link")).unwrap();
    std::os::unix::fs::symlink(dotfiles.join("link"), dir.join("home/.gitconfig")).unwrap();
    let project = project(&dir, &dotfiles);
    if cfg!(target_os = "linux") {
        // A link can't be a mount point, so a command there is refused, saying why.
        let mut command = Command::new("echo done");
        command.sandbox = Some(project.sandbox.clone());
        let refused = block_on!(project.computers.run(
            project.workspace,
            &command,
            &Collect::default(),
            &CancelSwitch::default()
        ));
        assert!(
            refused.is_err_and(|error| error.message().contains("symbolic link")),
            "a command ran where it could replace the link"
        );
        return;
    }
    let said = project.sandboxed("rm link ; echo PLANTED > link ; echo done");
    assert!(said.contains("done"), "{said}");
    assert!(!holds_planted(&dotfiles.join("link")), "{said}");
    assert!(dotfiles.join("link").is_symlink(), "{said}");
}

#[test]
fn a_link_to_git_s_settings_not_there_yet_keeps_where_they_would_be() {
    if !enforced() {
        return;
    }
    let dir = scratch("own-settings-dangling");
    let dotfiles = dir.join("home/dotfiles");
    std::fs::create_dir_all(&dotfiles).unwrap();
    std::fs::create_dir_all(dir.join("home/.config")).unwrap();
    std::os::unix::fs::symlink(dotfiles.join("git"), dir.join("home/.config/git")).unwrap();
    let project = project(&dir, &dotfiles);
    let said = project.sandboxed("mkdir -p git ; echo PLANTED > git/config ; echo done");
    assert!(said.contains("done"), "{said}");
    assert!(!holds_planted(&dotfiles.join("git/config")), "{said}");
}

#[test]
fn a_project_at_glave_s_folder_can_neither_read_nor_change_it() {
    if !enforced() {
        return;
    }
    let dir = scratch("own-settings-glave");
    let glave = dir.join("home/.glave");
    std::fs::create_dir_all(&glave).unwrap();
    std::fs::write(glave.join("config.toml"), "kept = true\n").unwrap();
    let project = project(&dir, &glave);
    let said =
        project.sandboxed("echo PLANTED >> config.toml ; echo PLANTED > mcp.json ; echo done");
    assert!(said.contains("done"), "{said}");
    assert!(!holds_planted(&glave.join("config.toml")), "{said}");
    assert!(!glave.join("mcp.json").exists(), "{said}");
    assert!(!project.sandboxed("cat config.toml").contains("kept = true"));
    assert!(project.tool_writes("mcp.json").is_err());
    assert!(!glave.join("mcp.json").exists());
}

/// glave's folder set elsewhere (`GLAVE_HOME`), under a name no protected path has.
#[test]
fn a_project_that_holds_glave_s_folder_can_neither_read_nor_change_it() {
    if !enforced() {
        return;
    }
    let dir = scratch("own-settings-above");
    let glave = dir.join("srv/state");
    std::fs::create_dir_all(&glave).unwrap();
    std::fs::write(glave.join("config.toml"), "kept = true\n").unwrap();
    let mut project = project(&dir, &dir.join("srv"));
    project.sandbox.glave_home = Some(glave.clone());
    let said = project.sandboxed(
        "echo PLANTED >> state/config.toml ; echo PLANTED > state/mcp.json ; \
         echo fine > mine.txt && echo done",
    );
    assert!(said.contains("done"), "{said}");
    assert!(!holds_planted(&glave.join("config.toml")), "{said}");
    assert!(!glave.join("mcp.json").exists(), "{said}");
    assert!(
        !project
            .sandboxed("cat state/config.toml")
            .contains("kept = true")
    );
    assert!(project.tool_writes("state/config.toml").is_err());
    assert!(!holds_planted(&glave.join("config.toml")));
}

#[test]
fn a_project_inside_git_s_settings_is_read_only_whole() {
    if !enforced() {
        return;
    }
    let dir = scratch("own-settings-inside");
    let hooks = dir.join("home/.config/git/hooks");
    let project = project(&dir, &hooks);
    let said = project.sandboxed("echo PLANTED > pre-commit ; echo done");
    assert!(said.contains("done"), "{said}");
    assert!(!hooks.join("pre-commit").exists(), "{said}");
    assert!(project.tool_writes("pre-commit").is_err());
}

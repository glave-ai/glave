//! `--on cloud`: each new thread on a worker of its own (`specs/computers.md`). The worker is
//! made from the project's environment on the backend `[workers]` in `config.toml` names,
//! within the organization's `[workers]` limits, with this repository cloned in at the current
//! commit (which must be pushed, so the worker can fetch it). When the session ends, every
//! worker is released: files it handed back are saved under GLAVE_HOME/out/<thread>, unpushed
//! work goes to a glave/wip/ branch, and the worker is destroyed.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, bail};
use glave_computer::{
    Credentials, Docker, DockerSettings, Kubernetes, KubernetesSettings, WorkerComputers,
    https_url, read_environment,
};
use glave_harness::Workers;
use glave_ports::Sources;
use glave_session::OnWorkers;

use crate::client::Glave;
use crate::home;
use crate::policy::{Backend, Choices, Constraints, On, WorkersFile};

/// Threads on workers, when `on` is `cloud`, or only tasks' threads when `tasks` and a backend
/// is set up: what the session server needs, and the workers themselves, to remove any left at
/// the end. A task host that cannot make workers says why and runs its tasks here.
pub async fn workers(
    choices: &Choices,
    constraints: &Constraints,
    folder: &Path,
    tasks: bool,
) -> anyhow::Result<(Option<OnWorkers>, Option<Arc<WorkerComputers>>)> {
    if choices.on.value != On::Cloud {
        if !tasks || choices.workers.is_none() {
            return Ok((None, None));
        }
        return match every_or_tasks(choices, constraints, folder, false).await {
            Ok(made) => Ok(made),
            Err(error) => {
                eprintln!("glave: tasks run here, not on workers: {error:#}");
                Ok((None, None))
            }
        };
    }
    every_or_tasks(choices, constraints, folder, true).await
}

/// Workers for every thread, or for tasks' threads only.
async fn every_or_tasks(
    choices: &Choices,
    constraints: &Constraints,
    folder: &Path,
    every_thread: bool,
) -> anyhow::Result<(Option<OnWorkers>, Option<Arc<WorkerComputers>>)> {
    let Some(backend) = &choices.workers else {
        bail!(
            "--on cloud (from {}) needs a worker backend: set [workers] backend = \"docker\" (or \
             \"kubernetes\") and glave = \"<a Linux build of glave>\" in config.toml",
            choices.on.from
        );
    };
    let found = read_environment(folder).await?.with_context(|| {
        format!(
            "--on cloud needs an environment for the worker: {} has no \
             .devcontainer/devcontainer.json or .glave/environment.toml naming an image",
            folder.display()
        )
    })?;
    let sources = sources(folder).await?;
    let computers = Arc::new(computers(&backend.value, constraints).await?);
    let on = OnWorkers {
        workers: Workers {
            computers: computers.clone(),
            sources,
            attachments: Vec::new(),
        },
        environment: found.environment,
        every_thread,
    };
    Ok((Some(on), Some(computers)))
}

/// Workers on the backend `file` names, within the constraints' limits.
async fn computers(
    file: &WorkersFile,
    constraints: &Constraints,
) -> anyhow::Result<WorkerComputers> {
    if let Some(key) = file.misplaced() {
        bail!(
            "[workers] {key} is not used by the {} backend; remove it from config.toml",
            file.backend.name()
        );
    }
    let limits = constraints.worker_limits();
    Ok(match file.backend {
        Backend::Docker => {
            let docker = Docker::new(
                DockerSettings {
                    socket: file.socket.clone(),
                    glave: file.glave.clone(),
                },
                std::env::var_os("HOME").map(PathBuf::from).as_deref(),
            );
            WorkerComputers::docker(docker, limits, Credentials::HostGit)
        }
        Backend::Kubernetes => {
            let kubernetes = Kubernetes::connect(KubernetesSettings {
                kubeconfig: file.kubeconfig.clone(),
                namespace: file.namespace.clone(),
                runtime_class: file.runtime_class.clone(),
                glave: file.glave.clone(),
            })
            .await?;
            WorkerComputers::kubernetes(kubernetes, limits, Credentials::HostGit)
        }
    })
}

/// Runs git in `folder`; its output, or an error saying what it said.
async fn git(folder: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(folder)
        .output()
        .await
        .context("could not run git")?;
    if !output.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// This repository as a worker clones it: its `origin` over HTTPS, at the current commit,
/// which must already be on a remote.
async fn sources(folder: &Path) -> anyhow::Result<Sources> {
    let url = git(folder, &["remote", "get-url", "origin"])
        .await
        .context("--on cloud clones this repository's origin, and it has none")?;
    let commit = git(folder, &["rev-parse", "HEAD"]).await?;
    let unpushed = git(folder, &["rev-list", "HEAD", "--not", "--remotes"]).await?;
    if !unpushed.is_empty() {
        bail!(
            "the current commit {commit} is not pushed, so a worker cannot fetch it: push it \
             first"
        );
    }
    Ok(Sources::Git {
        url: https_url(&url),
        at: Some(commit),
    })
}

/// Where a handed-back file goes: under `folder`, never outside it.
fn inside(folder: &Path, name: &str) -> Option<PathBuf> {
    let relative = Path::new(name);
    relative
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
        .then(|| folder.join(relative))
}

/// Releases every worker the session's threads work on, says what each left, and removes any
/// worker still running.
pub async fn release(server: &Glave, workers: &WorkerComputers) {
    for (thread, released) in server.release_workers().await {
        match released {
            Ok(left) => {
                if let Some(branch) = &left.branch {
                    eprintln!("glave: the worker's unpushed work is on the branch {branch}");
                }
                if let Some(lost) = &left.lost {
                    eprintln!("glave: work on the worker of {thread} could not be kept: {lost}");
                }
                if !left.files.is_empty() {
                    match save(&thread.to_string(), &left.files).await {
                        Ok(folder) => eprintln!(
                            "glave: {} file(s) the worker handed back are in {}",
                            left.files.len(),
                            folder.display()
                        ),
                        Err(error) => eprintln!("glave: could not keep the files: {error:#}"),
                    }
                }
                eprintln!("glave: the worker for thread {thread} is released");
            }
            Err(error) => eprintln!("glave: could not release the worker of {thread}: {error}"),
        }
    }
    if let Err(error) = workers.destroy_all().await {
        eprintln!("glave: could not remove a worker: {error}");
    }
}

/// Saves handed-back files under GLAVE_HOME/out/<thread>.
async fn save(thread: &str, files: &[(String, glave_ports::Bytes)]) -> anyhow::Result<PathBuf> {
    let folder = home::folder()?.join("out").join(thread);
    for (name, content) in files {
        let Some(path) = inside(&folder, name) else {
            continue;
        };
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&path, content.as_slice())
            .await
            .with_context(|| format!("could not write {}", path.display()))?;
    }
    Ok(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_cloud_without_a_backend_or_an_environment_says_what_is_missing() {
        use crate::policy::{Layer, Level, assemble};
        let dir = std::env::temp_dir().join(format!("glave-cloud-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let chosen = |text: &str| {
            let layer = Layer {
                level: Level::File(dir.join("config.toml")),
                values: crate::policy::config(text, &dir).unwrap(),
            };
            assemble(&[layer], &[], None).choices
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let constraints = Constraints::default();
        let here = runtime
            .block_on(workers(&chosen(""), &constraints, &dir, false))
            .unwrap();
        assert!(here.0.is_none() && here.1.is_none(), "here by default");
        let no_backend = runtime
            .block_on(workers(
                &chosen("on = \"cloud\"\n"),
                &constraints,
                &dir,
                false,
            ))
            .unwrap_err();
        assert!(
            no_backend.to_string().contains("[workers] backend"),
            "{no_backend}"
        );
        let backend = "on = \"cloud\"\n[workers]\nbackend = \"docker\"\nglave = \"/x\"\n";
        let no_environment = runtime
            .block_on(workers(&chosen(backend), &constraints, &dir, false))
            .unwrap_err();
        assert!(
            no_environment.to_string().contains("devcontainer.json"),
            "{no_environment}"
        );
        let docker_with_a_runtime_class = WorkersFile {
            backend: Backend::Docker,
            glave: "/x".into(),
            socket: None,
            kubeconfig: None,
            namespace: None,
            runtime_class: Some("gvisor".into()),
        };
        let refused = runtime
            .block_on(computers(&docker_with_a_runtime_class, &constraints))
            .unwrap_err();
        assert!(
            refused
                .to_string()
                .contains("runtime_class is not used by the docker"),
            "{refused}"
        );
    }

    #[test]
    fn handed_back_files_stay_in_their_folder() {
        let folder = Path::new("/home/me/.glave/out/t");
        assert_eq!(inside(folder, "report.md"), Some(folder.join("report.md")));
        assert_eq!(inside(folder, "a/b.txt"), Some(folder.join("a/b.txt")));
        assert_eq!(inside(folder, "../escape"), None);
        assert_eq!(inside(folder, "/etc/passwd"), None);
    }
}

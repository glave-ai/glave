//! `--on cloud`: each new thread on a worker of its own (`specs/computers.md`). The worker is
//! made from the project's environment on the backend `[workers]` in `config.toml` names,
//! within the organization's `[workers]` limits, with this repository cloned in at the current
//! commit (which must be pushed, so the worker can fetch it). When the session ends, every
//! worker is released: files it handed back are saved under `GLAVE_HOME/out/<thread>`, unpushed
//! work goes to a glave/wip/ branch, and the worker is destroyed.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, bail};
use glave_computer::{
    Docker, DockerSettings, Kubernetes, KubernetesSettings, People, WorkerComputers,
    WorkerCredentials, https_url, read_environment,
};
use glave_event::PrincipalId;
use glave_harness::{Homes, Workers};
use glave_ports::computer::{ComputerError, Home, Sources};
use glave_ports::{BoxFuture, OrgId, ProjectId};
use glave_process::Processes;
use glave_session::OnWorkers;

use crate::client::Glave;
use crate::home;
use glave_config::policy::{Backend, Choices, ComputersFile, Constraints, On, WorkersFile};

/// Where threads work: each person's on their own computer where the server's settings give
/// people one ([`homes`]); and the threads of `project`, the project of `folder`, on workers,
/// when its settings say `on` is `cloud`, git asked about the repository and for credentials
/// as one of `processes`: what the session server needs, and the workers themselves, to remove
/// any left at the end. Tasks too work on workers only then; otherwise on a worktree here,
/// like any thread asking for a copy.
pub(crate) async fn workers(
    choices: &Choices,
    constraints: &Constraints,
    folder: &Path,
    project: ProjectId,
    processes: &Processes,
) -> anyhow::Result<(
    Option<OnWorkers>,
    Option<Arc<WorkerComputers>>,
    Option<Arc<dyn Homes>>,
)> {
    let homes = homes(choices, constraints).await?;
    if choices.on.value != On::Cloud {
        return Ok((None, None, homes));
    }
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
    let sources = sources(processes, folder).await?;
    let computers = Arc::new(computers(&backend.value, constraints, processes).await?);
    let on = OnWorkers {
        workers: Workers {
            computers: Arc::<WorkerComputers>::clone(&computers),
            sources,
            attachments: Vec::new(),
        },
        environment: found.environment,
        project,
    };
    Ok((Some(on), Some(computers), homes))
}

/// Workers on the backend `file` names, within the constraints' limits, once those a glave
/// that is gone left there are removed.
async fn computers(
    file: &WorkersFile,
    constraints: &Constraints,
    processes: &Processes,
) -> anyhow::Result<WorkerComputers> {
    if let Some(key) = file.misplaced() {
        bail!(
            "[workers] {key} is not used by the {} backend; remove it from config.toml",
            file.backend.name()
        );
    }
    let limits = constraints.worker_limits();
    let computers = match file.backend {
        Backend::Docker => {
            let docker = Docker::new(
                DockerSettings {
                    socket: file.socket.clone(),
                    glave: file.glave.clone(),
                },
                std::env::var_os("HOME").map(PathBuf::from).as_deref(),
            );
            WorkerComputers::docker(
                docker,
                limits,
                WorkerCredentials::HostGit(processes.clone()),
            )
        }
        Backend::Kubernetes => {
            let kubernetes = Kubernetes::connect(KubernetesSettings {
                kubeconfig: file.kubeconfig.clone(),
                namespace: file.namespace.clone(),
                runtime_class: file.runtime_class.clone(),
                glave: file.glave.clone(),
            })
            .await?;
            let credentials = WorkerCredentials::HostGit(processes.clone());
            WorkerComputers::kubernetes(kubernetes, limits, credentials)
        }
    };
    // As glave starts: what a glave that ended without removing its workers left.
    match computers.reap().await {
        Ok(0) => {}
        Ok(removed) => eprintln!("glave: removed {removed} workers a glave that ended had left"),
        Err(error) => eprintln!(
            "glave: could not look for workers a glave that ended left: {}",
            error.message()
        ),
    }
    Ok(computers)
}

/// Each person's own computer, where the server's settings give people one (`[computers]`,
/// on the Kubernetes cluster `[workers]` reaches), within the organization's `[workers]`
/// limits; none otherwise, and threads work on the server's own computer, as on a laptop.
async fn homes(
    choices: &Choices,
    constraints: &Constraints,
) -> anyhow::Result<Option<Arc<dyn Homes>>> {
    let Some(computers) = &choices.computers else {
        return Ok(None);
    };
    let cluster = match &choices.workers {
        Some(workers) if workers.value.backend == Backend::Kubernetes => &workers.value,
        _ => bail!(
            "[computers] (from {}) puts people's computers on the Kubernetes cluster [workers] \
             reaches: set [workers] backend = \"kubernetes\" and glave = \"<a Linux build of \
             glave>\" in config.toml",
            computers.from
        ),
    };
    if let Some(key) = cluster.misplaced() {
        bail!("[workers] {key} is not used by the kubernetes backend; remove it from config.toml");
    }
    let kubernetes = Kubernetes::connect(KubernetesSettings {
        kubeconfig: cluster.kubeconfig.clone(),
        namespace: cluster.namespace.clone(),
        runtime_class: cluster.runtime_class.clone(),
        glave: cluster.glave.clone(),
    })
    .await?;
    let disk_gb = computers.value.disk_gb.unwrap_or(ComputersFile::DISK_GB);
    let limits = constraints.worker_limits();
    let people = People::new(kubernetes, &computers.value.image, &limits, disk_gb)?;
    Ok(Some(Arc::new(OnCluster(people))))
}

/// People's computers on a cluster, as the harness asks for them: each named by the person's
/// organization and id.
struct OnCluster(People);

impl Homes for OnCluster {
    fn wake(&self, org: OrgId, person: PrincipalId) -> Result<(), ComputerError> {
        self.0.wake(&People::key(org.as_uuid(), person.as_uuid()))
    }

    fn home(&self, org: OrgId, person: PrincipalId) -> BoxFuture<'_, Result<Home, ComputerError>> {
        Box::pin(async move {
            self.0
                .home(&People::key(org.as_uuid(), person.as_uuid()))
                .await
        })
    }
}

/// How long git has to read this repository, which it does in a moment.
const GIT_ANSWERS_WITHIN: std::time::Duration = std::time::Duration::from_secs(30);

/// Runs git in `folder`; its output, or an error saying what it said.
async fn git(processes: &Processes, folder: &Path, args: &[&str]) -> anyhow::Result<String> {
    let mut git = glave_process::Program::new("git");
    git.args(args).current_dir(folder);
    let output = processes
        .output(&mut git, GIT_ANSWERS_WITHIN)
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
async fn sources(processes: &Processes, folder: &Path) -> anyhow::Result<Sources> {
    let url = git(processes, folder, &["remote", "get-url", "origin"])
        .await
        .context("--on cloud clones this repository's origin, and it has none")?;
    let commit = git(processes, folder, &["rev-parse", "HEAD"]).await?;
    let unpushed = git(
        processes,
        folder,
        &["rev-list", "HEAD", "--not", "--remotes"],
    )
    .await?;
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
pub(crate) async fn release(server: &Glave, workers: &WorkerComputers) {
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

/// Saves handed-back files under `GLAVE_HOME/out/<thread>`.
async fn save(
    thread: &str,
    files: &[(String, glave_ports::computer::Bytes)],
) -> anyhow::Result<PathBuf> {
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

    /// Where threads work, without the people's computers, which have no `Debug`.
    async fn placed(
        choices: &Choices,
        constraints: &Constraints,
        folder: &Path,
        project: ProjectId,
        processes: &Processes,
    ) -> anyhow::Result<(Option<OnWorkers>, Option<Arc<WorkerComputers>>)> {
        let (on, workers, _) = workers(choices, constraints, folder, project, processes).await?;
        Ok((on, workers))
    }

    #[test]
    fn on_cloud_without_a_backend_or_an_environment_says_what_is_missing() {
        use glave_config::policy::{Layer, Level, assemble};
        let dir = glave_ports::testing::TempDir::new("cloud").unwrap();
        let chosen = |text: &str| {
            let layer = Layer {
                level: Level::File(dir.join("config.toml")),
                values: glave_config::policy::config(text, &dir).unwrap(),
            };
            assemble(&[layer], &[], None).choices
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let constraints = Constraints::default();
        let project = ProjectId::from_uuid(uuid::Uuid::from_u128(1));
        let processes = Processes::unrecorded();
        let here = runtime
            .block_on(workers(
                &chosen(""),
                &constraints,
                &dir,
                project,
                &processes,
            ))
            .unwrap();
        assert!(
            here.0.is_none() && here.1.is_none() && here.2.is_none(),
            "here by default"
        );
        let no_backend = runtime
            .block_on(placed(
                &chosen("on = \"cloud\"\n"),
                &constraints,
                &dir,
                project,
                &processes,
            ))
            .unwrap_err();
        assert!(
            no_backend.to_string().contains("[workers] backend"),
            "{no_backend}"
        );
        let backend = "on = \"cloud\"\n[workers]\nbackend = \"docker\"\nglave = \"/x\"\n";
        let no_environment = runtime
            .block_on(placed(
                &chosen(backend),
                &constraints,
                &dir,
                project,
                &processes,
            ))
            .unwrap_err();
        assert!(
            no_environment.to_string().contains("devcontainer.json"),
            "{no_environment}"
        );
        let people = format!("[computers]\nimage = \"x@sha256:{}\"\n", "0".repeat(64));
        let no_cluster = runtime
            .block_on(placed(
                &chosen(&people),
                &constraints,
                &dir,
                project,
                &processes,
            ))
            .unwrap_err();
        assert!(
            no_cluster.to_string().contains("backend = \"kubernetes\""),
            "{no_cluster}"
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
            .block_on(computers(
                &docker_with_a_runtime_class,
                &constraints,
                &processes,
            ))
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

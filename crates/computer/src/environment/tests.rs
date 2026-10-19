use std::collections::BTreeMap;
use std::fmt::Write as _;

use glave_ports::computer::{Image, Network, Size};
use proptest::prelude::*;
use serde_json::json;

use super::*;

#[test]
fn the_spec_example_reads() {
    let text = r#"
        image = "ghcr.io/acme/rust:1.99"     # an OCI image; "host" for the local machine
        setup = "scripts/setup.sh"
        cpus = 2
        memory_gb = 8
        disk_gb = 50
        network = "allowlist"
        allow = ["crates.io", "github.com"]
        secrets = ["GITHUB_TOKEN"]
        idle_minutes = 30
    "#;
    let environment = parse_environment_toml(text).unwrap();
    assert_eq!(
        environment.image,
        Image::Oci("ghcr.io/acme/rust:1.99".into())
    );
    assert_eq!(environment.setup, ["scripts/setup.sh"]);
    assert_eq!(
        environment.size,
        Size {
            cpus: Some(2),
            memory_gb: Some(8),
            disk_gb: Some(50)
        }
    );
    assert_eq!(environment.network, Some(Network::Allowlist));
    assert_eq!(environment.secrets, ["GITHUB_TOKEN"]);
    assert_eq!(environment.idle_minutes, Some(30));
}

#[test]
fn mistakes_in_environment_toml_are_refused() {
    for wrong in [
        "imgae = \"host\"",
        "image = \"\"",
        "network = \"sometimes\"",
        "allow = [\"crates.io\"]",
        "secrets = [\"not a name\"]",
        "secrets = [\"TOKEN\"]\nenv = { TOKEN = \"x\" }",
        "setup = \"  \"",
        "cpus = -1",
    ] {
        assert!(parse_environment_toml(wrong).is_err(), "{wrong}");
    }
    assert_eq!(parse_environment_toml("").unwrap(), Environment::host());
}

#[test]
fn a_devcontainer_reads_what_glave_honours() {
    let text = r#"{
        // A typical Rust devcontainer.
        "name": "rust",
        "image": "mcr.microsoft.com/devcontainers/rust:1",
        "features": {},
        "containerEnv": { "RUST_LOG": "info", "MODE": "a" },
        "remoteEnv": { "MODE": "b" },
        "onCreateCommand": "rustup component add clippy",
        "postCreateCommand": ["cargo", "fetch", "--locked", "it's"],
        "updateContentCommand": { "b": "echo two", "a": "echo one" },
        "hostRequirements": { "cpus": 4, "memory": "8gb", "storage": "1500mb" },
        "customizations": { "vscode": { "extensions": ["rust-lang.rust-analyzer"] } },
        "forwardPorts": [3000],
    }"#;
    let (environment, ignored) = parse_devcontainer(text).unwrap();
    assert_eq!(
        environment.image,
        Image::Oci("mcr.microsoft.com/devcontainers/rust:1".into())
    );
    assert_eq!(
        environment.setup,
        [
            "rustup component add clippy",
            "echo one",
            "echo two",
            r"cargo fetch --locked 'it'\''s'",
        ]
    );
    assert_eq!(environment.env.get("MODE").map(String::as_str), Some("b"));
    assert_eq!(
        environment.size,
        Size {
            cpus: Some(4),
            memory_gb: Some(8),
            disk_gb: Some(2)
        }
    );
    assert_eq!(ignored, ["customizations", "forwardPorts", "name"]);
}

#[test]
fn what_glave_cannot_honour_is_refused_by_name() {
    let base = json!({"image": "x"});
    for (key, value) in [
        ("build", json!({"dockerfile": "Dockerfile"})),
        (
            "features",
            json!({"ghcr.io/devcontainers/features/node:1": {}}),
        ),
        ("initializeCommand", json!("rm -rf ~")),
        ("mounts", json!(["source=/,target=/host,type=bind"])),
        ("privileged", json!(true)),
        ("postStartCommand", json!("npm start")),
        ("somethingNew", json!(1)),
        ("containerEnv", json!({"HOME_COPY": "${localEnv:HOME}"})),
        ("hostRequirements", json!({"gpu": true})),
    ] {
        let mut file = base.clone();
        file[key] = value;
        let error = parse_devcontainer(&file.to_string()).unwrap_err();
        assert!(
            matches!(error, EnvironmentError::Unsupported { .. }),
            "{key}: {error}"
        );
    }
    assert!(parse_devcontainer(r#"{"name": "no image"}"#).is_err());
    assert!(parse_devcontainer("[1]").is_err());
}

#[test]
fn devcontainer_json_wins_over_environment_toml() {
    let project = glave_ports::testing::TempDir::new("env-read").unwrap();
    std::fs::create_dir_all(project.join(".glave")).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert_eq!(runtime.block_on(read_environment(&project)).unwrap(), None);
    std::fs::write(project.join(".glave/environment.toml"), "cpus = 2").unwrap();
    let found = runtime
        .block_on(read_environment(&project))
        .unwrap()
        .unwrap();
    assert_eq!(found.environment.size.cpus, Some(2));
    std::fs::write(project.join(".devcontainer.json"), r#"{"image": "rust:1"}"#).unwrap();
    let found = runtime
        .block_on(read_environment(&project))
        .unwrap()
        .unwrap();
    assert_eq!(found.environment.image, Image::Oci("rust:1".into()));
    std::fs::write(project.join(".devcontainer.json"), r#"{"build": {}}"#).unwrap();
    let error = runtime.block_on(read_environment(&project)).unwrap_err();
    assert!(
        error.to_string().starts_with(".devcontainer.json: build"),
        "{error}"
    );
}

fn name() -> impl Strategy<Value = String> {
    "[A-Z_][A-Z0-9_]{0,8}"
}

fn text() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 ./:_-]{0,16}".prop_filter("not blank", |text| !text.trim().is_empty())
}

prop_compose! {
    fn environment()(
        image in prop_oneof![Just(Image::Host), "[a-z]{1,8}(/[a-z]{1,8})?:[0-9]{1,2}".prop_map(Image::Oci)],
        setup in proptest::collection::vec(text(), 0..3),
        cpus in proptest::option::of(1u32..64),
        memory_gb in proptest::option::of(1u32..512),
        disk_gb in proptest::option::of(1u32..4096),
        network in proptest::option::of(prop_oneof![Just(Network::Off), Just(Network::Allowlist), Just(Network::On)]),
        allow in proptest::collection::vec("[a-z]{1,8}\\.[a-z]{2,3}", 0..3),
        secrets in proptest::collection::btree_set(name(), 0..3),
        idle_minutes in proptest::option::of(1u32..1000),
        env in proptest::collection::btree_map(name(), text(), 0..3),
    ) -> Environment {
        let allow = if network == Some(Network::Allowlist) { allow } else { Vec::new() };
        let env: BTreeMap<_, _> = env.into_iter().filter(|(k, _)| !secrets.contains(k)).collect();
        Environment {
            image, setup, size: Size { cpus, memory_gb, disk_gb }, network, allow,
            secrets: secrets.into_iter().collect(), idle_minutes, env,
        }
    }
}

/// The environment as someone would write it in environment.toml.
fn as_toml(environment: &Environment) -> String {
    let quote = |text: &str| serde_json::to_string(text).unwrap();
    let list = |items: &[String]| {
        format!(
            "[{}]",
            items
                .iter()
                .map(|item| quote(item))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let mut text = format!("image = {}\n", quote(&environment.image.to_string()));
    let _ = writeln!(text, "setup = {}", list(&environment.setup));
    for (key, value) in [
        ("cpus", environment.size.cpus),
        ("memory_gb", environment.size.memory_gb),
        ("disk_gb", environment.size.disk_gb),
        ("idle_minutes", environment.idle_minutes),
    ] {
        if let Some(value) = value {
            let _ = writeln!(text, "{key} = {value}  # a comment");
        }
    }
    if let Some(network) = environment.network {
        let _ = writeln!(
            text,
            "network = {}",
            serde_json::to_string(&network).unwrap()
        );
    }
    let _ = writeln!(text, "allow = {}", list(&environment.allow));
    let _ = writeln!(text, "secrets = {}", list(&environment.secrets));
    text.push_str("[env]\n");
    for (name, value) in &environment.env {
        let _ = writeln!(text, "{name} = {}", quote(value));
    }
    text
}

proptest! {
    #[test]
    fn environment_toml_reads_back_what_was_written(environment in environment()) {
        prop_assert_eq!(parse_environment_toml(&as_toml(&environment)).unwrap(), environment);
    }

    #[test]
    fn devcontainers_read_back_what_was_written(environment in environment()) {
        let Image::Oci(image) = &environment.image else { return Ok(()); };
        let mut file = json!({"image": image, "containerEnv": environment.env});
        if !environment.setup.is_empty() {
            file["postCreateCommand"] = json!(environment.setup.join(" && "));
        }
        if let Some(cpus) = environment.size.cpus {
            file["hostRequirements"] = json!({"cpus": cpus});
        }
        let text = format!("// written by a test\n{}", serde_json::to_string_pretty(&file).unwrap());
        let (read, ignored) = parse_devcontainer(&text).unwrap();
        prop_assert!(ignored.is_empty());
        prop_assert_eq!(read.image, environment.image.clone());
        prop_assert_eq!(read.env, environment.env.clone());
        prop_assert_eq!(read.size.cpus, environment.size.cpus);
        let setup: Vec<String> = if environment.setup.is_empty() { Vec::new() } else { vec![environment.setup.join(" && ")] };
        prop_assert_eq!(read.setup, setup);
    }

    #[test]
    fn any_text_is_read_or_refused_without_crashing(text in "\\PC{0,120}") {
        let _ = parse_environment_toml(&text);
        let _ = parse_devcontainer(&text);
    }

    #[test]
    fn sizes_round_up_to_whole_gigabytes(megabytes in 1u64..10_000_000) {
        let read = devcontainer::gigabytes("memory", &json!(format!("{megabytes}MB"))).unwrap();
        prop_assert_eq!(u64::from(read), megabytes.div_ceil(1024));
    }
}

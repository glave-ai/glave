//! `cargo xtask deps`: the dependency rule (AGENTS.md, "Dependency rule"), checked against what
//! `cargo metadata` says. Every workspace crate has a layer in [`LAYERS`], and may depend only
//! on crates in the layers its own may use. Only normal dependencies count: dev-dependencies
//! are for tests, which may use adapters. An edge that crosses the rule on purpose is on
//! [`ALLOWED`], with its reason; one that no longer exists, or no longer crosses, fails until it
//! is taken off.

use serde_json::Value;

/// Where a crate sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layer {
    Types,
    Ports,
    Domain,
    Adapter,
    /// The binary, and the crates it is wired from.
    Binary,
    /// Developer tools.
    Tools,
}

use Layer::{Adapter, Binary, Domain, Ports, Tools, Types};

/// Every workspace crate's layer, each with why it sits there.
const LAYERS: &[(&str, Layer)] = &[
    // The versioned event format every crate reads and writes.
    ("glave-event", Types),
    // The interfaces the domain depends on, with in-memory versions and conformance suites.
    ("glave-ports", Ports),
    // The agent loop.
    ("glave-harness", Domain),
    // Routing, fallback, budgets and metering for every model call.
    ("glave-gateway", Domain),
    // The key hierarchy: content sealed at rest, and erased by destroying its keys.
    ("glave-keys", Domain),
    // Shell commands read and judged before they run.
    ("glave-shell", Domain),
    // Every operating-system process glave starts, owned and recorded, over tokio and rustix.
    ("glave-process", Adapter),
    // The computer port: this machine, and workers on Docker or Kubernetes.
    ("glave-computer", Adapter),
    // The verified caller: OIDC (jsonwebtoken) and local sign-in.
    ("glave-identity", Adapter),
    // MCP servers as a tool source, through rmcp.
    ("glave-mcp", Adapter),
    // The memory port: glave's own MCP server over the event log.
    ("glave-memory", Adapter),
    // web_fetch as a tool source, over reqwest.
    ("glave-web", Adapter),
    // The model port over OpenAI-compatible APIs.
    ("glave-provider-openai", Adapter),
    // The model port over Anthropic's Messages API.
    ("glave-provider-anthropic", Adapter),
    // Azure OpenAI's endpoint, and Entra tokens.
    ("glave-azure", Adapter),
    // Amazon Bedrock's endpoints, signed with SigV4.
    ("glave-aws", Adapter),
    // Vertex AI's endpoints, and Google's credentials.
    ("glave-google", Adapter),
    // The event log, blob store and key storage on SQLite.
    ("glave-store-sqlite", Adapter),
    // The event log, blob store and key storage on Postgres.
    ("glave-store-postgres", Adapter),
    // Config and policy files made into the settings the domain crates read, which live in the
    // ports crate.
    ("glave-config", Domain),
    // The session API: the harness served in-process, over stdio and over HTTP. It serves the
    // harness and verifies callers with glave-identity, so it is wired from both, as the binary is.
    ("glave-session", Binary),
    // The gateway served over HTTP, with sign-in.
    ("glave-gateway-service", Binary),
    // The command-line binary, which wires everything.
    ("glave", Binary),
    // The harness run on the eval tasks against real models.
    ("glave-eval", Tools),
    // Developer automation.
    ("xtask", Tools),
];

/// Normal dependencies that cross the rule on purpose: the crate, what it depends on, and why.
const ALLOWED: &[(&str, &str, &str)] = &[
    (
        "glave-harness",
        "glave-shell",
        "the harness judges every command with it before it runs; one judge, so concrete \
         rather than a port (a port needs two implementations)",
    ),
    (
        "glave-computer",
        "glave-process",
        "every process glave starts goes through one place that gives it an owner and records \
         it, so a crashed glave's commands are stopped at the next start (#361)",
    ),
    (
        "glave-mcp",
        "glave-process",
        "every process glave starts goes through one place that gives it an owner and records \
         it, so a crashed glave's MCP servers are stopped at the next start (#361)",
    ),
    (
        "glave-memory",
        "glave-mcp",
        "the memory server is reached as any MCP server is, through glave's own client",
    ),
    (
        "glave-store-sqlite",
        "glave-keys",
        "content is sealed with glave's keys before it is stored, and the keys kept wrapped \
         (specs/storage.md, Keys)",
    ),
    (
        "glave-store-postgres",
        "glave-keys",
        "content is sealed with glave's keys before it is stored, and the keys kept wrapped \
         (specs/storage.md, Keys)",
    ),
];

impl Layer {
    fn name(self) -> &'static str {
        match self {
            Types => "types",
            Ports => "ports",
            Domain => "domain",
            Adapter => "adapter",
            Binary => "binary",
            Tools => "tools",
        }
    }

    /// Whether a crate of this layer may depend on one of `other`.
    fn may_use(self, other: Self) -> bool {
        match self {
            Types => false,
            Ports => other == Types,
            Domain | Adapter => matches!(other, Types | Ports),
            Binary => other != Tools,
            Tools => true,
        }
    }

    /// The rule a crate of this layer keeps, as AGENTS.md words it.
    fn rule(self) -> &'static str {
        match self {
            Types => "shared types depend on nothing in glave",
            Ports => "ports depend only on types",
            Domain => {
                "domain crates depend only on types and ports, never on each other or on adapters"
            }
            Adapter => "adapters depend only on types, ports and their one external library",
            Binary => "nothing but tools depends on tools",
            Tools => "tools may depend on anything",
        }
    }
}

/// Fails when a crate depends on one its layer may not, naming both and the rule.
pub(crate) fn check() -> Result<(), String> {
    let metadata = crate::workspace::metadata()?;
    let problems = problems(&graph(&metadata)?, LAYERS, ALLOWED);
    if problems.is_empty() {
        println!(
            "xtask: {} crates keep the dependency rule ({} allowed exceptions)",
            LAYERS.len(),
            ALLOWED.len()
        );
        Ok(())
    } else {
        Err(format!(
            "{} break(s) of the dependency rule; each crate's layer and the allowed exceptions \
             are in xtask/src/deps.rs:\n  {}",
            problems.len(),
            problems.join("\n  ")
        ))
    }
}

/// Each workspace crate in `cargo metadata`'s JSON, and the workspace crates it depends on as
/// normal dependencies: not dev-dependencies, nor build-dependencies.
fn graph(metadata: &Value) -> Result<Vec<(&str, Vec<&str>)>, String> {
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("`cargo metadata` named no packages")?;
    let names = packages
        .iter()
        .map(|package| {
            package
                .get("name")
                .and_then(Value::as_str)
                .ok_or("`cargo metadata` named a package without its name")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut graph = Vec::new();
    for (package, name) in packages.iter().zip(&names) {
        let dependencies = package
            .get("dependencies")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("`cargo metadata` gave {name} no dependencies"))?;
        let mut on = Vec::new();
        for dependency in dependencies {
            let Some(dependency_name) = dependency.get("name").and_then(Value::as_str) else {
                return Err(format!(
                    "`cargo metadata` gave {name} a dependency without a name"
                ));
            };
            let normal = dependency.get("kind").is_none_or(Value::is_null);
            if normal && names.contains(&dependency_name) && !on.contains(&dependency_name) {
                on.push(dependency_name);
            }
        }
        graph.push((*name, on));
    }
    Ok(graph)
}

/// What in `graph` breaks the rule, given each crate's layer and the edges allowed across it.
fn problems(
    graph: &[(&str, Vec<&str>)],
    layers: &[(&str, Layer)],
    allowed: &[(&str, &str, &str)],
) -> Vec<String> {
    let layer_of = |name: &str| {
        layers
            .iter()
            .find(|(crate_name, _)| *crate_name == name)
            .map(|(_, layer)| *layer)
    };
    let mut problems = Vec::new();
    for (name, _) in layers {
        if !graph.iter().any(|(crate_name, _)| crate_name == name) {
            problems.push(format!(
                "{name} has a layer but is not in the workspace: take it off LAYERS"
            ));
        }
    }
    for (from, on) in graph {
        let Some(from_layer) = layer_of(from) else {
            problems.push(format!(
                "{from} has no layer: add it to LAYERS, with why it sits there"
            ));
            continue;
        };
        for to in on {
            let crosses = layer_of(to).filter(|to_layer| !from_layer.may_use(*to_layer));
            if let Some(to_layer) = crosses
                && !allowed.iter().any(|(f, t, _)| f == from && t == to)
            {
                problems.push(format!(
                    "{from} ({}) depends on {to} ({}): {}",
                    from_layer.name(),
                    to_layer.name(),
                    from_layer.rule()
                ));
            }
        }
    }
    for (from, to, _) in allowed {
        let depends = graph
            .iter()
            .any(|(name, on)| name == from && on.contains(to));
        if !depends {
            problems.push(format!(
                "{from} no longer depends on {to}: take it off ALLOWED"
            ));
        } else if let (Some(from_layer), Some(to_layer)) = (layer_of(from), layer_of(to))
            && from_layer.may_use(to_layer)
        {
            problems.push(format!(
                "{from} may depend on {to} without an exception: take it off ALLOWED"
            ));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A crate and its dependencies, each with its kind: none for a normal one.
    type Crate<'a> = (&'a str, &'a [(&'a str, Option<&'a str>)]);

    const MADE_UP: &[(&str, Layer)] = &[
        ("event", Types),
        ("ports", Ports),
        ("harness", Domain),
        ("keys", Domain),
        ("store", Adapter),
        ("cli", Binary),
        ("eval", Tools),
    ];

    /// Every layer depending on what it may, and tests and builds on what they like.
    const KEPT: &[Crate<'static>] = &[
        ("event", &[("serde", None)]),
        ("ports", &[("event", None)]),
        ("harness", &[("ports", None), ("store", Some("dev"))]),
        ("keys", &[("ports", None), ("cli", Some("build"))]),
        ("store", &[("ports", None), ("harness", Some("dev"))]),
        ("cli", &[("harness", None), ("store", None), ("keys", None)]),
        ("eval", &[("cli", None), ("store", None)]),
    ];

    /// `cargo metadata`'s JSON for `crates`.
    fn metadata(crates: &[Crate<'_>]) -> Value {
        let packages: Vec<Value> = crates
            .iter()
            .map(|(name, dependencies)| {
                let dependencies: Vec<Value> = dependencies
                    .iter()
                    .map(|(dependency, kind)| json!({"name": dependency, "kind": kind}))
                    .collect();
                json!({"name": name, "dependencies": dependencies})
            })
            .collect();
        json!({ "packages": packages })
    }

    fn problems_in(crates: &[Crate<'_>], allowed: &[(&str, &str, &str)]) -> Vec<String> {
        problems(&graph(&metadata(crates)).unwrap(), MADE_UP, allowed)
    }

    #[test]
    fn layers_that_keep_the_rule_pass_and_only_normal_dependencies_count() {
        assert_eq!(problems_in(KEPT, &[]), Vec::<String>::new());
        assert_eq!(
            graph(&metadata(KEPT)).unwrap().get(2),
            Some(&("harness", vec!["ports"]))
        );
        assert!(graph(&json!({})).is_err());
    }

    #[test]
    fn a_break_names_both_crates_and_the_rule() {
        let crates: &[Crate<'_>] = &[
            ("event", &[("ports", None)]),
            ("ports", &[("event", None), ("harness", None)]),
            ("harness", &[("keys", None), ("store", None)]),
            ("keys", &[]),
            ("store", &[("harness", None), ("eval", None)]),
            ("cli", &[("eval", None)]),
            ("eval", &[]),
        ];
        assert_eq!(
            problems_in(crates, &[]),
            [
                "event (types) depends on ports (ports): shared types depend on nothing in glave",
                "ports (ports) depends on harness (domain): ports depend only on types",
                "harness (domain) depends on keys (domain): domain crates depend only on types \
                 and ports, never on each other or on adapters",
                "harness (domain) depends on store (adapter): domain crates depend only on types \
                 and ports, never on each other or on adapters",
                "store (adapter) depends on harness (domain): adapters depend only on types, \
                 ports and their one external library",
                "store (adapter) depends on eval (tools): adapters depend only on types, ports \
                 and their one external library",
                "cli (binary) depends on eval (tools): nothing but tools depends on tools",
            ]
        );
    }

    #[test]
    fn every_crate_has_a_layer_and_every_layer_a_crate() {
        let crates: &[Crate<'_>] = &[
            ("event", &[]),
            ("ports", &[]),
            ("harness", &[("new", None)]),
            ("keys", &[]),
            ("store", &[]),
            ("new", &[]),
            ("eval", &[]),
        ];
        assert_eq!(
            problems_in(crates, &[]),
            [
                "cli has a layer but is not in the workspace: take it off LAYERS",
                "new has no layer: add it to LAYERS, with why it sits there",
            ]
        );
    }

    #[test]
    fn an_allowed_edge_passes_until_it_goes_or_no_longer_crosses() {
        let crates: &[Crate<'_>] = &[
            ("event", &[]),
            ("ports", &[]),
            ("harness", &[("keys", None)]),
            ("keys", &[]),
            ("store", &[("ports", None)]),
            ("cli", &[]),
            ("eval", &[]),
        ];
        let allowed = [
            ("harness", "keys", "on purpose"),
            ("store", "keys", "it went"),
            ("store", "ports", "it never crossed"),
        ];
        assert_eq!(
            problems_in(crates, &allowed),
            [
                "store no longer depends on keys: take it off ALLOWED",
                "store may depend on ports without an exception: take it off ALLOWED",
            ]
        );
    }
}

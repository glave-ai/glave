//! `glave gateway serve`: the gateway as an organization's service (`specs/gateway.md`,
//! "Surfaces"). Its models come from the environment, as glave's do (`GLAVE_BASE_URL`,
//! `GLAVE_API_KEY`, `GLAVE_MODEL`, `GLAVE_JUDGE_MODEL`), and its keys never leave it. Its
//! policy is the organization's `policy.toml`, kept for each caller: `[models]` on every call,
//! `[budget] person_per_day_usd` per person per day, and `[mcp] catalog` naming the servers of
//! its own `mcp.json` it offers at `/mcp`, each with the credentials only it holds.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context as _, bail};
use glave_event::{Currency, Money};
use glave_gateway::{Budgets, Limit};
use glave_gateway_service::{Policy, Service, Wiring};
use glave_mcp::{Limits, McpServer, ServerConfig};
use glave_ports::{Caller, OsEntropy, SystemClock};
use glave_session::signin::{SignIn, Trust};
use tokio::net::TcpListener;

use crate::config;
use crate::gateway::{Allowed, gateway};
use crate::model::Model;
use crate::{home, mcp};
use glave_config::policy::{self, Constraints, ORGANIZATION_POLICY, Source};

pub const USAGE: &str = "\
glave gateway: the gateway as a service, for everyone in an organization

Usage:
  glave gateway serve --http <address> --issuer <url> --audience <name> [--policy <file>]
      serve model calls (OpenAI Chat Completions at /v1/chat/completions, Anthropic Messages
      at /v1/messages) and the tool catalog (MCP at /mcp) to callers signed in with an OIDC
      access token from that issuer for that audience. The address is host:port, or a port
      on 127.0.0.1; the listener has no TLS, so put it behind a proxy that has. --issuer may
      be given more than once.

The models are the environment's, as for glave: GLAVE_BASE_URL, GLAVE_API_KEY, GLAVE_MODEL
(the main route) and GLAVE_JUDGE_MODEL (the judge route). The policy is --policy, or
/etc/glave/policy.toml: [models] is kept on every call, [budget] person_per_day_usd for each
person each day (UTC), and [mcp] catalog names the servers of mcp.json in GLAVE_HOME it offers
(every one when it names none), their ${VAR}s filled from the gateway's environment. Every call
is recorded, content-free, in the caller's stream in GLAVE_HOME's database.";

/// The project the gateway records calls in, apart from every project's.
const SCOPE: &str = "glave:gateway";

/// How to serve.
#[derive(Debug, PartialEq, Eq)]
struct Options {
    address: SocketAddr,
    issuers: Vec<String>,
    audience: String,
    policy: PathBuf,
}

fn parse(arguments: &[String]) -> Result<Options, String> {
    let mut words = arguments.iter();
    if words.next().map(String::as_str) != Some("serve") {
        return Err(
            "expected: glave gateway serve --http <address> --issuer <url> \
             --audience <name>"
                .into(),
        );
    }
    let (mut address, mut issuers, mut audience, mut policy) = (None, Vec::new(), None, None);
    while let Some(word) = words.next() {
        let mut value = |flag: &str| {
            words
                .next()
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match word.as_str() {
            "--http" => address = Some(crate::args::http_address(Some(value("--http")?))?),
            "--issuer" => issuers.push(value("--issuer")?),
            "--audience" => audience = Some(value("--audience")?),
            "--policy" => policy = Some(PathBuf::from(value("--policy")?)),
            other => return Err(format!("unknown option {other:?}")),
        }
    }
    match (address, issuers.is_empty(), audience) {
        (Some(address), false, Some(audience)) => Ok(Options {
            address,
            issuers,
            audience,
            policy: policy.unwrap_or_else(|| PathBuf::from(ORGANIZATION_POLICY)),
        }),
        _ => Err(
            "glave gateway serve needs --http <address>, --issuer <url> and --audience \
             <name>: a gateway never serves without sign-in"
                .into(),
        ),
    }
}

pub fn main(arguments: &[String]) -> ExitCode {
    if arguments
        .iter()
        .any(|word| word == "--help" || word == "-h")
    {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let options = match parse(arguments) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("glave: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("glave: could not start: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(options)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("glave: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// The organization's policy, the same for every caller until policies name people.
struct Organization(Constraints);

impl Policy for Organization {
    fn refuses_model(&self, _caller: &Caller, target: &str) -> Option<String> {
        self.0.refuses_model(target).map(ToString::to_string)
    }

    fn refuses_tool(&self, _caller: &Caller, _server: &str, _tool: &str) -> Option<String> {
        // Only the catalog's servers are started at all.
        None
    }
}

/// The budgets the policy keeps for each person.
fn budgets(constraints: &Constraints) -> Budgets {
    Budgets {
        person: constraints.person_per_day_micros().map(|micros| {
            Limit::Money(Money {
                micros: i64::try_from(micros).unwrap_or(i64::MAX),
                currency: Currency::USD,
            })
        }),
        ..Budgets::default()
    }
}

async fn run(options: Options) -> anyhow::Result<()> {
    let policies = policy::policy_files(std::slice::from_ref(&options.policy)).await?;
    let constraints = Constraints::of(&policies);
    if let Some((rule, why)) = constraints.not_enforced_by_the_gateway().first() {
        bail!("the gateway will not serve under {rule}: it is not enforced yet ({why})");
    }
    let env = |name| {
        std::env::var(name)
            .ok()
            .filter(|value: &String| !value.trim().is_empty())
    };
    let endpoint = config::endpoint(env("GLAVE_MODEL"), config::provider_here().await?.as_ref())?;
    let judge = env("GLAVE_JUDGE_MODEL");
    let allowed = Allowed {
        constraints: &constraints,
        main: Some(&Source::Env("GLAVE_MODEL")),
        judge: &Source::Env("GLAVE_JUDGE_MODEL"),
        models: &Source::Default,
    };
    let provider = Model::for_endpoint(&endpoint)?;
    let budgets = budgets(&constraints);
    let gateway = gateway(
        provider,
        &endpoint,
        judge.as_deref(),
        &[],
        Vec::new(),
        &allowed,
    )?
    .with_budgets(budgets);
    let home = home::folder()?;
    let processes = home::reaped().await?;
    let (catalog, mut secrets) = catalog(&home, &constraints, &processes).await?;
    secrets.extend(
        endpoint
            .provider
            .auth
            .fixed_key()
            .map(|key| key.expose().to_owned()),
    );
    let store = crate::data::Data::open(None).await?;
    let scope = crate::projects::named(&store, SCOPE).await?;
    let trust = Trust {
        issuers: options.issuers,
        audience: options.audience,
    };
    let service = Service::new(Wiring {
        gateway,
        policy: Arc::new(Organization(constraints)),
        signin: SignIn::new(trust, Box::new(SystemClock))?,
        log: store.log(),
        scope,
        clock: Box::new(SystemClock),
        entropy: Box::new(OsEntropy),
        catalog: catalog.clone(),
        secrets,
        notices: Arc::new(|notice: &str| eprintln!("glave gateway: {notice}")),
    });
    let listener = TcpListener::bind(options.address)
        .await
        .with_context(|| format!("could not listen on {}", options.address))?;
    eprintln!(
        "glave: serving the gateway on http://{}",
        listener.local_addr()?
    );
    let stop = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    let served = glave_gateway_service::serve(service, listener, stop).await;
    for server in &catalog {
        server.close().await;
    }
    served.context("the gateway stopped")
}

/// The catalog's servers from `mcp.json` in `home`: those `[mcp] catalog` names, or every one
/// when it names none. Started now; with them, every header and variable value they are given,
/// which are their credentials, never to be shown.
async fn catalog(
    home: &Path,
    constraints: &Constraints,
    processes: &glave_process::Processes,
) -> anyhow::Result<(Vec<Arc<McpServer>>, Vec<String>)> {
    let path = home.join(mcp::FILE);
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", path.display()));
        }
    };
    let environment = mcp::environment();
    let mut configs = if text.is_empty() {
        std::collections::BTreeMap::default()
    } else {
        glave_mcp::parse(&text, &environment).with_context(|| format!("in {}", path.display()))?
    };
    if let Some((names, rule)) = constraints.mcp_catalog() {
        let missing: Vec<&str> = names
            .iter()
            .filter(|name| !configs.contains_key(*name))
            .map(String::as_str)
            .collect();
        if !missing.is_empty() {
            bail!(
                "{rule} offers servers with no settings in {}: {}",
                path.display(),
                missing.join(", ")
            );
        }
        configs.retain(|name, _| names.contains(name));
    }
    let mut secrets = Vec::new();
    for config in configs.values() {
        match config {
            ServerConfig::Stdio { env, .. } => secrets.extend(env.values().cloned()),
            ServerConfig::Http { headers, .. } => secrets.extend(headers.values().cloned()),
        }
    }
    let servers = configs
        .iter()
        .map(|(name, config)| {
            Arc::new(McpServer::start(
                name,
                config,
                &environment,
                home,
                Limits::default(),
                processes,
            ))
        })
        .collect();
    Ok((servers, secrets))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn serving_needs_an_address_an_issuer_and_an_audience() {
        let options = parse(&words(
            "serve --http 8080 --issuer https://id.example.com --audience gw --policy /tmp/p.toml",
        ))
        .unwrap();
        assert_eq!(options.address.to_string(), "127.0.0.1:8080");
        assert_eq!(options.issuers, ["https://id.example.com"]);
        assert_eq!(options.policy, Path::new("/tmp/p.toml"));
        let default = parse(&words("serve --http 8080 --issuer https://i --audience gw")).unwrap();
        assert_eq!(default.policy, Path::new(ORGANIZATION_POLICY));
        assert!(parse(&words("serve --http 8080")).is_err());
        assert!(parse(&words("run")).is_err());
        assert!(parse(&words("serve --bogus")).is_err());
    }
}

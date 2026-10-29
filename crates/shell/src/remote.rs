//! Commands that act beyond this computer: publishing a package or image, deploying, changing a
//! cluster, cloud infrastructure, a shared database or a payments account. Ordinary when the
//! person asked for exactly that, so they are soft cautions: the judge may find the person's
//! say-so in what they asked. Pushes are found with the other git rules, and forges' tools
//! (`gh`, `glab`, `tea`) in `forge.rs`.

use glave_event::Risk;

use crate::judge::{Caution, Kind};
use crate::parse::Word;

/// Why `words` (a command after any wrapper) acts beyond this computer, if it does.
pub(crate) fn remote(name: &str, words: &[Word]) -> Option<Caution> {
    let args: Vec<&str> = words.iter().skip(1).map(Word::text).collect();
    let first = args.iter().find(|a| !a.starts_with('-')).copied();
    let has = |word: &str| args.contains(&word);
    let reason = match name {
        _ if publishes(name, first, &args) => Risk::PublishesPackage,
        "docker" | "podman" if first == Some("push") => Risk::PublishesImage,
        "kubectl" | "oc" if changes(first, KUBECTL) => Risk::ChangesKubernetes,
        "helm"
            if changes(
                first,
                &["install", "upgrade", "uninstall", "rollback", "delete"],
            ) =>
        {
            Risk::ChangesKubernetes
        }
        "terraform" | "tofu" if changes(first, &["apply", "destroy", "import"]) => {
            Risk::ChangesCloudInfrastructure
        }
        "pulumi" if changes(first, &["up", "destroy", "refresh"]) => {
            Risk::ChangesCloudInfrastructure
        }
        "vercel" | "netlify" if has("--prod") || has("--production") => Risk::DeploysToProduction,
        "fly" | "flyctl" | "firebase" if first == Some("deploy") => Risk::Deploys,
        "wrangler" if changes(first, &["deploy", "publish"]) => Risk::Deploys,
        "aws" => aws(&args)?,
        "gcloud" | "az" if args.iter().any(|a| CLOUD_VERBS.contains(a)) => {
            Risk::ChangesCloudResources
        }
        "psql" | "mysql" | "mariadb" if writes_sql(&args) => Risk::ChangesDatabase,
        "redis-cli"
            if args.iter().any(|a| a.eq_ignore_ascii_case("flushall"))
                || args.iter().any(|a| a.eq_ignore_ascii_case("flushdb")) =>
        {
            Risk::WipesRedis
        }
        "stripe"
            if args
                .iter()
                .any(|a| matches!(*a, "create" | "update" | "delete")) =>
        {
            Risk::ChangesPayments
        }
        _ => return None,
    };
    Some(Caution {
        kind: Kind::Remote,
        reason,
    })
}

/// Package managers' publish commands: `npm publish`, `cargo publish`, `twine upload`.
fn publishes(name: &str, first: Option<&str>, args: &[&str]) -> bool {
    match name {
        "npm" | "pnpm" | "yarn" | "bun" | "cargo" | "poetry" | "uv" => first == Some("publish"),
        "gem" => first == Some("push"),
        "twine" => first == Some("upload"),
        "python" | "python3" => args.contains(&"twine") && args.contains(&"upload"),
        _ => false,
    }
}

const KUBECTL: &[&str] = &[
    "apply", "create", "delete", "replace", "patch", "scale", "rollout", "drain", "cordon", "edit",
    "set", "label", "annotate",
];

const CLOUD_VERBS: &[&str] = &["create", "delete", "deploy", "update", "set", "remove"];

fn changes(first: Option<&str>, verbs: &[&str]) -> bool {
    first.is_some_and(|first| verbs.contains(&first))
}

/// The prefixes of `aws` operations that change something.
const AWS_VERBS: &[&str] = &[
    "delete-",
    "update-",
    "put-",
    "create-",
    "terminate-",
    "modify-",
    "attach-",
    "detach-",
    "run-",
    "start-",
    "stop-",
    "reboot-",
];

/// `aws` calls that change something: `delete-…`, `update-…`, `s3 rm`, a copy to S3.
fn aws(args: &[&str]) -> Option<Risk> {
    let words: Vec<&str> = args
        .iter()
        .copied()
        .filter(|a| !a.starts_with('-'))
        .collect();
    let verb = words.get(1).copied()?;
    if words.first() == Some(&"s3") {
        let to_s3 = words.last().is_some_and(|last| last.starts_with("s3://"));
        return match verb {
            "rm" | "mv" | "rb" => Some(Risk::DeletesFromS3),
            "cp" | "sync" if to_s3 => Some(Risk::WritesToS3),
            _ => None,
        };
    }
    (AWS_VERBS.iter().any(|prefix| verb.starts_with(prefix)) || verb == "deploy")
        .then_some(Risk::ChangesCloudResources)
}

/// Whether a `psql -c` or `mysql -e` statement writes.
fn writes_sql(args: &[&str]) -> bool {
    const WRITES: &[&str] = &[
        "alter", "drop", "delete", "update", "insert", "truncate", "create", "grant", "revoke",
    ];
    let statement = args
        .windows(2)
        .find(|pair| {
            matches!(
                pair.first(),
                Some(&"-c" | &"-e" | &"--command" | &"--execute")
            )
        })
        .and_then(|pair| pair.get(1));
    statement.is_some_and(|sql| {
        sql.trim_matches(|c| c == '\'' || c == '"')
            .split(|c: char| !c.is_ascii_alphabetic())
            .filter(|word| !word.is_empty())
            .any(|word| WRITES.contains(&word.to_ascii_lowercase().as_str()))
    })
}

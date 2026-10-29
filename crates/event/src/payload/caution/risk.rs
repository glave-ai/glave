//! What a call would do that needs the person, in one table: the shell's reasons for a command
//! (`glave-shell`), a read of a file that usually holds secrets, and a save to memory after
//! untrusted content. Each is a clause the model reads as it is ("it deletes ~/x") and the
//! person as a sentence of its own.

use serde::{Deserialize, Serialize};

use super::Worded;

worded! {
    /// What a call would do that needs the person, with the values it is written with.
    Risk {
        // Commands glave cannot read.
        NestsTooDeep = "risk-nests-too-deep" "it nests shells too deep to read",
        RunsFetchedCode = "risk-runs-fetched-code" "it runs code fetched from the network",

        // Deleting and discarding.
        DeletesUnknown { what } = "risk-deletes-unknown"
            "it deletes { $what }, which glave cannot tell in advance",
        Deletes { path } = "risk-deletes" "it deletes { $path }",
        DeletesOutside { path } = "risk-deletes-outside" "it deletes { $path }, outside the project",
        DiscardsChanges = "risk-discards-changes" "it discards uncommitted changes",
        DeletesUntracked = "risk-deletes-untracked" "it deletes untracked files",
        DiscardsAllChanges = "risk-discards-all-changes" "it discards all uncommitted changes",
        DropsStash = "risk-drops-stash" "it throws away stashed changes",
        DeletesBranch = "risk-deletes-branch"
            "it deletes a branch even if it isn't merged (git branch -D)",
        RewritesShared = "risk-rewrites-shared" "it rewrites or deletes what others share",

        // The computer itself.
        RunsAsAdministrator = "risk-runs-as-administrator" "it runs as the administrator",
        Repartitions = "risk-repartitions" "it formats or repartitions a disk",
        FormatsDisk = "risk-formats-disk" "it formats a disk",
        ErasesDisk = "risk-erases-disk" "it erases a disk",
        WritesDevice = "risk-writes-device" "it writes straight to a device",
        ShutsDown = "risk-shuts-down" "it shuts down or restarts the computer",
        ChangesPermissions = "risk-changes-permissions" "it changes permissions beyond the project",

        // What runs later on its own.
        ChangesGitSettings = "risk-changes-git-settings"
            "it changes git settings beyond this project",
        ChangesScheduledJobs = "risk-changes-scheduled-jobs" "it changes scheduled jobs",
        RunsAtLogin = "risk-runs-at-login" "it installs something that runs at login",
        RunsAtStartUp = "risk-runs-at-start-up" "it installs something that runs at start-up",
        WritesStartUp { path } = "risk-writes-start-up"
            "it writes { $path }, which runs later on its own",

        // Secrets the model would see.
        ReadsSecrets { path } = "risk-reads-secrets" "it reads { $path }, which usually holds secrets",
        SecretFile = "risk-secret-file" "this file usually holds secrets; the model would see them",
        CredentialHelper = "risk-credential-helper"
            "it reaches glave's credential helper, which hands out tokens",
        GitKeepsCredentials = "risk-git-keeps-credentials"
            "it changes how git keeps credentials, which could show one",
        PrintsGitCredentials = "risk-prints-git-credentials"
            "it prints git's stored credentials, which the model would see",
        PrintsGitHubToken = "risk-prints-github-token"
            "it prints a GitHub token, which the model would see",
        PrintsGitLabToken = "risk-prints-gitlab-token"
            "it prints a GitLab token, which the model would see",
        PrintsKeychainPassword = "risk-prints-keychain-password"
            "it prints a password from the keychain, which the model would see",
        PrintsKeychainPasswords = "risk-prints-keychain-passwords"
            "it prints the keychain's passwords, which the model would see",
        PrintsGoogleCloudToken = "risk-prints-google-cloud-token"
            "it prints a Google Cloud token, which the model would see",
        PrintsAzureToken = "risk-prints-azure-token"
            "it prints an Azure token, which the model would see",
        PrintsOnePasswordSecret = "risk-prints-1password-secret"
            "it prints a secret from 1Password, which the model would see",
        PrintsKubernetesCredentials = "risk-prints-kubernetes-credentials"
            "it prints a Kubernetes cluster's credentials, which the model would see",
        PrintsAwsCredentials = "risk-prints-aws-credentials"
            "it prints AWS credentials, which the model would see",
        PrintsAwsRegistryPassword = "risk-prints-aws-registry-password"
            "it prints an AWS registry password, which the model would see",
        PrintsVaultSecret = "risk-prints-vault-secret"
            "it prints a secret from Vault, which the model would see",

        // A forge's sign-in, keys, secrets and API.
        WritesThroughApi { forge } = "risk-writes-through-api"
            "it writes through { $forge }'s API, which reaches anything the token can",
        ChangesSignIn { forge } = "risk-changes-sign-in"
            "it changes how this computer signs in to { $forge }",
        ChangesRepositorySecrets = "risk-changes-repository-secrets"
            "it changes a repository's secrets",
        ChangesRepositoryVariables = "risk-changes-repository-variables"
            "it changes a repository's variables",
        ChangesProjectVariables = "risk-changes-project-variables"
            "it changes a project's variables",
        ChangesAccountKeys = "risk-changes-account-keys" "it changes a GitHub account's keys",

        // What others share on a forge.
        OpensPullRequest = "risk-opens-pull-request" "it opens a pull request",
        CommentsOnPullRequest = "risk-comments-on-pull-request" "it comments on a pull request",
        MergesPullRequest = "risk-merges-pull-request" "it merges a pull request",
        ChangesPullRequest = "risk-changes-pull-request" "it changes a pull request",
        OpensMergeRequest = "risk-opens-merge-request" "it opens a merge request",
        CommentsOnMergeRequest = "risk-comments-on-merge-request"
            "it comments on a merge request",
        MergesMergeRequest = "risk-merges-merge-request" "it merges a merge request",
        ChangesMergeRequest = "risk-changes-merge-request" "it changes a merge request",
        OpensIssue = "risk-opens-issue" "it opens an issue",
        CommentsOnIssue = "risk-comments-on-issue" "it comments on an issue",
        CommentsOnIssueOrPullRequest = "risk-comments-on-issue-or-pull-request"
            "it comments on an issue or a pull request",
        ChangesIssue = "risk-changes-issue" "it changes an issue",
        PublishesRelease = "risk-publishes-release" "it publishes a release",
        ChangesRelease = "risk-changes-release" "it changes a release",
        CreatesRepository = "risk-creates-repository" "it creates a repository",
        ForksRepository = "risk-forks-repository" "it forks a repository",
        ChangesRepository = "risk-changes-repository" "it changes a repository",
        PublishesGist = "risk-publishes-gist" "it publishes a gist",
        ChangesGist = "risk-changes-gist" "it changes a gist",
        PublishesSnippet = "risk-publishes-snippet" "it publishes a snippet",
        ChangesRepositoryLabels = "risk-changes-repository-labels"
            "it changes a repository's labels",
        ChangesProjectLabels = "risk-changes-project-labels" "it changes a project's labels",
        RunsWorkflow = "risk-runs-workflow" "it runs a workflow",
        ChangesWorkflows = "risk-changes-workflows" "it changes a repository's workflows",
        RunsPipeline = "risk-runs-pipeline" "it runs a pipeline",

        // Beyond this computer.
        Pushes = "risk-pushes" "it pushes to a remote",
        PublishesPackage = "risk-publishes-package" "it publishes a package",
        PublishesImage = "risk-publishes-image" "it publishes an image",
        ChangesKubernetes = "risk-changes-kubernetes" "it changes a Kubernetes cluster",
        ChangesCloudInfrastructure = "risk-changes-cloud-infrastructure"
            "it changes cloud infrastructure",
        ChangesCloudResources = "risk-changes-cloud-resources" "it changes cloud resources",
        DeploysToProduction = "risk-deploys-to-production" "it deploys to production",
        Deploys = "risk-deploys" "it deploys",
        DeletesFromS3 = "risk-deletes-from-s3" "it deletes from S3",
        WritesToS3 = "risk-writes-to-s3" "it writes to S3",
        ChangesDatabase = "risk-changes-database" "it changes a database",
        WipesRedis = "risk-wipes-redis" "it wipes a Redis database",
        ChangesPayments = "risk-changes-payments" "it changes a payments account",

        // A save to memory.
        RemembersAfterUntrusted = "risk-remembers-after-untrusted"
            "this thread has read untrusted content or a secrets file, so saving to memory now could carry a planted instruction into every later thread; it runs only if the person asked for this to be remembered",
    }
}

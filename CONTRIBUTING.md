# Contributing

Thanks for wanting to help. glave is built by people and AI agents together, and the same rules apply to both: they're in [AGENTS.md](AGENTS.md). Please read it before your first change.

## The short version

1. **Open or find an issue first,** so we agree on the change before you build it.
2. **One branch, one change.** Branch names start with the change's type (`feat/`, `fix/`, `docs/`, `chore/`), and pull request titles are conventional (`feat(web): …`).
3. **Run the full gate** before asking for review:
   ```sh
   cargo xtask ci
   ```
   It covers formatting, lints, tests, the dependency audit and file-size limits, plus the web checks when Node is installed.
4. **Every pull request is reviewed before it merges.**

## Sign your work (DCO)

We use the [Developer Certificate of Origin](https://developercertificate.org/). It certifies that you wrote the change, or have the right to contribute it under the project's license. Sign off every commit:

```sh
git commit -s -m "fix(shell): …"
```

This adds a `Signed-off-by:` line with your name and email. There's no separate agreement to sign.

## Security

Please report vulnerabilities privately; see [SECURITY.md](SECURITY.md).

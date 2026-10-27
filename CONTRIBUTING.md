# Contributing

Thanks for wanting to help. glave is built by people and AI agents together, and the same rules apply to both: they're in [AGENTS.md](AGENTS.md). Please read it before your first change.

## The short version

1. **Open or find an issue first,** so we agree on the change before you build it.
2. **One branch, one change.** Branch names start with the change's type (`feat/`, `fix/`, `docs/`, `chore/`), and pull request titles are conventional (`feat(web): …`).
3. **Run the full gate** before asking for review:
   ```sh
   cargo xtask ci --all
   ```
   It covers formatting, lints, tests, the dependency audit and file-size limits, plus the web checks when Node is installed. While you work, `cargo xtask ci` alone checks only the crates your change reaches, and the web when you changed it.
4. **Every pull request is reviewed before it merges.**

## Security

Please report vulnerabilities privately; see [SECURITY.md](SECURITY.md).

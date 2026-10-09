# Glave

The open-source user agent for AI: one binary, any model, with Hope as the free house model.

Early development. Private until v0.1.

## Try it

`glave` runs hope, or any model with an OpenAI-compatible API, on the project in the current folder.

```sh
cargo build --release -p glave

# A local model with Ollama
GLAVE_MODEL=gemma4:12b ./target/release/glave "explain this project"

# Any model on OpenRouter
GLAVE_BASE_URL=https://openrouter.ai/api/v1 GLAVE_MODEL=<model id> GLAVE_API_KEY=<key> \
  ./target/release/glave
```

Without a prompt it starts a session where each line is a turn. By default glave acts without asking, except before reading files that usually hold secrets; `--approval ask` asks before every change and command. Every turn ends with a receipt (calls, tokens, cost). Threads are kept in `~/.glave/glave.db` (or `$GLAVE_HOME`); `--continue` picks up your last thread in the current folder, and `--events file.jsonl` exports one. `glave --help` lists everything.

The local database is not encrypted yet; its folder is readable only by you.

## Development

```sh
cargo xtask ci
```

Runs the full gate: format, lint, tests, dependency audit, and file-size limits. The rules every contributor (human or agent) follows are in [AGENTS.md](AGENTS.md).

## License

[Apache-2.0](LICENSE)

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

Without a prompt it starts a session where each line is a turn. It asks before writing, editing or running anything (`--yes` skips that), ends every turn with a receipt (calls, tokens, cost), and `--events file.jsonl` saves the thread's events. `glave --help` lists everything.

## Development

```sh
cargo xtask ci
```

Runs the full gate: format, lint, tests, dependency audit, and file-size limits. The rules every contributor (human or agent) follows are in [AGENTS.md](AGENTS.md).

## License

[Apache-2.0](LICENSE)

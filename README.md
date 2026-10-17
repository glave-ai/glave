# glave

An open-source Claude or ChatGPT that you run yourself, where every conversation can have a real computer behind it.

Run it on your own machine and it just works: privately, with any model. A company runs the very same software as its own private AI cloud, with its sign-in, models, data and rules.

> **Early.** glave works today and is tested on three model families, but it's young. Expect rough edges and changes.

## What it does

- **Chat with any model:** OpenAI, Anthropic, OpenRouter, any OpenAI-compatible server (Ollama, vLLM, LM Studio), and through your company's cloud: AWS Bedrock, Google Vertex AI, Azure OpenAI.
- **Real work on a real computer:** glave reads, writes and runs code in your project folders, in a sandbox (macOS and Linux). It asks before anything risky, and a small judge lets through the risky-looking things you clearly asked for, so it doesn't nag.
- **Your projects, your threads:** many threads per project, each working on its own git branch, with memory you can see, edit and erase.
- **Connections:** connect GitHub (or GitLab, Gitea, Bitbucket) and MCP servers. Tokens are sealed and handed out per use, never left lying on the machine.
- **Yours:** everything is stored encrypted on your machine, or in your company's Postgres. No account with us, and nothing sent anywhere except to the models you choose.

## Run it

On macOS or Linux:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/glave-ai/glave/releases/latest/download/glave-installer.sh | sh
glave app
```

Your browser opens, signed in. Add a model by pasting a key, and start a chat.

Prefer the terminal? `glave` starts a session right there. `glave --help` lists everything.

With Homebrew instead: `brew install glave-ai/tap/glave`. Both ways work once the first release is published; until then, build glave from source.

### From source

You need Rust (see `rust-toolchain.toml`) and Node 22.22.3 or later.

```sh
git clone https://github.com/glave-ai/glave
cd glave
(cd web && npm ci && npm run build)
cargo build --release -p glave --features web
./target/release/glave app
```

## For companies

The same binary runs as a server for a whole organization: sign-in with your identity provider (OIDC), Postgres for storage, your models through your cloud contract, and agent computers on your Kubernetes cluster. The design is in [specs/](specs/), and the plan is in [ROADMAP.md](ROADMAP.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). The rules every contributor follows, human or AI agent, are in [AGENTS.md](AGENTS.md). Security issues: see [SECURITY.md](SECURITY.md).

## License

[Apache-2.0](LICENSE)

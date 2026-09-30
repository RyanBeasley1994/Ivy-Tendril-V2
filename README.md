<h1>
  <img src="src/apps/tendril-app/src-tauri/icons/128x128@2x.png" alt="Forge logo" width="64" valign="middle" /> Forge
</h1>

<p><strong>An agentic software factory: plan the work, let coding agents build it in isolated worktrees, and review what comes back.</strong></p>

> **Forge is a heavily edited and extended fork of [Ivy Tendril](https://github.com/Ivy-Interactive/Ivy-Tendril-V2) by [Ivy Interactive](https://github.com/Ivy-Interactive).**
> Their work is the foundation this is built on. See [Credit](#credit) and [License](#license).

Built and maintained by **Ryan Beasley**.

---

## What Forge does

- **Plans and missions.** Describe a change and Forge turns it into a plan an agent executes. A mission goes further: it breaks a whole feature into milestones, runs each one, and judges it against its acceptance criteria.
- **Parallel worktrees.** Every run happens in its own git worktree, so your main branch stays clean until you review, approve and merge.
- **Code review with verification gates.** Inspect diffs, run the project's verifications, and approve or send work back.
- **Chat with your agents.** Talk to any configured coding agent, with the model chosen by profile (Deep / Balanced / Quick).
- **Remote servers.** Run the Forge server on a VPS and drive it from the desktop app, over HTTPS or a tunnel.
- **Voice and rich input.** Dictate prompts and drop in files, logs and screenshots.
- **GitHub integration.** Pull issues into plans, and open, update and merge pull requests.

## What's different from Tendril

Forge started from Tendril V2 and has been reworked throughout. Highlights so far:

- **Profile-based chat.** The chat picker runs each harness on its Deep / Balanced / Quick profile, the same models and efforts your plans use, instead of a fixed default model.
- **Local LLMs for Codex.** When Codex's own `config.toml` points at a non-OpenAI provider (Ollama, LM Studio, vLLM…), Forge offers that model and stops overriding it with OpenAI model ids, in chat and in jobs.
- **Remote servers that stay connected.** Secure WebSocket (`wss://`) support, keepalive pings for connections behind Cloudflare and other proxies, and a folder browser that shows the *server's* disk when you are connected remotely.
- **Add projects without leaving your work.** "Add New Project" opens in place over the Create Plan / Mission dialog, and a searchable **Clone from GitHub** dropdown lists every repository your `gh` account can clone.
- **Reworked UI.** A tighter Create dialog, a days-and-hours usage countdown, and a new look and identity.
- **No upstream services.** Update checks, newsletter sign-up and community links to Ivy's services are gone.

## Supported agents

Forge runs on top of the coding agent you already use. If it runs in a terminal, it runs in Forge:

**Claude Code** · **Codex** · **GitHub Copilot** · **Gemini** · **OpenCode** · **Cursor** · **Apple Foundation Models** · and any other CLI agent.

---

## Building from source

### Prerequisites

- [Rust](https://rustup.rs/) (edition 2021)
- [Node.js](https://nodejs.org/) v22+ and [pnpm](https://pnpm.io/) v11+
- [Vite+](https://viteplus.dev/) (`vp`)
- GitHub CLI (`gh`), signed in with `gh auth login`

### Run in development

```bash
pnpm install
pnpm --filter @ivy-interactive/components build   # the shared UI library
pnpm dev:tauri                                     # desktop app with hot reload
```

### Build the desktop app (macOS)

```bash
# 1. The server binary the app bundles
cargo build --release --bin tendril
cp target/release/tendril \
  "src/apps/tendril-app/src-tauri/binaries/tendril-$(rustc -vV | sed -n 's|host: ||p')"

# 2. The app bundle
cd src/apps/tendril-app
pnpm tauri build --bundles app
# -> target/release/bundle/macos/Forge.app
```

### Run the server on its own

The server and CLI are one binary. It is still called `tendril` for now, as are the crates and some
internal paths (`~/.tendril`, `TENDRIL_HOME`).

```bash
tendril run        # serves the HTTP & WebSocket API on 127.0.0.1:5010
tendril doctor     # checks the installation
tendril --help     # every subcommand
```

To use it from the desktop app on another machine, set a password on the server, then enter its
address under **Settings → Remote Server**.

### Tests

```bash
pnpm test                 # web and component tests
cargo test --workspace    # Rust tests
```

---

## Directory layout

```
├── src/
│   ├── apps/
│   │   ├── tendril-app/      # Tauri desktop app + React frontend (Forge)
│   │   └── tendril-docs/     # Documentation site
│   ├── packages/
│   │   └── components/       # Shared UI component library + Storybook
│   ├── crates/
│   │   ├── tendril-core/     # Domain models, SQLite database, worktree engine
│   │   ├── tendril-server/   # Axum REST & WebSocket server
│   │   └── tendril-cli/      # Command-line interface
│   ├── extensions/vscode/    # VS Code-family extension
│   ├── promptwares/          # Agent definitions & firmware
│   └── skills/               # Agent workflow skills
├── docs/
├── Cargo.toml                # Cargo workspace
└── pnpm-workspace.yaml       # pnpm workspace
```

---

## Credit

Forge exists because of **[Ivy Tendril](https://github.com/Ivy-Interactive/Ivy-Tendril-V2)**, created by
**[Ivy Interactive](https://github.com/Ivy-Interactive)**. The architecture, the plan and job engine, the
worktree model and much of the UI began as their work. Forge is an independent fork: it is not
affiliated with or endorsed by Ivy Interactive, and "Ivy" and "Tendril" are their names, not Forge's.

## License

Forge is distributed under the same license as the project it forks, the
**[Functional Source License, Version 1.1, ALv2 Future License (FSL-1.1-ALv2)](LICENSE)**.

- Portions are **Copyright 2026 Ivy Interactive**, used and modified under that license.
- Modifications are Copyright 2026 Ryan Beasley.

Under FSL-1.1, each release may be used for any purpose other than a *Competing Use* (a commercial
product or service that competes with the licensor's), and it converts to the Apache License 2.0
two years after it is made available. Read [LICENSE](LICENSE) for the exact terms.

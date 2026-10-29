<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/brand/lockup-dark.svg">
    <img alt="Demi" src="docs/assets/brand/lockup-light.svg" height="72">
  </picture>
</p>

<p align="center">
  <strong>Your agent, across all your machines.<br>
  A web-based AI agent you host on your own server.</strong>
</p>

<p align="center">
  <a href="#license"><img alt="License" src="https://img.shields.io/badge/license-Apache%202.0-blue"></a>
  <a href="https://github.com/demi-sh/demi/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/demi-sh/demi"></a>
  <a href="https://github.com/demi-sh/demi/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/demi-sh/demi/actions/workflows/ci.yml/badge.svg"></a>
</p>

<p align="center">
  <b>English</b> · <a href="README.de.md">Deutsch</a> · <a href="README.es.md">Español</a> · <a href="README.fr.md">Français</a> · <a href="README.pt-BR.md">Português</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a>
</p>

<p align="center">
  <img src="docs/assets/product.webp" alt="Demi in a browser: an agent builds a two-column layout for an iPhone app, with a comment and a side question on its reply and suggested replies under it, the app on a half-folded iPhone Duo in the side panel, and the same conversation on a phone." width="800">
</p>

Demi is a web-based AI agent you host on your own server. Open it in any
browser and hand it anything: research, websites, files or code. Your
conversations stay on your server, and the work runs in a cloud sandbox or on
your own machines, even after you close the tab.

> [!NOTE]
> Demi is in early development. Expect breaking changes between releases.

## Quick start

Deploy Demi in a few clicks:

<p>
  <a href="docs/delivery/railway.md"><img alt="Deploy on Railway" src="https://railway.com/button.svg" height="36"></a>
  <a href="https://render.com/deploy?repo=https://github.com/demi-sh/demi"><img alt="Deploy on Render" src="docs/assets/deploy/deploy-on-render.svg" height="36"></a>
  <a href="docs/delivery/fly.md"><img alt="Deploy on Fly.io" src="docs/assets/deploy/deploy-on-fly.svg" height="36"></a>
  <a href="docs/delivery/docker-compose.md"><img alt="Run with Docker Compose" src="docs/assets/deploy/run-with-docker-compose.svg" height="36"></a>
</p>

Or install it on a Linux server of your own, with a domain name:

```sh
curl -fsSL https://demi.sh/install.sh | sudo bash
```

Or ask a coding agent, such as Claude Code or Codex, to install it:

```text
Install Demi on my server by following https://raw.githubusercontent.com/demi-sh/demi/main/docs/delivery/install-with-an-agent.md
```

Then open Demi, create the admin account, and add a model in
**Settings › Providers**. The [installation guide](docs/delivery/installation.md)
covers supported systems, HTTPS, the Canary channel and what each setup costs.

<p align="center">
  <img src="docs/assets/concept.webp" alt="From a computer or a phone, you reach one Demi server, which holds every conversation and works through runners on your laptop, desktop, HomeLab and VPS, or in cloud sandboxes." width="800">
</p>

## Key features

### Chat, work and code

- **One conversation for everything**: quick questions and multi-step jobs alike.
- **Coding**: edit repositories, run builds and tests, review each change as a diff.
- **Research**: search the web and read pages, PDFs and documents.
- **Real browser**: Demi drives Chrome. Watch it and take over any time.
- **Phone apps**: install and test apps on iOS Simulators and Android Emulators.
- **Files and data**: documents, spreadsheets and images, returned as attachments.
- **Images, video and audio**: made through the model providers you add.
- **Comments and questions**: select words in a reply to ask about them.
- **Share and search**: share a read-only link, and search every message.

### Anywhere, on any machine

- **Any browser**: computer, tablet, phone or foldable, with nothing to install.
- **Always on**: tasks keep running after you close the tab, and notify you.
- **One home for conversations**: start on your laptop, continue on your phone.
- **Private cloud sandbox**: each user gets their own Linux sandbox.
- **Your own computers**: connect macOS, Windows and Linux with one command.
- **Many machines, one task**: build on a Mac, test on Windows, switch mid-task.
- **Files everywhere**: browse, preview and search every machine's files.
- **Port forwarding**: reach a port of one machine from another.
- **Sign in your way**: QR code or passkey, in nine languages.

### Agent

- **Multi-agent**: a team of agents splits a large task and works in parallel.
- **Profiles and workflows**: specialist agents, shared through your repository.
- **Schedules**: wake a conversation on a timer, by cron, or on a change.
- **Conversations work together**: one conversation messages or reads another.
- **Skills**: teach Demi new tasks.
- **Secure credentials**: secrets go through a form, never the conversation.

### Models and teams

- **Your subscriptions**: Claude, ChatGPT and Grok Build, rotating accounts at their limits.
- **Any API**: Anthropic, OpenAI, Gemini, coding plans such as Kimi, GLM or MiniMax, and local models.
- **Switch models any time**, even in the middle of a task.
- **Usage and cost**: see each subscription's limits and what the work cost.
- **Multi-user**: each user has their own sandbox, devices and conversations.

## An agent built on a shell

Demi doesn't wrap Claude Code or Codex: its own runtime drives every model.
Where most agents give the model a long list of tools, Demi gives it one, a
shell. The browser, other machines and subagents are `demi` commands in it:

```sh
T=$(demi browser tab new) && demi browser open $T http://localhost:5173 && demi browser screenshot $T | demi file view
demi host shell --host windows 'cargo test'
demi agent spawn --profile research --description "Compare pricing pages" < brief.md
```

- **Composable**: commands chain with pipes, so one step does the work of several tool calls.
- **Lean prompts**: the model reads a command's `--help` only when it needs it.
- **Same shell everywhere**: one bash-compatible shell, written in Rust, on every machine.
- **Same for every model**: switching models doesn't change what the agent can do.
- **Built for long jobs**: commands run in the background and report back.

> [!NOTE]
> **Integrations**: your own programs build agent products on Demi's
> conversations, agents and Clouds through its API. See
> [Build on Demi with an integration](docs/guides/integrations.md).

## Architecture

```text
  Browsers (computer, phone, tablet)
        │  HTTPS + WebSocket
        ▼
  ┌──────────────────────────┐   HTTPS   ┌────────────────────┐
  │ Demi server              │──────────▶│ Model providers    │
  │ users, conversations,    │           └────────────────────┘
  │ agents                   │
  └──────────────────────────┘
        ▲                ▲
        │ WebSocket      │ WebSocket
  ┌─────┴──────────┐  ┌──┴───────────────────────────┐
  │ Runner in your │  │ Runners on your computers    │
  │ cloud sandbox  │  │ and servers (macOS, Windows, │
  │                │  │ Linux)                       │
  └────────────────┘  └──────────────────────────────┘
```

- **The server runs the agent, runners do the work**: each runner connects
  out, so it works behind NAT and firewalls.
- **One runner everywhere**: every feature works on every machine.
- **Light on bandwidth**: large files stay where they were made until you open them.
- **Native**: Rust for the server, runner and tools; Vue and TypeScript for the web app.

Read the [design docs](docs/README.md), starting with the
[overview](docs/overview.md).

## Comparison

How Demi differs from OpenHands, Orca, T3 Code, DeepSeek Harness, opencode,
Pi and herdr, feature by feature: [Demi compared](docs/comparison.md).

## Documentation

- [Design docs](docs/README.md): how every part of Demi works
- [Contributing](CONTRIBUTING.md): development setup, checks, and adding a
  provider
- [Security](SECURITY.md): reporting vulnerabilities and where data is stored

## License

Demi is released under the [Apache License 2.0](LICENSE).

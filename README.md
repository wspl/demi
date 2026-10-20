<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/brand/lockup-dark.svg">
    <img alt="Demi" src="docs/assets/brand/lockup-light.svg" height="72">
  </picture>
</p>

<p align="center">
  <strong>A self-hosted AI agent for chat, work and code.<br>
  Use it from any browser. Let it work in the cloud and on all your computers.</strong>
</p>

<p align="center">
  <a href="#license"><img alt="License" src="https://img.shields.io/badge/license-Apache%202.0%20%2B%20Demi%20License-blue"></a>
  <a href="https://github.com/wspl/demi/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/wspl/demi"></a>
  <a href="https://github.com/wspl/demi/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/wspl/demi/actions/workflows/ci.yml/badge.svg"></a>
</p>

<p align="center">
  <b>English</b> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a>
</p>

<!-- Screenshot -->

Demi is a self-hosted AI agent you use from any browser. Ask it a question or
hand it a job: research, operating websites, working with files, writing
code, all in the same conversation.

Every conversation lives on your server, in one place for all your devices.
Your computers only run the work: the agent works in a cloud sandbox or
across your own machines, and tasks keep running when you close the tab.

<p align="center">
  <img src="docs/assets/concept.webp" alt="From a computer or a phone, you reach one Demi server, which holds every conversation and works through runners on your laptop, desktop, HomeLab and VPS, or in cloud sandboxes." width="800">
</p>

> [!NOTE]
> Demi is in early development. Expect breaking changes between releases.

## Quick start

You need the following:

- A Linux server you have root on, running one of these
  ([supported distributions](docs/delivery/installation.md#distributions)):
  - Ubuntu 22.04 or later, Debian 12 or later
  - Fedora, RHEL 10, Rocky Linux, AlmaLinux
  - Arch Linux
- A domain name, served over HTTPS in front of the server
  ([HTTPS setup](docs/delivery/installation.md#https)).

Then run this command on the server:

```sh
curl -fsSL https://demi.sh/install.sh | sudo bash
```

The installer asks a few questions and sets Demi up. After it finishes:

1. Open your domain right away and create the admin account.
2. Go to **Settings › Providers** and sign in with a subscription or add an
   API key.
3. Start a conversation. It runs in your cloud sandbox.
4. To work on your own computer, choose **Add Device** from the conversation's
   host menu and run the command it shows on that computer.

See the [installation guide](docs/delivery/installation.md) for every option,
including installing without questions, and [upgrades](docs/delivery/upgrades.md)
for moving to a new release.

## Key features

Demi's features fall into four groups.

### Chat, work and code

- **One conversation for everything**: Ask a quick question or hand off a
  multi-step job in the same place. No modes to switch.
- **Coding**: Edit repositories, run builds and tests, and review every change
  as a diff.
- **Research**: Search the web and read pages, PDFs and documents.
- **Real browser**: Demi drives Chrome to sign in, fill in forms and download
  files. Watch it in the side panel and take over at any time.
- **Files and data**: Work with documents, spreadsheets and images, and get the
  results back as attachments.
- **Share**: Share a conversation, or a single answer, as a read-only link.

### Anywhere, on any machine

- **Works in any browser**: Desktop, tablet or phone, with nothing to install.
- **Always on**: Tasks keep running on the server after you close the tab.
- **One place for every conversation**: The server stores conversations
  instead of scattering them across the machines the agent worked on. Start on
  your laptop and continue on your phone, live.
- **QR code and passkey sign-in**: Scan a code to sign in on your phone, or use
  Touch ID, Face ID or Windows Hello.
- **Private cloud sandbox**: Every user gets their own Linux sandbox on your
  server. It runs under gVisor, can't reach your server's services or your
  private network, and can't see other users' sandboxes.
- **Bring your own computers**: Connect macOS, Windows and Linux machines with
  a single command. No open ports needed.
- **Multi-machine tasks**: Use several machines in one conversation, for
  example to build on a Mac and test on Windows.
- **Switch machines mid-task**: Move a conversation between the cloud and your
  computers without starting over.

### Agent

- **Multi-agent**: Demi splits a large task across a team of agents that work
  in parallel, then combines their results.
- **Agent profiles**: Define specialist agents, such as a researcher or a
  reviewer, each with its own model, reasoning effort, instructions and
  commands.
- **Workflows**: Bundle a set of agent profiles with instructions for one kind
  of work. Keep workflows in a project's `.demi/workflows/`, pick one in the
  message box, and share them with your team through the repository.
- **Skills**: Install skills to teach Demi new tasks.
- **Secure credentials**: Demi requests API keys and passwords through a
  secure form, and they never appear in the conversation.

### Models and teams

- **Use your subscriptions**: Sign in with Claude, ChatGPT or Grok Build
  accounts. Add several accounts per subscription, and Demi rotates between
  them as their usage limits run out.
- **Any API**: Anthropic, OpenAI, Gemini, or any compatible endpoint.
- **Switch models any time**: Change the model or reasoning effort, even in the
  middle of a task.
- **Multi-user**: Each user has their own sandbox, devices and
  conversations, on the admin's subscriptions or their own.
- **Admin tools**: Add users, manage roles, reset passwords and send phone
  sign-in codes.
- **One-line install and upgrades**: Set up a server with one command and
  upgrade it with another.

## An agent built on a shell

Demi is both the app and the agent.

It doesn't wrap Claude Code or Codex. Its own runtime drives every model, and a
Claude or ChatGPT subscription serves only the model.

Most agents give the model a long list of tools. Demi gives it one: a
shell. Everything else, from the browser to other machines to subagents, is a
`demi` command inside it:

```sh
demi browser open http://localhost:5173 && demi browser screenshot t1 | demi file view
demi host shell --host windows 'cargo test'
demi agent spawn --profile research --description "Compare pricing pages" < brief.md
```

The design has these benefits:

- **Composable**: Commands chain with pipes and `&&`, so the agent does in one
  step what usually takes several tool calls.
- **Lean prompts**: The model sees one line per command group and reads
  `--help` when it needs more. New capabilities don't bloat every request.
- **Same everywhere**: Every machine runs the same bash-compatible shell and
  GNU tools, written in Rust ([brush](https://github.com/reubeno/brush),
  [uutils](https://github.com/uutils/coreutils), ripgrep,
  [jaq](https://github.com/01mf02/jaq)). A script that works in the cloud
  works on macOS and Windows too.
- **Same for every model**: Every provider sees the same single tool, so
  switching models doesn't change what the agent can do.
- **Built for long jobs**: Commands keep running in the background and report
  back when they finish, so dev servers and test suites never block the
  conversation.

> [!NOTE]
> **Planned**: the Demi agent runtime as a standalone TypeScript library, so
> you can build your own agents on it without the Demi app.

## Architecture

The following diagram shows how the parts connect:

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
  │ (gVisor)       │  │ Linux)                       │
  └────────────────┘  └──────────────────────────────┘
```

The design has these properties:

- **The server runs the agent, runners do the work**: Conversations, agents
  and settings live on the server. A small runner on each machine only handles
  files, commands and the browser. It connects out to the server, so it works
  behind NAT and firewalls.
- **One runner everywhere**: The cloud sandbox and your computers run the same
  runner, so every feature works on every machine.
- **Light on bandwidth**: Large outputs and files stay on the machine that made
  them. The browser loads them only when you open them.
- **Native**: The server, the runner and the agent's tools are written in Rust.
  The web app is built with Vue and TypeScript.

Read the [design docs](docs/README.md), starting with the
[overview](docs/overview.md).

## Comparison

How Demi differs from Pi, DeepSeek Harness, OpenHands, opencode, Orca,
herdr and T3 Code, feature by feature: [Demi compared](docs/comparison.md).

## Documentation

- [Design docs](docs/README.md): how every part of Demi works
- [Contributing](CONTRIBUTING.md): development setup, checks, and adding a
  provider
- [Security](SECURITY.md): reporting vulnerabilities and where data is stored

## License

Most of Demi is released under the [Apache License 2.0](LICENSE). Use these
parts for anything:

- The agent runtime.
- The model providers.
- The runner you install on your machines.
- The commands.
- The plugins and their SDK.
- The UI component library and its gallery.
- The protocols.
- The docs.

The server and the web app (`crates/backend*`, `crates/server`,
`crates/machine-manager` and `packages/web`) are released under the
[Demi License](LICENSE-DEMI), which is the Apache License 2.0 with additional
conditions:

- **Free to use**: Use, modify and run them, including at your company for
  your own team.
- **Free to support**: Charge for deploying, integrating or supporting Demi
  that others run themselves.
- **Not for resale as a service**: Don't offer them to others for a fee as a
  hosted or multi-tenant service, or as a product that is substantially Demi.
- **Keep the branding**: Don't remove or change the Demi name, logo or
  copyright notices in the web app.

For other uses, contact [support@demi.sh](mailto:support@demi.sh).

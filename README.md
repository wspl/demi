<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="brand/lockup-dark.svg">
    <img alt="Demi" src="brand/lockup-light.svg" height="72">
  </picture>
</p>

<p align="center">
  <strong>A self-hosted AI agent for chat, work and code.<br>
  Use it from any browser. Let it work in the cloud and on all your computers.</strong>
</p>

<p align="center">
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/badge/license-Demi%20License-blue"></a>
  <a href="https://github.com/wspl/demi/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/wspl/demi"></a>
  <a href="https://github.com/wspl/demi/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/wspl/demi/actions/workflows/ci.yml/badge.svg"></a>
</p>

<p align="center">
  <b>English</b> · <a href="README.zh-CN.md">简体中文</a>
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

## Key features

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

### Anywhere, on any machine

- **Works in any browser**: Desktop, tablet or phone, with nothing to install.
- **Always on**: Tasks keep running on the server after you close the tab.
- **One place for every conversation**: Conversations are stored on the
  server, not scattered across the machines the agent worked on. Start on your
  laptop and continue on your phone, live.
- **QR code and passkey sign-in**: Scan a code to sign in on your phone, or use
  Touch ID, Face ID or Windows Hello.
- **Private cloud sandbox**: Every user gets their own Linux sandbox on your
  server, isolated with gVisor.
- **Bring your own computers**: Connect macOS, Windows and Linux machines with
  a single command. No open ports needed.
- **Multi-machine tasks**: Use several machines in one conversation, for
  example to build on a Mac and test on Windows.
- **Switch machines mid-task**: Move a conversation between the cloud and your
  computers without starting over.
- **Share a preview**: Expose a dev server on any machine through a temporary
  public URL.

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
- **Secure credentials**: API keys and passwords are requested through a secure
  form and never appear in the conversation.

### Models and teams

- **Use your subscriptions**: Sign in with Claude, ChatGPT or Grok Build
  accounts. Add several accounts per subscription and switch between them,
  with each account's usage limits in view.
- **Any API**: Anthropic, OpenAI, Gemini, or any compatible endpoint.
- **Switch models any time**: Change the model or reasoning effort, even in the
  middle of a task.
- **Multi-user**: Each user has their own sandbox, models, devices and
  conversations.
- **Admin tools**: Add users, manage roles, reset passwords and send phone
  sign-in codes.
- **One-line install and upgrades**: Set up a server with one command and
  upgrade it with another.

## A harness built on a shell

Demi is both the app and the agent harness. It doesn't wrap Claude Code or
Codex: its own runtime drives every model, and a Claude or ChatGPT
subscription is used only for the model.

Most harnesses give the model a long list of tools. Demi gives it one: a
shell. Everything else, from the browser to other machines to subagents, is a
`demi` command inside it:

```sh
demi browser open http://localhost:5173 && demi browser screenshot t1 | demi file view
demi host shell --host windows 'cargo test'
demi agent spawn --profile research --description "Compare pricing pages" < brief.md
```

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

## Quick start

**Requirements**: a Linux server (Ubuntu 22.04+ or Debian 12+) with root
access, a domain name, and HTTPS in front of it.

```sh
curl -fsSL https://github.com/wspl/demi/releases/latest/download/install.sh | sudo bash
```

The installer asks for your domain and HTTPS setup, then prints the address
of your server. From there:

1. Open the address and create the admin account.
2. Go to **Settings › Providers** and sign in with a subscription or add an
   API key.
3. Start a conversation. It runs in your cloud sandbox.
4. To work on your own computer, choose **Add Device** from the conversation's
   host menu and run the command it shows on that computer.

See the [installation guide](docs/delivery/installation.md) for all options and
[upgrades](docs/delivery/upgrades.md) for moving to a new release.

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
  │ (gVisor)       │  │ Linux)                       │
  └────────────────┘  └──────────────────────────────┘
```

- **The server runs the agent, runners do the work**: Conversations, agents
  and settings live on the server. A small runner on each machine only handles
  files, commands and the browser. It connects out to the
  server, so it works behind NAT and firewalls.
- **One runner everywhere**: The cloud sandbox and your computers run the same
  runner, so every feature works on every machine.
- **Light on bandwidth**: Large outputs and files stay on the machine that made
  them. The browser loads them only when you open them.
- **Native**: The server, the runner and the agent's tools are written in Rust.
  The web app is built with Vue and TypeScript.

Read the [design docs](docs/README.md), starting with the
[overview](docs/overview.md).

## Comparison

As of October 2026, based on each product's documentation.

| | Demi | [Pi](https://github.com/earendil-works/pi) | [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) | [OpenHands](https://github.com/OpenHands/OpenHands) | [opencode](https://github.com/anomalyco/opencode) | [Orca](https://github.com/stablyai/orca) | [herdr](https://github.com/herdrdev/herdr) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Source code | Public ([Demi License](LICENSE)) | MIT | MIT | MIT | MIT | MIT | Apache-2.0 |
| Harness | Its own | Its own | Its own | Its own, or wraps ACP agents | Its own | Wraps Claude Code, Codex, OpenCode, Pi and others | Wraps Claude Code, Codex, OpenCode, Pi and others |
| Interface | Web app for many users, on any device | Terminal | Local web app and terminal | Web app | Terminal, web, desktop | Desktop app, mobile companion | Terminal UI |
| Where conversations live | On your server, one place for all devices | On each machine | On each machine | On each backend | On each machine or server | On each machine | On each machine |
| Where tasks run | Cloud sandbox or any of your computers | The machine it runs on | The machine it runs on | Local, Docker, VM or cloud | The machine it runs on | Local worktrees, SSH hosts or your cloud VMs | Local or SSH machines |
| Several machines in one conversation | Yes | — | — | — | No | No | No |
| Tool design | Shell only | Tool calls; optional code mode | Tool calls; optional code mode | Tool calls | Tool calls; code mode for MCP tools | Those of the wrapped agent | Those of the wrapped agent |
| Models | Any, including Claude, ChatGPT and Grok subscriptions | Many providers and subscriptions | DeepSeek and others by API key | Any | Any | Those of the wrapped agents | Those of the wrapped agents |
| Browser automation | On any of your machines | Through extensions | Experimental plugin | In the SDK | — | Built-in browser | — |
| Teams and admin | Yes | — | Shared access token | Enterprise edition | Shared password | — | — |

— Not mentioned in the product's documentation.

<details>
<summary>Sources</summary>

- Pi: [README](https://github.com/earendil-works/pi), [Sessions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/sessions.md), [Providers](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md), [Settings](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/settings.md)
- DeepSeek Harness: [README](https://github.com/deepseek-ai/deepseek-harness), [Architecture](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/architecture.md), [Providers](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/providers.md), [Tool catalog](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/tool-catalog.md), [Public deployments](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/public-deployments.md)
- OpenHands: [README](https://github.com/OpenHands/OpenHands), [Backends](https://docs.openhands.dev/openhands/usage/agent-canvas/backends), [Enterprise vs. open source](https://docs.openhands.dev/enterprise/enterprise-vs-oss), [Browser use](https://docs.openhands.dev/sdk/guides/agent-browser-use)
- opencode: [Web](https://opencode.ai/docs/web/), [Server](https://opencode.ai/docs/server/), [Tools](https://opencode.ai/docs/tools/), [Providers](https://opencode.ai/docs/providers/)
- Orca: [README](https://github.com/stablyai/orca), [Supported agents](https://www.onorca.dev/docs/agents/supported), [Ways to run](https://www.onorca.dev/docs/ways-to-run), [Browser](https://www.onorca.dev/docs/browser/overview)
- herdr: [README](https://github.com/herdrdev/herdr), [Agents](https://herdr.dev/docs/agents/), [Connecting machines](https://herdr.dev/docs/connecting-machines/), [Session state](https://herdr.dev/docs/session-state/)

</details>

## Documentation

- [Design docs](docs/README.md): how every part of Demi works.
- [Contributing](CONTRIBUTING.md): development setup, checks, and adding a
  provider.
- [Security](SECURITY.md): reporting vulnerabilities and where data is stored.

## License

Demi is released under the [Demi License](LICENSE), which is the Apache License
2.0 with additional conditions:

- **Free to use**: Use, modify and run Demi, including at your company for your
  own team.
- **Free to support**: Charge for deploying, integrating or supporting Demi
  that others run themselves.
- **Not for resale as a service**: Don't offer Demi to others for a fee as a
  hosted or multi-tenant service, or as a product that is substantially Demi.
- **Keep the branding**: Don't remove or change the Demi name, logo or
  copyright notices in the web app.

For other uses, contact [support@demi.sh](mailto:support@demi.sh).

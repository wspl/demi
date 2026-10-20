# Demi compared

**English** · [简体中文](comparison.zh-CN.md) · [繁體中文](comparison.zh-TW.md) · [日本語](comparison.ja.md) · [한국어](comparison.ko.md)

How Demi differs from agent apps of its kind, as of October 2026, from each product's documentation and source code. The table lists a row only where the products differ.

The table uses these symbols and labels:

- ✓ Supported
- ◐ Partly
- ✗ Not supported
- **Web**: used in the browser, with no app of its own
- **PWA**: installed from the browser as an app

The following table compares the products feature by feature:

| | Demi | [Pi][pi] | [DeepSeek Harness][deepseek-harness] | [OpenHands][openhands] | [opencode][opencode] | [Orca][orca] | [herdr][herdr] | [T3 Code][t3-code] |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **Models and subscriptions** | | | | | | | | |
| Claude subscription | ✓ | ✓ | ◐¹ | ✓ | ✗ | ✓ | ✓ | ✓ |
| ChatGPT, Grok and others | ✓ | ✓ | ◐¹ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Multiple accounts | ✓ | ✗ | ✗ | ✗ | ✓ | ✓ | ✗ | ✓ |
| Account rotation | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| Mid-task model switch | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ✗ | ✓ |
| **Interface** | | | | | | | | |
| Web app | ✓ | ✗ | ✓ | ✓ | ✓ | ✓ | ✗ | ✓ |
| Mobile | PWA | ✗ | Web | Web | Web | ✓ | ✗ | ✓ |
| Desktop app | PWA | ✗ | ✓ | ✓ | ✓ | ✓ | ✗ | ✓ |
| Terminal UI | ✗ | ✓ | ✗ | ✓ | ✓ | ✗ | ✓ | ✗ |
| **Execution** | | | | | | | | |
| Central server | ✓ | ✗ | ◐ | ◐ | ◐ | ◐ | ✗ | ✗ |
| Runs with all devices off | ✓ | ✗ | ◐ | ✓² | ◐ | ◐ | ◐ | ◐ |
| Synced conversations | ✓ | ✗ | ✗ | ◐ | ◐ | ◐ | ✗ | ◐ |
| Cloud sandbox | ✓ | ✗ | ✗ | ✓ | ✗ | ◐ | ✗ | ✗ |
| Remote machines | ✓ | ◐ | ◐ | ◐ | ◐ | ✓ | ✓ | ✓ |
| Multi-machine tasks | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| Machine switching | ✓ | ◐ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| **Agent** | | | | | | | | |
| Own agent | ✓ | ✓ | ✓ | ✓ | ✓ | ✗ | ✗ | ✗ |
| Shell-only tools | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| Multi-agent | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Agent profiles | ✓ | ◐ | ✓ | ✓ | ✓ | ✗ | ✗ | ✗ |
| Skills | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ✓ |
| MCP | ✗ | ✓ | ✓ | ✓ | ✓ | ◐ | ✗ | ✓ |
| Browser automation | ✓ | ✗ | ◐ | ✓ | ✓ | ✓ | ✗ | ✓ |
| Computer use | ✗ | ✗ | ◐³ | ✗ | ✗ | ✓ | ✗ | ◐⁴ |
| **Teams** | | | | | | | | |
| Multi-user | ✓ | ✗ | ✗ | ◐² | ✗ | ✗ | ✗ | ✗ |
| Admin console | ✓ | ✗ | ✗ | ◐² | ◐² | ✗ | ✗ | ✗ |
| Per-user isolation | ✓ | ✗ | ✗ | ◐² | ✗ | ✗ | ✗ | ✗ |

The numbered notes in the table say the following:

1. Only through the Claude Code and Codex subagents it starts. Its own subscription sign-in is in the code but has no entry in the app.
2. Paid cloud or enterprise edition only.
3. Experimental, turned on by hand.
4. iOS and Android simulators only, not the desktop.

[pi]: https://github.com/earendil-works/pi
[deepseek-harness]: https://github.com/deepseek-ai/deepseek-harness
[openhands]: https://github.com/OpenHands/OpenHands
[opencode]: https://github.com/anomalyco/opencode
[orca]: https://github.com/stablyai/orca
[herdr]: https://github.com/herdrdev/herdr
[t3-code]: https://github.com/pingdotgg/t3code

## What the rows mean

The table's rows mean the following:

- **Subscriptions**: signing in with a subscription instead of an API key. A subscription used through an agent the product runs, such as Claude Code or Codex, counts.
- **Multiple accounts**, **Account rotation**: done by the product itself, not by an agent it runs. Rotation moves work to another account when one reaches its limit.
- **Central server**: one server holds the agent and every conversation, and your computers only run its commands.
  - ◐: a server that runs commands only on its own machine.
  - ✗: each machine runs an agent of its own, or there is no server.
- **Runs with all devices off**: a task keeps going while your computers and phone are off.
  - ◐: only when you install it on a machine of your own that stays on.
- **Synced conversations**: every conversation is in one list and reads the same on every device, whichever machine it runs on.
  - ◐: synced only among the devices connected to one server.
- **Multi-machine tasks**: one conversation works on several machines.
- **Machine switching**: a conversation moves to another machine and goes on.
- **Own agent**: the product's own agent drives the model, rather than running Claude Code, Codex or other agents.
- **Shell-only tools**: the model gets one tool, a shell, and every capability is a command in it.
- **Computer use**: the agent works the desktop with screenshots, mouse and keyboard, beyond a browser.

<details>
<summary>Sources</summary>

- Pi: [README](https://github.com/earendil-works/pi), [Providers](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md), [OAuth sign-ins](https://github.com/earendil-works/pi/tree/main/packages/ai/src/auth/oauth), [Credential store](https://github.com/earendil-works/pi/blob/main/packages/ai/src/auth/credential-store.ts), [SSH extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/ssh.ts), [Subagent extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/subagent/README.md), [Containerization](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/containerization.md), [Termux](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/termux.md), [Slash commands](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/slash-commands.md)
- DeepSeek Harness: [README](https://github.com/deepseek-ai/deepseek-harness), [Providers](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/providers.md), [Claude Code subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-claude-code/README.md), [Codex subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-codex/README.md), [Sign-in flows](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/llm/llm-pi-ai/src/login.ts), [Public deployments](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/public-deployments.md), [SSH](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/ssh.md), [Persistence](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/persistence.md), [Tool catalog](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/tool-catalog.md), [Agent presets](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/preset/agent-preset/README.md), [Browser use](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/browser-use.md), [Computer use](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/computer-use/README.md), [Desktop app](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/desktop/README.md)
- OpenHands: [README](https://github.com/OpenHands/OpenHands), [ACP agents](https://docs.openhands.dev/openhands/usage/agent-canvas/acp-agents), [Subscription sign-in](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/openai.py), [Credentials](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/credentials.py), [Backends](https://docs.openhands.dev/openhands/usage/agent-canvas/backends), [Remote backend](https://docs.openhands.dev/openhands/usage/agent-canvas/backend-setup/remote), [Phone and tablet access](https://docs.openhands.dev/openhands/usage/agent-canvas/mobile-access), [Cloud UI](https://docs.openhands.dev/openhands/usage/cloud/cloud-ui), [Enterprise vs. open source](https://docs.openhands.dev/enterprise/enterprise-vs-oss), [Tools](https://github.com/OpenHands/software-agent-sdk/tree/main/openhands-tools/openhands/tools)
- opencode: [Providers](https://opencode.ai/docs/providers/), [Provider plugins](https://github.com/anomalyco/opencode/tree/dev/packages/core/src/plugin/provider), [Web](https://opencode.ai/docs/web/), [Server](https://opencode.ai/docs/server/), [TUI](https://opencode.ai/docs/tui/), [Agents](https://opencode.ai/docs/agents/), [MCP servers](https://opencode.ai/docs/mcp-servers/), [Browser plugin](https://github.com/anomalyco/opencode/blob/dev/packages/plugin-browser/README.md), [Enterprise](https://opencode.ai/docs/enterprise/)
- Orca: [README](https://github.com/stablyai/orca), [Supported agents](https://www.onorca.dev/docs/agents/supported), [Accounts](https://www.onorca.dev/docs/agents/codex-hot-swap), [Ways to run](https://www.onorca.dev/docs/ways-to-run), [Remote servers](https://www.onorca.dev/docs/remote-servers), [Mobile](https://www.onorca.dev/docs/mobile), [Computer use](https://www.onorca.dev/docs/cli/computer-use), [Browser](https://www.onorca.dev/docs/browser/overview)
- herdr: [README](https://github.com/herdrdev/herdr), [Agents](https://herdr.dev/docs/agents/), [Connecting machines](https://herdr.dev/docs/connecting-machines/), [Persistence and remote](https://herdr.dev/docs/persistence-remote/), [Work from your phone](https://herdr.dev/docs/how-to-work/#work-from-your-phone), [Agent skill](https://herdr.dev/docs/agent-skill/)
- T3 Code: [README](https://github.com/pingdotgg/t3code), [Claude](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-claude.md), [Codex](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-codex.md), [Remote access](https://github.com/pingdotgg/t3code/blob/main/docs/user/remote-access.md), [Background service](https://github.com/pingdotgg/t3code/blob/main/docs/user/background-service.md), [Devices](https://github.com/pingdotgg/t3code/blob/main/docs/user/devices.md), [Outside agents](https://github.com/pingdotgg/t3code/blob/main/docs/user/outside-agents.md)

</details>

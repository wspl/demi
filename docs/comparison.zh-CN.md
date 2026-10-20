# Demi 对比

[English](comparison.md) · **简体中文** · [繁體中文](comparison.zh-TW.md) · [日本語](comparison.ja.md) · [한국어](comparison.ko.md)

Demi 与同类 Agent 应用的差异，截至 2026 年 10 月，依据各产品的文档和源码整理。只列出各产品之间有差异的项。

表中使用以下符号和标签：

- ✓ 支持
- ◐ 部分支持
- ✗ 不支持
- **Web**：在浏览器里使用，没有独立应用
- **PWA**：从浏览器安装成应用

下表逐项对比各产品：

| | Demi | [Pi][pi] | [DeepSeek Harness][deepseek-harness] | [OpenHands][openhands] | [opencode][opencode] | [Orca][orca] | [herdr][herdr] | [T3 Code][t3-code] |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **模型与订阅** | | | | | | | | |
| Claude 订阅 | ✓ | ✓ | ◐¹ | ✓ | ✗ | ✓ | ✓ | ✓ |
| ChatGPT、Grok 等订阅 | ✓ | ✓ | ◐¹ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 多账号 | ✓ | ✗ | ✗ | ✗ | ✓ | ✓ | ✗ | ✓ |
| 账号轮换 | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| 任务中切换模型 | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ✗ | ✓ |
| **界面** | | | | | | | | |
| 网页版 | ✓ | ✗ | ✓ | ✓ | ✓ | ✓ | ✗ | ✓ |
| 手机 | PWA | ✗ | Web | Web | Web | ✓ | ✗ | ✓ |
| 桌面应用 | PWA | ✗ | ✓ | ✓ | ✓ | ✓ | ✗ | ✓ |
| 终端界面 | ✗ | ✓ | ✗ | ✓ | ✓ | ✗ | ✓ | ✗ |
| **执行** | | | | | | | | |
| 中心服务器 | ✓ | ✗ | ◐ | ◐ | ◐ | ◐ | ✗ | ✗ |
| 设备全关也能运行 | ✓ | ✗ | ◐ | ✓² | ◐ | ◐ | ◐ | ◐ |
| 对话同步 | ✓ | ✗ | ✗ | ◐ | ◐ | ◐ | ✗ | ◐ |
| 云端沙箱 | ✓ | ✗ | ✗ | ✓ | ✗ | ◐ | ✗ | ✗ |
| 远程机器 | ✓ | ◐ | ◐ | ◐ | ◐ | ✓ | ✓ | ✓ |
| 一个任务用多台机器 | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| 中途换机器 | ✓ | ◐ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| **Agent** | | | | | | | | |
| 自研 Agent | ✓ | ✓ | ✓ | ✓ | ✓ | ✗ | ✗ | ✗ |
| 只用 Shell 一个工具 | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| 多 Agent | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Agent 配置 | ✓ | ◐ | ✓ | ✓ | ✓ | ✗ | ✗ | ✗ |
| Skills | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ✓ |
| MCP | ✗ | ✓ | ✓ | ✓ | ✓ | ◐ | ✗ | ✓ |
| 浏览器操作 | ✓ | ✗ | ◐ | ✓ | ✓ | ✓ | ✗ | ✓ |
| 电脑操作 | ✗ | ✗ | ◐³ | ✗ | ✗ | ✓ | ✗ | ◐⁴ |
| **团队** | | | | | | | | |
| 多用户 | ✓ | ✗ | ✗ | ◐² | ✗ | ✗ | ✗ | ✗ |
| 管理后台 | ✓ | ✗ | ✗ | ◐² | ◐² | ✗ | ✗ | ✗ |
| 用户隔离 | ✓ | ✗ | ✗ | ◐² | ✗ | ✗ | ✗ | ✗ |

表中的编号注释如下：

1. 只能通过它启动的 Claude Code、Codex 子 Agent 使用。它自己的订阅登录已写在代码里，但应用里没有入口。
2. 仅付费的云端版或企业版。
3. 实验功能，需要手动开启。
4. 只能操作 iOS 和 Android 模拟器，不能操作电脑桌面。

[pi]: https://github.com/earendil-works/pi
[deepseek-harness]: https://github.com/deepseek-ai/deepseek-harness
[openhands]: https://github.com/OpenHands/OpenHands
[opencode]: https://github.com/anomalyco/opencode
[orca]: https://github.com/stablyai/orca
[herdr]: https://github.com/herdrdev/herdr
[t3-code]: https://github.com/pingdotgg/t3code

## 各项的含义

各行的含义如下：

- **订阅**：用订阅账号登录，而不是 API Key。通过产品运行的 Claude Code、Codex 等 Agent 用上的订阅，也算支持。
- **多账号**、**账号轮换**：要产品自己做到，不算它运行的 Agent 的能力。轮换是指一个账号用到额度上限时，自动换另一个账号继续。
- **中心服务器**：一台服务器保存 Agent 和所有对话，你的各台电脑只负责执行命令。
  - ◐：有服务器，但只能在它自己那台机器上执行。
  - ✗：每台机器各跑一个 Agent，或者没有服务器。
- **设备全关也能运行**：你的电脑和手机都关了，任务照样继续。
  - ◐：要你自己准备一台一直开着的机器，把它装上去。
- **对话同步**：不管对话在哪台机器上执行，所有对话都在同一个列表里，在任何设备上看到的都一样。
  - ◐：只有连同一台服务器的设备之间同步。
- **一个任务用多台机器**：一个对话同时在几台机器上干活。
- **中途换机器**：对话换到另一台机器上接着做。
- **自研 Agent**：由产品自己的 Agent 驱动模型，而不是运行 Claude Code、Codex 等现成的 Agent。
- **只用 Shell 一个工具**：模型只拿到一个工具，就是 Shell，所有能力都是 Shell 里的命令。
- **电脑操作**：Agent 通过截屏、鼠标和键盘操作电脑桌面，不限于浏览器。

<details>
<summary>出处</summary>

- Pi: [README](https://github.com/earendil-works/pi), [Providers](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md), [OAuth sign-ins](https://github.com/earendil-works/pi/tree/main/packages/ai/src/auth/oauth), [Credential store](https://github.com/earendil-works/pi/blob/main/packages/ai/src/auth/credential-store.ts), [SSH extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/ssh.ts), [Subagent extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/subagent/README.md), [Containerization](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/containerization.md), [Termux](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/termux.md), [Slash commands](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/slash-commands.md)
- DeepSeek Harness: [README](https://github.com/deepseek-ai/deepseek-harness), [Providers](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/providers.md), [Claude Code subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-claude-code/README.md), [Codex subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-codex/README.md), [Sign-in flows](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/llm/llm-pi-ai/src/login.ts), [Public deployments](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/public-deployments.md), [SSH](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/ssh.md), [Persistence](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/persistence.md), [Tool catalog](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/tool-catalog.md), [Agent presets](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/preset/agent-preset/README.md), [Browser use](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/browser-use.md), [Computer use](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/computer-use/README.md), [Desktop app](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/desktop/README.md)
- OpenHands: [README](https://github.com/OpenHands/OpenHands), [ACP agents](https://docs.openhands.dev/openhands/usage/agent-canvas/acp-agents), [Subscription sign-in](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/openai.py), [Credentials](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/credentials.py), [Backends](https://docs.openhands.dev/openhands/usage/agent-canvas/backends), [Remote backend](https://docs.openhands.dev/openhands/usage/agent-canvas/backend-setup/remote), [Phone and tablet access](https://docs.openhands.dev/openhands/usage/agent-canvas/mobile-access), [Cloud UI](https://docs.openhands.dev/openhands/usage/cloud/cloud-ui), [Enterprise vs. open source](https://docs.openhands.dev/enterprise/enterprise-vs-oss), [Tools](https://github.com/OpenHands/software-agent-sdk/tree/main/openhands-tools/openhands/tools)
- opencode: [Providers](https://opencode.ai/docs/providers/), [Provider plugins](https://github.com/anomalyco/opencode/tree/dev/packages/core/src/plugin/provider), [Web](https://opencode.ai/docs/web/), [Server](https://opencode.ai/docs/server/), [TUI](https://opencode.ai/docs/tui/), [Agents](https://opencode.ai/docs/agents/), [MCP servers](https://opencode.ai/docs/mcp-servers/), [Browser plugin](https://github.com/anomalyco/opencode/blob/dev/packages/plugin-browser/README.md), [Enterprise](https://opencode.ai/docs/enterprise/)
- Orca: [README](https://github.com/stablyai/orca), [Supported agents](https://www.onorca.dev/docs/agents/supported), [Accounts](https://www.onorca.dev/docs/agents/codex-hot-swap), [Ways to run](https://www.onorca.dev/docs/ways-to-run), [Remote servers](https://www.onorca.dev/docs/remote-servers), [Mobile](https://www.onorca.dev/docs/mobile), [Computer use](https://www.onorca.dev/docs/cli/computer-use), [Browser](https://www.onorca.dev/docs/browser/overview)
- herdr: [README](https://github.com/herdrdev/herdr), [Agents](https://herdr.dev/docs/agents/), [Connecting machines](https://herdr.dev/docs/connecting-machines/), [Persistence and remote](https://herdr.dev/docs/persistence-remote/), [Work from your phone](https://herdr.dev/docs/how-to-work/#work-from-your-phone), [Agent skill](https://herdr.dev/docs/agent-skill/)
- T3 Code: [README](https://github.com/pingdotgg/t3code), [Claude](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-claude.md), [Codex](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-codex.md), [Remote access](https://github.com/pingdotgg/t3code/blob/main/docs/user/remote-access.md), [Background service](https://github.com/pingdotgg/t3code/blob/main/docs/user/background-service.md), [Devices](https://github.com/pingdotgg/t3code/blob/main/docs/user/devices.md), [Outside agents](https://github.com/pingdotgg/t3code/blob/main/docs/user/outside-agents.md)

</details>

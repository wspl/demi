# Demi 对比

[English](comparison.md) · **简体中文** · [繁體中文](comparison.zh-TW.md) · [日本語](comparison.ja.md) · [한국어](comparison.ko.md)

Demi 与同类 Agent 应用的差异，截至 2026 年 10 月，依据各产品的文档和源码整理。只列出各产品之间有差异的项，以及一项目前都不支持的能力。各列按与 Demi 的接近程度，从最接近排到最不接近。

表中使用以下符号和标签：

- ✓ 支持
- ◐ 部分支持
- 空白：不支持
- **Web**：在浏览器里使用，没有独立应用
- **PWA**：从浏览器安装成应用

下表逐项对比各产品：

| | Demi | [OpenHands][openhands] | [Orca][orca] | [T3 Code][t3-code] | [DeepSeek Harness][deepseek-harness] | [opencode][opencode] | [Pi][pi] | [herdr][herdr] |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **模型与订阅** | | | | | | | | |
| Claude 订阅 | ✓ | ✓ | ✓ | ✓ | ◐¹ | | ✓ | ✓ |
| ChatGPT、Grok 等订阅 | ✓ | ✓ | ✓ | ✓ | ◐¹ | ✓ | ✓ | ✓ |
| 多账号 | ✓ | | ✓ | ✓ | | ✓ | | |
| 账号轮换 | ✓ | | | | | | | |
| 订阅用量和重置时间 | ✓ | ◐² | ✓ | ✓ | ◐³ | | | |
| 任务中切换模型 | ✓ | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | |
| **界面** | | | | | | | | |
| 网页版 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | | |
| 手机 | PWA | Web | ✓ | ✓ | Web | Web | | |
| 桌面应用 | PWA | ✓ | ✓ | ✓ | ✓ | ✓ | | |
| 终端界面 | | ✓ | | | | ✓ | ✓ | ✓ |
| 语音输入 | | ✓ | ✓ | ✓ | ◐⁴ | | | |
| 推送到手机 | ✓ | | ✓ | ✓⁵ | | | | ◐⁶ |
| 搜索全部对话 | ✓ | | ◐⁷ | ✓ | ◐⁸ | ◐⁸ | ✓ | |
| **执行** | | | | | | | | |
| 中心服务器 | ✓ | ◐ | ◐ | | ◐ | ◐ | | |
| 设备全关也能运行 | ✓ | ✓⁹ | ◐ | ◐ | ◐ | ◐ | | ◐ |
| 对话同步 | ✓ | ◐ | ◐ | ◐ | | ◐ | | |
| 云端沙箱 | ✓ | ✓ | ◐ | | | | | |
| 远程机器 | ✓ | ◐ | ✓ | ✓ | ◐ | ◐ | ◐ | ✓ |
| 一个任务用多台机器 | ✓ | | | | | | | |
| 中途换机器 | ✓ | | | | | | ◐ | |
| 端口转发 | ✓ | | ✓¹⁰ | | | | | |
| 定时任务 | ✓ | ✓ | ✓ | ✓ | ✓ | ◐¹¹ | | |
| **文件** | | | | | | | | |
| 文件浏览器 | ✓ | ✓ | ✓ | ✓ | ◐¹² | ✓ | | |
| 文件管理 | ◐¹³ | ◐¹⁴ | ✓ | ◐¹⁵ | | | | |
| 搜索文件内容 | ✓ | | ✓ | ✓ | | | | |
| Git worktree | ◐¹⁶ | ✓ | ✓ | ✓ | ◐⁴ | ✓ | | ✓ |
| 在 diff 的行上评论 | | | ✓ | ◐¹⁷ | | ✓ | | |
| **Agent** | | | | | | | | |
| 自研 Agent | ✓ | ✓ | | | ✓ | ✓ | ✓ | |
| 只用 Shell 一个工具 | ✓ | | | | | | | |
| 多 Agent | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ✓ |
| Agent 配置 | ✓ | ✓ | | | ✓ | ✓ | ◐ | |
| Skills | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ |
| MCP | | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | |
| 第三方插件 | | ✓ | ◐⁴ | | ✓ | ✓ | ✓ | ✓ |
| 给 Agent 用的密钥 | ✓ | ✓ | | | | | | |
| 浏览器操作 | ✓ | ✓ | ✓ | ✓ | ◐ | ◐¹⁸ | | |
| 实时查看和操作浏览器 | ✓ | ◐¹⁹ | ✓ | ✓ | | ◐¹⁸ | | |
| 电脑操作 | | | ◐¹⁸ | ◐²⁰ | ◐⁴ | | | |
| **手机设备** | | | | | | | | |
| iOS 模拟器 | ✓ | | ◐¹⁸ | ✓ | | | | |
| iOS 真机 | | | | | | | | |
| Android 模拟器 | ✓ | | ◐¹⁸ | ✓ | | | | |
| Android 真机 | | | ◐¹⁸ | ✓ | | | | |
| **媒体生成** | | | | | | | | |
| 图片生成与编辑 | ✓ | ◐²¹ | ◐²¹ | ◐²¹ | ◐²¹ | | ◐²² | ◐²¹ |
| 视频生成 | ✓ | | | | | | | |
| 语音、音乐和音效 | ✓ | | | | | | | |
| **团队** | | | | | | | | |
| 多用户 | ✓ | ◐⁹ | | | | | | |
| 管理后台 | ✓ | ◐⁹ | | | | ◐⁹ | | |
| 用户隔离 | ✓ | ◐⁹ | | | | | | |
| 分享对话链接 | ✓ | ◐²³ | | ◐²⁴ | ◐⁴ | | ✓ | |

表中的编号注释如下：

1. 只能通过它启动的 Claude Code、Codex 子 Agent 使用。它自己的订阅登录已写在代码里，但应用里没有入口。
2. 按 API 价格估算的套餐用量。
3. 只显示 DeepSeek 自家账号的额度。
4. 实验功能，需要手动开启。
5. 只能通过 T3 Connect。
6. 只有一个推送到 Telegram 的示例插件。
7. 按标题、目录、分支、模型和预览搜，不搜全文。
8. 只按标题搜。
9. 仅付费的云端版或企业版。
10. 只能从 SSH 主机转发到这台电脑，会自动找出主机上正在监听的端口。
11. 只能借助 GitHub Actions 的 cron 定时运行。
12. 只能看它所在那台机器的文件，网页界面看不到 SSH 主机上的文件。
13. 能上传、下载、新建文件夹，不能重命名、删除、移动和编辑。
14. 只能下载整个工作区。
15. 只能编辑文件内容。
16. 只把已有的 worktree 显示在所属项目下，不能新建。
17. 可以引用任意一段文字并附上评论，不是针对 diff 的某一行。
18. 只能在运行它桌面应用的那台电脑上用，网页客户端和远程机器上都不行。
19. 只显示 Agent 最近一张截图，不能操作。
20. 只能操作 iOS 和 Android 模拟器，不能操作电脑桌面。
21. 靠它运行的 Codex 自带的生图工具。
22. 只能在 codemode 脚本里调用，codemode 要手动开启。
23. 只有用 OpenHands Cloud 后端时才行。
24. Guest links，测试版，需要 T3 Connect。

[openhands]: https://github.com/OpenHands/OpenHands
[orca]: https://github.com/stablyai/orca
[t3-code]: https://github.com/pingdotgg/t3code
[deepseek-harness]: https://github.com/deepseek-ai/deepseek-harness
[opencode]: https://github.com/anomalyco/opencode
[pi]: https://github.com/earendil-works/pi
[herdr]: https://github.com/herdrdev/herdr

## 各项的含义

各行的含义如下：

- **跟机器有关的行**（文件、worktree、diff、浏览器、电脑操作、手机设备）：Agent 在远程机器上工作时也能用，才算 ✓，例如 SSH 主机、另一台电脑或云端机器；只能在运行产品的那台电脑上用的，算 ◐。
- **订阅**：用订阅账号登录，而不是 API Key。通过产品运行的 Claude Code、Codex 等 Agent 用上的订阅，也算支持。
- **多账号**、**账号轮换**：要产品自己做到，不算它运行的 Agent 的能力。轮换是指一个账号用到额度上限时，自动换另一个账号继续。
- **订阅用量和重置时间**：每个订阅的额度用了多少，什么时候重置。
- **推送到手机**：应用关着时，通知也能送到手机上。
- **搜索全部对话**：在所有对话里搜任意一条消息里的文字。
- **中心服务器**：一台服务器保存 Agent 和所有对话，你的各台电脑只负责执行命令。
  - ◐：有服务器，但只能在它自己那台机器上执行。
  - 空白：每台机器各跑一个 Agent，或者没有服务器。
- **设备全关也能运行**：你的电脑和手机都关了，任务照样继续。
  - ◐：要你自己准备一台一直开着的机器，把它装上去。
- **对话同步**：不管对话在哪台机器上执行，所有对话都在同一个列表里，在任何设备上看到的都一样。
  - ◐：只有连同一台服务器的设备之间同步。
- **一个任务用多台机器**：一个对话同时在几台机器上干活。
- **中途换机器**：对话换到另一台机器上接着做。
- **端口转发**：一台机器上的程序能访问另一台机器的端口，就像 `ssh -L` 那样。
- **定时任务**：任务在设定的时间或间隔自己开始。
- **文件管理**：在应用里上传、下载、新建、重命名、删除、移动和编辑文件。
- **在 diff 的行上评论**：对改动的某一行写评论，评论会交给 Agent。
- **自研 Agent**：由产品自己的 Agent 驱动模型，而不是运行 Claude Code、Codex 等现成的 Agent。
- **只用 Shell 一个工具**：模型只拿到一个工具，就是 Shell，所有能力都是 Shell 里的命令。
- **第三方插件**：用户能装别人写的代码，给产品加上工具或界面，不算 Skills 和 MCP。Demi 自己的功能都是插件，但不加载别人的插件。
- **给 Agent 用的密钥**：用户按名字保存的密钥，Agent 在命令里使用。
- **实时查看和操作浏览器**：用户能实时看到 Agent 在用的浏览器，也能自己上手操作。
- **电脑操作**：Agent 通过截屏、鼠标和键盘操作电脑桌面，不限于浏览器。
- **iOS 真机**、**Android 真机**：连在电脑上的真手机，不是模拟器。
- **分享对话链接**：用一个链接把对话给别人看。

<details>
<summary>出处</summary>

- OpenHands: [README](https://github.com/OpenHands/OpenHands), [ACP agents](https://docs.openhands.dev/openhands/usage/agent-canvas/acp-agents), [Subscription sign-in](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/openai.py), [Credentials](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/credentials.py), [Backends](https://docs.openhands.dev/openhands/usage/agent-canvas/backends), [Remote backend](https://docs.openhands.dev/openhands/usage/agent-canvas/backend-setup/remote), [Phone and tablet access](https://docs.openhands.dev/openhands/usage/agent-canvas/mobile-access), [Cloud UI](https://docs.openhands.dev/openhands/usage/cloud/cloud-ui), [Enterprise vs. open source](https://docs.openhands.dev/enterprise/enterprise-vs-oss), [Tools](https://github.com/OpenHands/software-agent-sdk/tree/main/openhands-tools/openhands/tools), [Automations](https://docs.openhands.dev/openhands/usage/agent-canvas/prebuilt-automations), [Secrets](https://github.com/OpenHands/OpenHands/blob/main/src/routes/secrets-settings.tsx), [Files](https://github.com/OpenHands/OpenHands/blob/main/src/routes/files-tab.tsx), [Workspace download](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/conversation-panel/conversation-card/conversation-card-context-menu.tsx), [Browser panel](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/browser/browser.tsx), [Public share](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/conversation/conversation-name-context-menu.tsx), [Dictation](https://github.com/OpenHands/OpenHands/blob/main/src/hooks/chat/use-dictation.ts), [Worktrees](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/chat/workspace-mode-selector.tsx), [Usage](https://github.com/OpenHands/OpenHands/blob/main/src/routes/usage-tab.tsx), [Plugins](https://github.com/OpenHands/OpenHands/blob/main/src/routes/skills-plugins.tsx)
- Orca: [README](https://github.com/stablyai/orca), [Supported agents](https://www.onorca.dev/docs/agents/supported), [Accounts](https://www.onorca.dev/docs/agents/codex-hot-swap), [Ways to run](https://www.onorca.dev/docs/ways-to-run), [Remote servers](https://www.onorca.dev/docs/remote-servers), [Mobile](https://www.onorca.dev/docs/mobile), [Computer use](https://www.onorca.dev/docs/cli/computer-use), [Browser](https://www.onorca.dev/docs/browser/overview), [File explorer](https://www.onorca.dev/docs/editing/file-explorer), [Editor](https://www.onorca.dev/docs/editing/monaco), [Session history](https://www.onorca.dev/docs/agents/session-history), [Worktrees](https://www.onorca.dev/docs/model/worktrees), [Annotate AI diffs](https://www.onorca.dev/docs/review/annotate-ai-diff), [SSH and port forwarding](https://www.onorca.dev/docs/ssh), [Automations](https://www.onorca.dev/docs/cli/automations), [Usage tracking](https://www.onorca.dev/docs/agents/usage-tracking), [Settings and plugins](https://www.onorca.dev/docs/settings), [iOS emulator skill](https://github.com/stablyai/orca/blob/main/skills/orca-emulator/SKILL.md), [Android emulator skill](https://github.com/stablyai/orca/blob/main/skills/orca-emulator-android/SKILL.md), [Client creation policy](https://github.com/stablyai/orca/blob/main/src/renderer/src/lib/client-creation-action-policy.ts)
- T3 Code: [README](https://github.com/pingdotgg/t3code), [Claude](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-claude.md), [Codex](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-codex.md), [Remote access](https://github.com/pingdotgg/t3code/blob/main/docs/user/remote-access.md), [Background service](https://github.com/pingdotgg/t3code/blob/main/docs/user/background-service.md), [Devices](https://github.com/pingdotgg/t3code/blob/main/docs/user/devices.md), [Outside agents](https://github.com/pingdotgg/t3code/blob/main/docs/user/outside-agents.md), [Usage and limits](https://github.com/pingdotgg/t3code/blob/main/docs/user/usage.md), [Composer](https://github.com/pingdotgg/t3code/blob/main/docs/user/composer.md), [Mobile notifications](https://github.com/pingdotgg/t3code/blob/main/docs/user/mobile-notifications.md), [Scheduled tasks](https://github.com/pingdotgg/t3code/blob/main/docs/user/project-settings.md), [Guest links](https://github.com/pingdotgg/t3code/blob/main/docs/user/guest-links.md), [Thread sidebar](https://github.com/pingdotgg/t3code/blob/main/docs/user/thread-sidebar.md), [Thread search](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/orchestration-v2/ThreadSearch.ts), [File browser](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/files/FileBrowserPanel.tsx), [File saving](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/files/fileSaveCoordinator.ts), [Content search](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/search/ProjectContentSearchDialog.tsx)
- DeepSeek Harness: [README](https://github.com/deepseek-ai/deepseek-harness), [Providers](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/providers.md), [Claude Code subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-claude-code/README.md), [Codex subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-codex/README.md), [Sign-in flows](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/llm/llm-pi-ai/src/login.ts), [Public deployments](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/public-deployments.md), [SSH](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/ssh.md), [Persistence](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/persistence.md), [Tool catalog](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/tool-catalog.md), [Agent presets](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/preset/agent-preset/README.md), [Browser use](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/browser-use.md), [Computer use](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/computer-use/README.md), [Desktop app](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/desktop/README.md), [Schedule](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/schedule.md), [Files sidebar](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-sidebar-files/README.md), [Workspace](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-workspace/README.md), [Plugin indexes](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/plugin-market.md), [Account quota](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-settings-account/src/client/AccountQuotaNotice.tsx), [Voice input](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/voice-input-bundle/README.md), [Liveshare](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/liveshare-protocol/README.md), [Worktree](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/worktree/README.md)
- opencode: [Providers](https://opencode.ai/docs/providers/), [Provider plugins](https://github.com/anomalyco/opencode/tree/dev/packages/core/src/plugin/provider), [Web](https://opencode.ai/docs/web/), [Server](https://opencode.ai/docs/server/), [TUI](https://opencode.ai/docs/tui/), [Agents](https://opencode.ai/docs/agents/), [MCP servers](https://opencode.ai/docs/mcp-servers/), [Browser plugin](https://github.com/anomalyco/opencode/blob/dev/packages/plugin-browser/README.md), [Enterprise](https://opencode.ai/docs/enterprise/), [GitHub](https://opencode.ai/docs/github/), [Plugins](https://opencode.ai/docs/plugins/), [Files](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/workspaces/files/model.tsx), [Workspaces](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/workspaces/submenu.tsx), [Session search](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/home/sessions/search.ts), [Review comments](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/composer/comment-note.ts), [AI library](https://github.com/anomalyco/opencode/blob/dev/packages/ai/README.md)
- Pi: [README](https://github.com/earendil-works/pi), [Providers](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md), [OAuth sign-ins](https://github.com/earendil-works/pi/tree/main/packages/ai/src/auth/oauth), [Credential store](https://github.com/earendil-works/pi/blob/main/packages/ai/src/auth/credential-store.ts), [SSH extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/ssh.ts), [Subagent extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/subagent/README.md), [Containerization](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/containerization.md), [Termux](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/termux.md), [Slash commands](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/slash-commands.md), [Extensions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md), [Packages](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/packages.md), [Codemode](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/codemode.md), [Sessions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/sessions.md)
- herdr: [README](https://github.com/herdrdev/herdr), [Agents](https://herdr.dev/docs/agents/), [Connecting machines](https://herdr.dev/docs/connecting-machines/), [Persistence and remote](https://herdr.dev/docs/persistence-remote/), [Work from your phone](https://herdr.dev/docs/how-to-work/#work-from-your-phone), [Agent skill](https://herdr.dev/docs/agent-skill/), [Configuration](https://herdr.dev/docs/configuration/), [Plugins](https://herdr.dev/docs/plugins/)
- Codex: [Image generation skill](https://github.com/openai/codex/blob/main/codex-rs/skills/src/assets/samples/imagegen/SKILL.md)

</details>

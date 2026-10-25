# Demi compared

**English** · [简体中文](comparison.zh-CN.md) · [繁體中文](comparison.zh-TW.md) · [日本語](comparison.ja.md) · [한국어](comparison.ko.md)

How Demi differs from agent apps of its kind, as of October 2026, from each product's documentation and source code. The table lists a row only where the products differ, and one capability that none of them has yet. Its columns go from the product closest to Demi to the furthest.

The table uses these symbols and labels:

- ✓ Supported
- ◐ Partly
- Blank: not supported
- **Web**: used in the browser, with no app of its own
- **PWA**: installed from the browser as an app

The following table compares the products feature by feature:

| | Demi | [OpenHands][openhands] | [Orca][orca] | [T3 Code][t3-code] | [DeepSeek Harness][deepseek-harness] | [opencode][opencode] | [Pi][pi] | [herdr][herdr] |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **Models and subscriptions** | | | | | | | | |
| Claude subscription | ✓ | ✓ | ✓ | ✓ | ◐¹ | | ✓ | ✓ |
| ChatGPT, Grok and others | ✓ | ✓ | ✓ | ✓ | ◐¹ | ✓ | ✓ | ✓ |
| Multiple accounts | ✓ | | ✓ | ✓ | | ✓ | | |
| Account rotation | ✓ | | | | | | | |
| Subscription usage | ✓ | ◐² | ✓ | ✓ | ◐³ | | | |
| Mid-task model switch | ✓ | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | |
| **Interface** | | | | | | | | |
| Web app | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | | |
| Mobile | PWA | Web | ✓ | ✓ | Web | Web | | |
| Desktop app | PWA | ✓ | ✓ | ✓ | ✓ | ✓ | | |
| Terminal UI | | ✓ | | | | ✓ | ✓ | ✓ |
| Voice input | | ✓ | ✓ | ✓ | ◐⁴ | | | |
| Phone notifications | ✓ | | ✓ | ✓⁵ | | | | ◐⁶ |
| Conversation search | ✓ | | ◐⁷ | ✓ | ◐⁸ | ◐⁸ | ✓ | |
| **Execution** | | | | | | | | |
| Central server | ✓ | ◐ | ◐ | | ◐ | ◐ | | |
| Runs with all devices off | ✓ | ✓⁹ | ◐ | ◐ | ◐ | ◐ | | ◐ |
| Synced conversations | ✓ | ◐ | ◐ | ◐ | | ◐ | | |
| Cloud sandbox | ✓ | ✓ | ◐ | | | | | |
| Remote machines | ✓ | ◐ | ✓ | ✓ | ◐ | ◐ | ◐ | ✓ |
| Multi-machine tasks | ✓ | | | | | | | |
| Machine switching | ✓ | | | | | | ◐ | |
| Port forwarding | ✓ | | ✓¹⁰ | | | | | |
| Scheduled tasks | ✓ | ✓ | ✓ | ✓ | ✓ | ◐¹¹ | | |
| **Files** | | | | | | | | |
| File browser | ✓ | ✓ | ✓ | ✓ | ◐¹² | ✓ | | |
| File management | ◐¹³ | ◐¹⁴ | ✓ | ◐¹⁵ | | | | |
| File content search | ✓ | | ✓ | ✓ | | | | |
| Git worktrees | ◐¹⁶ | ✓ | ✓ | ✓ | ◐⁴ | ✓ | | ✓ |
| Line comments on diffs | | | ✓ | ◐¹⁷ | | ✓ | | |
| **Agent** | | | | | | | | |
| Own agent | ✓ | ✓ | | | ✓ | ✓ | ✓ | |
| Shell-only tools | ✓ | | | | | | | |
| Multi-agent | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ✓ |
| Agent profiles | ✓ | ✓ | | | ✓ | ✓ | ◐ | |
| Skills | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ |
| MCP | | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | |
| Third-party plugins | | ✓ | ◐⁴ | | ✓ | ✓ | ✓ | ✓ |
| Secrets for the agent | ✓ | ✓ | | | | | | |
| Browser automation | ✓ | ✓ | ✓ | ✓ | ◐ | ◐¹⁸ | | |
| Live browser view and control | ✓ | ◐¹⁹ | ✓ | ✓ | | ◐¹⁸ | | |
| Computer use | | | ◐¹⁸ | ◐²⁰ | ◐⁴ | | | |
| **Mobile devices** | | | | | | | | |
| iOS Simulator | ✓ | | ◐¹⁸ | ✓ | | | | |
| iOS device | | | | | | | | |
| Android Emulator | ✓ | | ◐¹⁸ | ✓ | | | | |
| Android device | | | ◐¹⁸ | ✓ | | | | |
| **Media generation** | | | | | | | | |
| Image generation and editing | ✓ | ◐²¹ | ◐²¹ | ◐²¹ | ◐²¹ | | ◐²² | ◐²¹ |
| Video generation | ✓ | | | | | | | |
| Speech, music and sound effects | ✓ | | | | | | | |
| **Teams** | | | | | | | | |
| Multi-user | ✓ | ◐⁹ | | | | | | |
| Admin console | ✓ | ◐⁹ | | | | ◐⁹ | | |
| Per-user isolation | ✓ | ◐⁹ | | | | | | |
| Shared conversation links | ✓ | ◐²³ | | ◐²⁴ | ◐⁴ | | ✓ | |

The numbered notes in the table say the following:

1. Only through the Claude Code and Codex subagents it starts. Its own subscription sign-in is in the code but has no entry in the app.
2. An estimate of plan usage at API prices.
3. Only the quota of its own DeepSeek account.
4. Experimental, turned on by hand.
5. Through T3 Connect only.
6. Only through an example plugin that sends to Telegram.
7. By title, folder, branch, model and preview, not the whole text.
8. By title only.
9. Paid cloud or enterprise edition only.
10. From an SSH host to this computer only; it finds the host's listening ports for you.
11. Only as a GitHub Actions job on a cron schedule.
12. Only the files of the machine it runs on; its web views do not reach an SSH host.
13. Uploading, downloading and new folders; no renaming, deleting, moving or editing.
14. Downloading the whole workspace only.
15. Editing a file's contents only.
16. Shows existing worktrees under their project, but does not create one.
17. A comment on a quoted passage of any text, not on a line of a diff.
18. Only on the computer that runs its desktop app, not in its web client or on a remote machine.
19. Shows the agent's last screenshot, which cannot be operated.
20. iOS and Android simulators only, not the desktop.
21. Through the image tool of the Codex agent it runs.
22. Only from a codemode script, which is turned on by hand.
23. With an OpenHands Cloud backend only.
24. Guest links, a beta that needs T3 Connect.

[openhands]: https://github.com/OpenHands/OpenHands
[orca]: https://github.com/stablyai/orca
[t3-code]: https://github.com/pingdotgg/t3code
[deepseek-harness]: https://github.com/deepseek-ai/deepseek-harness
[opencode]: https://github.com/anomalyco/opencode
[pi]: https://github.com/earendil-works/pi
[herdr]: https://github.com/herdrdev/herdr

## What the rows mean

The table's rows mean the following:

- **Rows about a machine** (files, worktrees, diffs, the browser, computer use and mobile devices): a feature is ✓ only when it also works while the agent works on a remote machine, such as an SSH host, another computer or a cloud machine. One that works only on the computer that runs the product is ◐.
- **Subscriptions**: signing in with a subscription instead of an API key. A subscription used through an agent the product runs, such as Claude Code or Codex, counts.
- **Multiple accounts**, **Account rotation**: done by the product itself, not by an agent it runs. Rotation moves work to another account when one reaches its limit.
- **Subscription usage**: how much of each subscription's limits is used, and when each limit resets.
- **Phone notifications**: a notification reaches the phone while the app is closed.
- **Conversation search**: finds any words of any message across all conversations.
- **Central server**: one server holds the agent and every conversation, and your computers only run its commands.
  - ◐: a server that runs commands only on its own machine.
  - Blank: each machine runs an agent of its own, or there is no server.
- **Runs with all devices off**: a task keeps going while your computers and phone are off.
  - ◐: only when you install it on a machine of your own that stays on.
- **Synced conversations**: every conversation is in one list and reads the same on every device, whichever machine it runs on.
  - ◐: synced only among the devices connected to one server.
- **Multi-machine tasks**: one conversation works on several machines.
- **Machine switching**: a conversation moves to another machine and goes on.
- **Port forwarding**: a program on one machine reaches a port of another, as `ssh -L` makes it.
- **Scheduled tasks**: a task starts by itself at a set time or interval.
- **File management**: uploading, downloading, creating, renaming, deleting, moving and editing files in the app.
- **Line comments on diffs**: a comment on one line of a change, which goes to the agent.
- **Own agent**: the product's own agent drives the model, rather than running Claude Code, Codex or other agents.
- **Shell-only tools**: the model gets one tool, a shell, and every capability is a command in it.
- **Third-party plugins**: code from someone else that the user installs to add tools or screens to the product, beyond skills and MCP servers. Demi's own features are plugins, but it loads no one else's.
- **Secrets for the agent**: secrets the user saves by name, which the agent uses in its commands.
- **Live browser view and control**: the user watches the agent's browser as it works and can operate it.
- **Computer use**: the agent works the desktop with screenshots, mouse and keyboard, beyond a browser.
- **iOS device**, **Android device**: a real phone connected to the computer, not a simulator.
- **Shared conversation links**: a link that shows a conversation to someone else.

<details>
<summary>Sources</summary>

- OpenHands: [README](https://github.com/OpenHands/OpenHands), [ACP agents](https://docs.openhands.dev/openhands/usage/agent-canvas/acp-agents), [Subscription sign-in](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/openai.py), [Credentials](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/credentials.py), [Backends](https://docs.openhands.dev/openhands/usage/agent-canvas/backends), [Remote backend](https://docs.openhands.dev/openhands/usage/agent-canvas/backend-setup/remote), [Phone and tablet access](https://docs.openhands.dev/openhands/usage/agent-canvas/mobile-access), [Cloud UI](https://docs.openhands.dev/openhands/usage/cloud/cloud-ui), [Enterprise vs. open source](https://docs.openhands.dev/enterprise/enterprise-vs-oss), [Tools](https://github.com/OpenHands/software-agent-sdk/tree/main/openhands-tools/openhands/tools), [Automations](https://docs.openhands.dev/openhands/usage/agent-canvas/prebuilt-automations), [Secrets](https://github.com/OpenHands/OpenHands/blob/main/src/routes/secrets-settings.tsx), [Files](https://github.com/OpenHands/OpenHands/blob/main/src/routes/files-tab.tsx), [Workspace download](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/conversation-panel/conversation-card/conversation-card-context-menu.tsx), [Browser panel](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/browser/browser.tsx), [Public share](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/conversation/conversation-name-context-menu.tsx), [Dictation](https://github.com/OpenHands/OpenHands/blob/main/src/hooks/chat/use-dictation.ts), [Worktrees](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/chat/workspace-mode-selector.tsx), [Usage](https://github.com/OpenHands/OpenHands/blob/main/src/routes/usage-tab.tsx), [Plugins](https://github.com/OpenHands/OpenHands/blob/main/src/routes/skills-plugins.tsx)
- Orca: [README](https://github.com/stablyai/orca), [Supported agents](https://www.onorca.dev/docs/agents/supported), [Accounts](https://www.onorca.dev/docs/agents/codex-hot-swap), [Ways to run](https://www.onorca.dev/docs/ways-to-run), [Remote servers](https://www.onorca.dev/docs/remote-servers), [Mobile](https://www.onorca.dev/docs/mobile), [Computer use](https://www.onorca.dev/docs/cli/computer-use), [Browser](https://www.onorca.dev/docs/browser/overview), [File explorer](https://www.onorca.dev/docs/editing/file-explorer), [Editor](https://www.onorca.dev/docs/editing/monaco), [Session history](https://www.onorca.dev/docs/agents/session-history), [Worktrees](https://www.onorca.dev/docs/model/worktrees), [Annotate AI diffs](https://www.onorca.dev/docs/review/annotate-ai-diff), [SSH and port forwarding](https://www.onorca.dev/docs/ssh), [Automations](https://www.onorca.dev/docs/cli/automations), [Usage tracking](https://www.onorca.dev/docs/agents/usage-tracking), [Settings and plugins](https://www.onorca.dev/docs/settings), [iOS emulator skill](https://github.com/stablyai/orca/blob/main/skills/orca-emulator/SKILL.md), [Android emulator skill](https://github.com/stablyai/orca/blob/main/skills/orca-emulator-android/SKILL.md), [Client creation policy](https://github.com/stablyai/orca/blob/main/src/renderer/src/lib/client-creation-action-policy.ts)
- T3 Code: [README](https://github.com/pingdotgg/t3code), [Claude](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-claude.md), [Codex](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-codex.md), [Remote access](https://github.com/pingdotgg/t3code/blob/main/docs/user/remote-access.md), [Background service](https://github.com/pingdotgg/t3code/blob/main/docs/user/background-service.md), [Devices](https://github.com/pingdotgg/t3code/blob/main/docs/user/devices.md), [Outside agents](https://github.com/pingdotgg/t3code/blob/main/docs/user/outside-agents.md), [Usage and limits](https://github.com/pingdotgg/t3code/blob/main/docs/user/usage.md), [Composer](https://github.com/pingdotgg/t3code/blob/main/docs/user/composer.md), [Mobile notifications](https://github.com/pingdotgg/t3code/blob/main/docs/user/mobile-notifications.md), [Scheduled tasks](https://github.com/pingdotgg/t3code/blob/main/docs/user/project-settings.md), [Guest links](https://github.com/pingdotgg/t3code/blob/main/docs/user/guest-links.md), [Thread sidebar](https://github.com/pingdotgg/t3code/blob/main/docs/user/thread-sidebar.md), [Thread search](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/orchestration-v2/ThreadSearch.ts), [File browser](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/files/FileBrowserPanel.tsx), [File saving](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/files/fileSaveCoordinator.ts), [Content search](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/search/ProjectContentSearchDialog.tsx)
- DeepSeek Harness: [README](https://github.com/deepseek-ai/deepseek-harness), [Providers](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/providers.md), [Claude Code subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-claude-code/README.md), [Codex subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-codex/README.md), [Sign-in flows](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/llm/llm-pi-ai/src/login.ts), [Public deployments](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/public-deployments.md), [SSH](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/ssh.md), [Persistence](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/persistence.md), [Tool catalog](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/tool-catalog.md), [Agent presets](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/preset/agent-preset/README.md), [Browser use](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/browser-use.md), [Computer use](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/computer-use/README.md), [Desktop app](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/desktop/README.md), [Schedule](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/schedule.md), [Files sidebar](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-sidebar-files/README.md), [Workspace](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-workspace/README.md), [Plugin indexes](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/plugin-market.md), [Account quota](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-settings-account/src/client/AccountQuotaNotice.tsx), [Voice input](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/voice-input-bundle/README.md), [Liveshare](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/liveshare-protocol/README.md), [Worktree](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/worktree/README.md)
- opencode: [Providers](https://opencode.ai/docs/providers/), [Provider plugins](https://github.com/anomalyco/opencode/tree/dev/packages/core/src/plugin/provider), [Web](https://opencode.ai/docs/web/), [Server](https://opencode.ai/docs/server/), [TUI](https://opencode.ai/docs/tui/), [Agents](https://opencode.ai/docs/agents/), [MCP servers](https://opencode.ai/docs/mcp-servers/), [Browser plugin](https://github.com/anomalyco/opencode/blob/dev/packages/plugin-browser/README.md), [Enterprise](https://opencode.ai/docs/enterprise/), [GitHub](https://opencode.ai/docs/github/), [Plugins](https://opencode.ai/docs/plugins/), [Files](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/workspaces/files/model.tsx), [Workspaces](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/workspaces/submenu.tsx), [Session search](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/home/sessions/search.ts), [Review comments](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/composer/comment-note.ts), [AI library](https://github.com/anomalyco/opencode/blob/dev/packages/ai/README.md)
- Pi: [README](https://github.com/earendil-works/pi), [Providers](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md), [OAuth sign-ins](https://github.com/earendil-works/pi/tree/main/packages/ai/src/auth/oauth), [Credential store](https://github.com/earendil-works/pi/blob/main/packages/ai/src/auth/credential-store.ts), [SSH extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/ssh.ts), [Subagent extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/subagent/README.md), [Containerization](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/containerization.md), [Termux](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/termux.md), [Slash commands](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/slash-commands.md), [Extensions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md), [Packages](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/packages.md), [Codemode](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/codemode.md), [Sessions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/sessions.md)
- herdr: [README](https://github.com/herdrdev/herdr), [Agents](https://herdr.dev/docs/agents/), [Connecting machines](https://herdr.dev/docs/connecting-machines/), [Persistence and remote](https://herdr.dev/docs/persistence-remote/), [Work from your phone](https://herdr.dev/docs/how-to-work/#work-from-your-phone), [Agent skill](https://herdr.dev/docs/agent-skill/), [Configuration](https://herdr.dev/docs/configuration/), [Plugins](https://herdr.dev/docs/plugins/)
- Codex: [Image generation skill](https://github.com/openai/codex/blob/main/codex-rs/skills/src/assets/samples/imagegen/SKILL.md)

</details>

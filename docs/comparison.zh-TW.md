# Demi 比較

[English](comparison.md) · [简体中文](comparison.zh-CN.md) · **繁體中文** · [日本語](comparison.ja.md) · [한국어](comparison.ko.md)

Demi 與同類智慧體應用程式的差異，截至 2026 年 10 月，依據各產品的文件和原始碼整理。只列出各產品之間有差異的項目。

✓ 支援 · ◐ 部分支援 · ✗ 不支援 · **Web**：在瀏覽器裡使用，沒有獨立的應用程式 · **PWA**：從瀏覽器安裝成應用程式

| | Demi | [Pi](https://github.com/earendil-works/pi) | [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) | [OpenHands](https://github.com/OpenHands/OpenHands) | [opencode](https://github.com/anomalyco/opencode) | [Orca](https://github.com/stablyai/orca) | [herdr](https://github.com/herdrdev/herdr) | [T3 Code](https://github.com/pingdotgg/t3code) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **模型與訂閱** | | | | | | | | |
| Claude 訂閱 | ✓ | ✓ | ◐¹ | ✓ | ✗ | ✓ | ✓ | ✓ |
| ChatGPT、Grok 等訂閱 | ✓ | ✓ | ◐¹ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 多帳號 | ✓ | ✗ | ✗ | ✗ | ✓ | ✓ | ✗ | ✓ |
| 帳號輪替 | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| 任務中切換模型 | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ✗ | ✓ |
| **介面** | | | | | | | | |
| 網頁版 | ✓ | ✗ | ✓ | ✓ | ✓ | ✓ | ✗ | ✓ |
| 手機 | PWA | ✗ | Web | Web | Web | ✓ | ✗ | ✓ |
| 桌面應用程式 | PWA | ✗ | ✓ | ✓ | ✓ | ✓ | ✗ | ✓ |
| 終端機介面 | ✗ | ✓ | ✗ | ✓ | ✓ | ✗ | ✓ | ✗ |
| **執行** | | | | | | | | |
| 中央伺服器 | ✓ | ✗ | ◐ | ◐ | ◐ | ◐ | ✗ | ✗ |
| 裝置全關也能執行 | ✓ | ✗ | ◐ | ✓² | ◐ | ◐ | ◐ | ◐ |
| 對話同步 | ✓ | ✗ | ✗ | ◐ | ◐ | ◐ | ✗ | ◐ |
| 雲端沙箱 | ✓ | ✗ | ✗ | ✓ | ✗ | ◐ | ✗ | ✗ |
| 遠端機器 | ✓ | ◐ | ◐ | ◐ | ◐ | ✓ | ✓ | ✓ |
| 一個任務用多台機器 | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| 中途換機器 | ✓ | ◐ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| **智慧體** | | | | | | | | |
| 自行開發的智慧體 | ✓ | ✓ | ✓ | ✓ | ✓ | ✗ | ✗ | ✗ |
| 只用 Shell 一個工具 | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| 多智慧體 | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 智慧體設定檔 | ✓ | ◐ | ✓ | ✓ | ✓ | ✗ | ✗ | ✗ |
| Skills | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ✓ |
| MCP | ✗ | ✓ | ✓ | ✓ | ✓ | ◐ | ✗ | ✓ |
| 瀏覽器操作 | ✓ | ✗ | ◐ | ✓ | ✓ | ✓ | ✗ | ✓ |
| 電腦操作 | ✗ | ✗ | ◐³ | ✗ | ✗ | ✓ | ✗ | ◐⁴ |
| **團隊** | | | | | | | | |
| 多使用者 | ✓ | ✗ | ✗ | ◐² | ✗ | ✗ | ✗ | ✗ |
| 管理主控台 | ✓ | ✗ | ✗ | ◐² | ◐² | ✗ | ✗ | ✗ |
| 使用者隔離 | ✓ | ✗ | ✗ | ◐² | ✗ | ✗ | ✗ | ✗ |

1. 只能透過它啟動的 Claude Code、Codex 子智慧體使用。它自己的訂閱登入已寫在程式碼裡，但應用程式裡沒有入口。
2. 僅限付費的雲端版或企業版。
3. 實驗功能，需要手動開啟。
4. 只能操作 iOS 和 Android 模擬器，不能操作電腦桌面。

## 各項的意思

- **訂閱**：用訂閱帳號登入，而不是 API 金鑰。透過產品執行的 Claude Code、Codex 等智慧體用上的訂閱，也算支援。
- **多帳號**、**帳號輪替**：要產品自己做到，不算它執行的智慧體的能力。輪替是指一個帳號用到額度上限時，自動換另一個帳號繼續。
- **中央伺服器**：一台伺服器保存智慧體和所有對話，你的各台電腦只負責執行指令。◐：有伺服器，但只能在它自己那台機器上執行。✗：每台機器各跑一個智慧體，或者沒有伺服器。
- **裝置全關也能執行**：你的電腦和手機都關了，任務照樣繼續。◐：要你自己準備一台一直開著的機器，把它裝上去。
- **對話同步**：不管對話在哪台機器上執行，所有對話都在同一個列表裡，在任何裝置上看到的都一樣。◐：只有連同一台伺服器的裝置之間同步。
- **一個任務用多台機器**：一個對話同時在幾台機器上工作。**中途換機器**：對話換到另一台機器上接著做。
- **自行開發的智慧體**：由產品自己的智慧體驅動模型，而不是執行 Claude Code、Codex 等現成的智慧體。
- **只用 Shell 一個工具**：模型只拿到一個工具，就是 Shell，所有能力都是 Shell 裡的指令。
- **電腦操作**：智慧體透過截圖、滑鼠和鍵盤操作電腦桌面，不限於瀏覽器。

<details>
<summary>出處</summary>

- Pi: [README](https://github.com/earendil-works/pi), [Providers](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md), [OAuth sign-ins](https://github.com/earendil-works/pi/tree/main/packages/ai/src/auth/oauth), [Credential store](https://github.com/earendil-works/pi/blob/main/packages/ai/src/auth/credential-store.ts), [SSH extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/ssh.ts), [Subagent extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/subagent/README.md), [Containerization](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/containerization.md), [Termux](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/termux.md), [Slash commands](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/slash-`crates/command-declarations/src/lib.rs`)
- DeepSeek Harness: [README](https://github.com/deepseek-ai/deepseek-harness), [Providers](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/providers.md), [Claude Code subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-claude-code/README.md), [Codex subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-codex/README.md), [Sign-in flows](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/llm/llm-pi-ai/src/login.ts), [Public deployments](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/public-deployments.md), [SSH](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/ssh.md), [Persistence](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/persistence.md), [Tool catalog](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/tool-catalog.md), [Agent presets](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/preset/agent-preset/README.md), [Browser use](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/browser-use.md), [Computer use](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/computer-use/README.md), [Desktop app](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/desktop/README.md)
- OpenHands: [README](https://github.com/OpenHands/OpenHands), [ACP agents](https://docs.openhands.dev/openhands/usage/agent-canvas/acp-agents), [Subscription sign-in](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/openai.py), [Credentials](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/credentials.py), [Backends](https://docs.openhands.dev/openhands/usage/agent-canvas/backends), [Remote backend](https://docs.openhands.dev/openhands/usage/agent-canvas/backend-setup/remote), [Phone and tablet access](https://docs.openhands.dev/openhands/usage/agent-canvas/mobile-access), [Cloud UI](https://docs.openhands.dev/openhands/usage/cloud/cloud-ui), [Enterprise vs. open source](https://docs.openhands.dev/enterprise/enterprise-vs-oss), [Tools](https://github.com/OpenHands/software-agent-sdk/tree/main/openhands-tools/openhands/tools)
- opencode: [Providers](https://opencode.ai/docs/providers/), [Provider plugins](https://github.com/anomalyco/opencode/tree/dev/packages/core/src/plugin/provider), [Web](https://opencode.ai/docs/web/), [Server](https://opencode.ai/docs/server/), [TUI](https://opencode.ai/docs/tui/), [Agents](https://opencode.ai/docs/agents/), [MCP servers](https://opencode.ai/docs/mcp-servers/), [Browser plugin](https://github.com/anomalyco/opencode/blob/dev/packages/plugin-browser/README.md), [Enterprise](https://opencode.ai/docs/enterprise/)
- Orca: [README](https://github.com/stablyai/orca), [Supported agents](https://www.onorca.dev/docs/agents/supported), [Accounts](https://www.onorca.dev/docs/agents/codex-hot-swap), [Ways to run](https://www.onorca.dev/docs/ways-to-run), [Remote servers](https://www.onorca.dev/docs/remote-servers), [Mobile](https://www.onorca.dev/docs/mobile), [Computer use](https://www.onorca.dev/docs/cli/computer-use), [Browser](https://www.onorca.dev/docs/browser/overview)
- herdr: [README](https://github.com/herdrdev/herdr), [Agents](https://herdr.dev/docs/agents/), [Connecting machines](https://herdr.dev/docs/connecting-machines/), [Persistence and remote](https://herdr.dev/docs/persistence-remote/), [Work from your phone](https://herdr.dev/docs/how-to-work/#work-from-your-phone), [Agent skill](https://herdr.dev/docs/agent-skill/)
- T3 Code: [README](https://github.com/pingdotgg/t3code), [Claude](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-claude.md), [Codex](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-codex.md), [Remote access](https://github.com/pingdotgg/t3code/blob/main/docs/user/remote-access.md), [Background service](https://github.com/pingdotgg/t3code/blob/main/docs/user/background-service.md), [Devices](https://github.com/pingdotgg/t3code/blob/main/docs/user/devices.md), [Outside agents](https://github.com/pingdotgg/t3code/blob/main/docs/user/outside-agents.md)

</details>

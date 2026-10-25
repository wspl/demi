# Demi 比較

[English](comparison.md) · [简体中文](comparison.zh-CN.md) · **繁體中文** · [日本語](comparison.ja.md) · [한국어](comparison.ko.md)

Demi 與同類智慧體應用程式的差異，截至 2026 年 10 月，依據各產品的文件和原始碼整理。只列出各產品之間有差異的項目，以及一項目前都不支援的能力。各欄按與 Demi 的接近程度，從最接近排到最不接近。

表中使用以下符號和標籤：

- ✓ 支援
- ◐ 部分支援
- 空白：不支援
- **Web**：在瀏覽器裡使用，沒有獨立的應用程式
- **PWA**：從瀏覽器安裝成應用程式

下表逐項比較各產品：

| | Demi | [OpenHands][openhands] | [Orca][orca] | [T3 Code][t3-code] | [DeepSeek Harness][deepseek-harness] | [opencode][opencode] | [Pi][pi] | [herdr][herdr] |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **模型與訂閱** | | | | | | | | |
| Claude 訂閱 | ✓ | ✓ | ✓ | ✓ | ◐¹ | | ✓ | ✓ |
| ChatGPT、Grok 等訂閱 | ✓ | ✓ | ✓ | ✓ | ◐¹ | ✓ | ✓ | ✓ |
| 多帳號 | ✓ | | ✓ | ✓ | | ✓ | | |
| 帳號輪替 | ✓ | | | | | | | |
| 訂閱用量和重置時間 | ✓ | ◐² | ✓ | ✓ | ◐³ | | | |
| 任務中切換模型 | ✓ | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | |
| **介面** | | | | | | | | |
| 網頁版 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | | |
| 手機 | PWA | Web | ✓ | ✓ | Web | Web | | |
| 桌面應用程式 | PWA | ✓ | ✓ | ✓ | ✓ | ✓ | | |
| 終端機介面 | | ✓ | | | | ✓ | ✓ | ✓ |
| 語音輸入 | | ✓ | ✓ | ✓ | ◐⁴ | | | |
| 推播到手機 | ✓ | | ✓ | ✓⁵ | | | | ◐⁶ |
| 搜尋所有對話 | ✓ | | ◐⁷ | ✓ | ◐⁸ | ◐⁸ | ✓ | |
| **執行** | | | | | | | | |
| 中央伺服器 | ✓ | ◐ | ◐ | | ◐ | ◐ | | |
| 裝置全關也能執行 | ✓ | ✓⁹ | ◐ | ◐ | ◐ | ◐ | | ◐ |
| 對話同步 | ✓ | ◐ | ◐ | ◐ | | ◐ | | |
| 雲端沙箱 | ✓ | ✓ | ◐ | | | | | |
| 遠端機器 | ✓ | ◐ | ✓ | ✓ | ◐ | ◐ | ◐ | ✓ |
| 一個任務用多台機器 | ✓ | | | | | | | |
| 中途換機器 | ✓ | | | | | | ◐ | |
| 連接埠轉送 | ✓ | | ✓¹⁰ | | | | | |
| 排程任務 | ✓ | ✓ | ✓ | ✓ | ✓ | ◐¹¹ | | |
| **檔案** | | | | | | | | |
| 檔案瀏覽器 | ✓ | ✓ | ✓ | ✓ | ◐¹² | ✓ | | |
| 檔案管理 | ◐¹³ | ◐¹⁴ | ✓ | ◐¹⁵ | | | | |
| 搜尋檔案內容 | ✓ | | ✓ | ✓ | | | | |
| Git worktree | ◐¹⁶ | ✓ | ✓ | ✓ | ◐⁴ | ✓ | | ✓ |
| 在 diff 的行上留言 | | | ✓ | ◐¹⁷ | | ✓ | | |
| **智慧體** | | | | | | | | |
| 自行開發的智慧體 | ✓ | ✓ | | | ✓ | ✓ | ✓ | |
| 只用 Shell 一個工具 | ✓ | | | | | | | |
| 多智慧體 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ✓ |
| 智慧體設定檔 | ✓ | ✓ | | | ✓ | ✓ | ◐ | |
| Skills | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ |
| MCP | | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | |
| 第三方外掛 | | ✓ | ◐⁴ | | ✓ | ✓ | ✓ | ✓ |
| 給智慧體用的密鑰 | ✓ | ✓ | | | | | | |
| 瀏覽器操作 | ✓ | ✓ | ✓ | ✓ | ◐ | ◐¹⁸ | | |
| 即時查看和操作瀏覽器 | ✓ | ◐¹⁹ | ✓ | ✓ | | ◐¹⁸ | | |
| 電腦操作 | | | ◐¹⁸ | ◐²⁰ | ◐⁴ | | | |
| **手機裝置** | | | | | | | | |
| iOS 模擬器 | ✓ | | ◐¹⁸ | ✓ | | | | |
| iOS 實機 | | | | | | | | |
| Android 模擬器 | ✓ | | ◐¹⁸ | ✓ | | | | |
| Android 實機 | | | ◐¹⁸ | ✓ | | | | |
| **媒體生成** | | | | | | | | |
| 圖片生成與編輯 | ✓ | ◐²¹ | ◐²¹ | ◐²¹ | ◐²¹ | | ◐²² | ◐²¹ |
| 影片生成 | ✓ | | | | | | | |
| 語音、音樂和音效 | ✓ | | | | | | | |
| **團隊** | | | | | | | | |
| 多使用者 | ✓ | ◐⁹ | | | | | | |
| 管理主控台 | ✓ | ◐⁹ | | | | ◐⁹ | | |
| 使用者隔離 | ✓ | ◐⁹ | | | | | | |
| 分享對話連結 | ✓ | ◐²³ | | ◐²⁴ | ◐⁴ | | ✓ | |

表中的編號註解如下：

1. 只能透過它啟動的 Claude Code、Codex 子智慧體使用。它自己的訂閱登入已寫在程式碼裡，但應用程式裡沒有入口。
2. 按 API 價格估算的方案用量。
3. 只顯示 DeepSeek 自家帳號的額度。
4. 實驗功能，需要手動開啟。
5. 只能透過 T3 Connect。
6. 只有一個推播到 Telegram 的範例外掛。
7. 按標題、目錄、分支、模型和預覽搜尋，不搜尋全文。
8. 只按標題搜尋。
9. 僅限付費的雲端版或企業版。
10. 只能從 SSH 主機轉送到這台電腦，會自動找出主機上正在監聽的連接埠。
11. 只能借助 GitHub Actions 的 cron 排程執行。
12. 只能看它所在那台機器的檔案，網頁介面看不到 SSH 主機上的檔案。
13. 能上傳、下載、新增資料夾，不能重新命名、刪除、移動和編輯。
14. 只能下載整個工作區。
15. 只能編輯檔案內容。
16. 只把已有的 worktree 顯示在所屬專案下，不能新增。
17. 可以引用任意一段文字並附上留言，不是針對 diff 的某一行。
18. 只能在執行它桌面應用程式的那台電腦上用，網頁用戶端和遠端機器上都不行。
19. 只顯示智慧體最近一張截圖，不能操作。
20. 只能操作 iOS 和 Android 模擬器，不能操作電腦桌面。
21. 靠它執行的 Codex 內建的生圖工具。
22. 只能在 codemode 腳本裡呼叫，codemode 要手動開啟。
23. 只有用 OpenHands Cloud 後端時才行。
24. Guest links，測試版，需要 T3 Connect。

[openhands]: https://github.com/OpenHands/OpenHands
[orca]: https://github.com/stablyai/orca
[t3-code]: https://github.com/pingdotgg/t3code
[deepseek-harness]: https://github.com/deepseek-ai/deepseek-harness
[opencode]: https://github.com/anomalyco/opencode
[pi]: https://github.com/earendil-works/pi
[herdr]: https://github.com/herdrdev/herdr

## 各項的意思

各行的意思如下：

- **跟機器有關的行**（檔案、worktree、diff、瀏覽器、電腦操作、手機裝置）：智慧體在遠端機器上工作時也能用，才算 ✓，例如 SSH 主機、另一台電腦或雲端機器；只能在執行產品的那台電腦上用的，算 ◐。
- **訂閱**：用訂閱帳號登入，而不是 API 金鑰。透過產品執行的 Claude Code、Codex 等智慧體用上的訂閱，也算支援。
- **多帳號**、**帳號輪替**：要產品自己做到，不算它執行的智慧體的能力。輪替是指一個帳號用到額度上限時，自動換另一個帳號繼續。
- **訂閱用量和重置時間**：每個訂閱的額度用了多少，什麼時候重置。
- **推播到手機**：應用程式關著時，通知也能送到手機上。
- **搜尋所有對話**：在所有對話裡搜尋任意一則訊息裡的文字。
- **中央伺服器**：一台伺服器保存智慧體和所有對話，你的各台電腦只負責執行指令。
  - ◐：有伺服器，但只能在它自己那台機器上執行。
  - 空白：每台機器各跑一個智慧體，或者沒有伺服器。
- **裝置全關也能執行**：你的電腦和手機都關了，任務照樣繼續。
  - ◐：要你自己準備一台一直開著的機器，把它裝上去。
- **對話同步**：不管對話在哪台機器上執行，所有對話都在同一個列表裡，在任何裝置上看到的都一樣。
  - ◐：只有連同一台伺服器的裝置之間同步。
- **一個任務用多台機器**：一個對話同時在幾台機器上工作。
- **中途換機器**：對話換到另一台機器上接著做。
- **連接埠轉送**：一台機器上的程式能存取另一台機器的連接埠，就像 `ssh -L` 那樣。
- **排程任務**：任務在設定的時間或間隔自己開始。
- **檔案管理**：在應用程式裡上傳、下載、新增、重新命名、刪除、移動和編輯檔案。
- **在 diff 的行上留言**：對變更的某一行寫留言，留言會交給智慧體。
- **自行開發的智慧體**：由產品自己的智慧體驅動模型，而不是執行 Claude Code、Codex 等現成的智慧體。
- **只用 Shell 一個工具**：模型只拿到一個工具，就是 Shell，所有能力都是 Shell 裡的指令。
- **第三方外掛**：使用者能安裝別人寫的程式碼，替產品加上工具或介面，不算 Skills 和 MCP。Demi 自己的功能都是外掛，但不載入別人的外掛。
- **給智慧體用的密鑰**：使用者按名稱儲存的密鑰，智慧體在指令裡使用。
- **即時查看和操作瀏覽器**：使用者能即時看到智慧體在用的瀏覽器，也能自己動手操作。
- **電腦操作**：智慧體透過截圖、滑鼠和鍵盤操作電腦桌面，不限於瀏覽器。
- **iOS 實機**、**Android 實機**：連在電腦上的真手機，不是模擬器。
- **分享對話連結**：用一個連結把對話給別人看。

<details>
<summary>出處</summary>

- OpenHands: [README](https://github.com/OpenHands/OpenHands), [ACP agents](https://docs.openhands.dev/openhands/usage/agent-canvas/acp-agents), [Subscription sign-in](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/openai.py), [Credentials](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/credentials.py), [Backends](https://docs.openhands.dev/openhands/usage/agent-canvas/backends), [Remote backend](https://docs.openhands.dev/openhands/usage/agent-canvas/backend-setup/remote), [Phone and tablet access](https://docs.openhands.dev/openhands/usage/agent-canvas/mobile-access), [Cloud UI](https://docs.openhands.dev/openhands/usage/cloud/cloud-ui), [Enterprise vs. open source](https://docs.openhands.dev/enterprise/enterprise-vs-oss), [Tools](https://github.com/OpenHands/software-agent-sdk/tree/main/openhands-tools/openhands/tools), [Automations](https://docs.openhands.dev/openhands/usage/agent-canvas/prebuilt-automations), [Secrets](https://github.com/OpenHands/OpenHands/blob/main/src/routes/secrets-settings.tsx), [Files](https://github.com/OpenHands/OpenHands/blob/main/src/routes/files-tab.tsx), [Workspace download](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/conversation-panel/conversation-card/conversation-card-context-menu.tsx), [Browser panel](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/browser/browser.tsx), [Public share](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/conversation/conversation-name-context-menu.tsx), [Dictation](https://github.com/OpenHands/OpenHands/blob/main/src/hooks/chat/use-dictation.ts), [Worktrees](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/chat/workspace-mode-selector.tsx), [Usage](https://github.com/OpenHands/OpenHands/blob/main/src/routes/usage-tab.tsx), [Plugins](https://github.com/OpenHands/OpenHands/blob/main/src/routes/skills-plugins.tsx)
- Orca: [README](https://github.com/stablyai/orca), [Supported agents](https://www.onorca.dev/docs/agents/supported), [Accounts](https://www.onorca.dev/docs/agents/codex-hot-swap), [Ways to run](https://www.onorca.dev/docs/ways-to-run), [Remote servers](https://www.onorca.dev/docs/remote-servers), [Mobile](https://www.onorca.dev/docs/mobile), [Computer use](https://www.onorca.dev/docs/cli/computer-use), [Browser](https://www.onorca.dev/docs/browser/overview), [File explorer](https://www.onorca.dev/docs/editing/file-explorer), [Editor](https://www.onorca.dev/docs/editing/monaco), [Session history](https://www.onorca.dev/docs/agents/session-history), [Worktrees](https://www.onorca.dev/docs/model/worktrees), [Annotate AI diffs](https://www.onorca.dev/docs/review/annotate-ai-diff), [SSH and port forwarding](https://www.onorca.dev/docs/ssh), [Automations](https://www.onorca.dev/docs/cli/automations), [Usage tracking](https://www.onorca.dev/docs/agents/usage-tracking), [Settings and plugins](https://www.onorca.dev/docs/settings), [iOS emulator skill](https://github.com/stablyai/orca/blob/main/skills/orca-emulator/SKILL.md), [Android emulator skill](https://github.com/stablyai/orca/blob/main/skills/orca-emulator-android/SKILL.md), [Client creation policy](https://github.com/stablyai/orca/blob/main/src/renderer/src/lib/client-creation-action-policy.ts)
- T3 Code: [README](https://github.com/pingdotgg/t3code), [Claude](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-claude.md), [Codex](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-codex.md), [Remote access](https://github.com/pingdotgg/t3code/blob/main/docs/user/remote-access.md), [Background service](https://github.com/pingdotgg/t3code/blob/main/docs/user/background-service.md), [Devices](https://github.com/pingdotgg/t3code/blob/main/docs/user/devices.md), [Outside agents](https://github.com/pingdotgg/t3code/blob/main/docs/user/outside-agents.md), [Usage and limits](https://github.com/pingdotgg/t3code/blob/main/docs/user/usage.md), [Composer](https://github.com/pingdotgg/t3code/blob/main/docs/user/composer.md), [Mobile notifications](https://github.com/pingdotgg/t3code/blob/main/docs/user/mobile-notifications.md), [Scheduled tasks](https://github.com/pingdotgg/t3code/blob/main/docs/user/project-settings.md), [Guest links](https://github.com/pingdotgg/t3code/blob/main/docs/user/guest-links.md), [Thread sidebar](https://github.com/pingdotgg/t3code/blob/main/docs/user/thread-sidebar.md), [Thread search](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/orchestration-v2/ThreadSearch.ts), [File browser](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/files/FileBrowserPanel.tsx), [File saving](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/files/fileSaveCoordinator.ts), [Content search](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/search/ProjectContentSearchDialog.tsx)
- DeepSeek Harness: [README](https://github.com/deepseek-ai/deepseek-harness), [Providers](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/providers.md), [Claude Code subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-claude-code/README.md), [Codex subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-codex/README.md), [Sign-in flows](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/llm/llm-pi-ai/src/login.ts), [Public deployments](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/public-deployments.md), [SSH](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/ssh.md), [Persistence](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/persistence.md), [Tool catalog](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/tool-catalog.md), [Agent presets](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/preset/agent-preset/README.md), [Browser use](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/browser-use.md), [Computer use](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/computer-use/README.md), [Desktop app](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/desktop/README.md), [Schedule](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/schedule.md), [Files sidebar](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-sidebar-files/README.md), [Workspace](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-workspace/README.md), [Plugin indexes](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/plugin-market.md), [Account quota](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-settings-account/src/client/AccountQuotaNotice.tsx), [Voice input](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/voice-input-bundle/README.md), [Liveshare](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/liveshare-protocol/README.md), [Worktree](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/worktree/README.md)
- opencode: [Providers](https://opencode.ai/docs/providers/), [Provider plugins](https://github.com/anomalyco/opencode/tree/dev/packages/core/src/plugin/provider), [Web](https://opencode.ai/docs/web/), [Server](https://opencode.ai/docs/server/), [TUI](https://opencode.ai/docs/tui/), [Agents](https://opencode.ai/docs/agents/), [MCP servers](https://opencode.ai/docs/mcp-servers/), [Browser plugin](https://github.com/anomalyco/opencode/blob/dev/packages/plugin-browser/README.md), [Enterprise](https://opencode.ai/docs/enterprise/), [GitHub](https://opencode.ai/docs/github/), [Plugins](https://opencode.ai/docs/plugins/), [Files](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/workspaces/files/model.tsx), [Workspaces](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/workspaces/submenu.tsx), [Session search](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/home/sessions/search.ts), [Review comments](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/composer/comment-note.ts), [AI library](https://github.com/anomalyco/opencode/blob/dev/packages/ai/README.md)
- Pi: [README](https://github.com/earendil-works/pi), [Providers](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md), [OAuth sign-ins](https://github.com/earendil-works/pi/tree/main/packages/ai/src/auth/oauth), [Credential store](https://github.com/earendil-works/pi/blob/main/packages/ai/src/auth/credential-store.ts), [SSH extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/ssh.ts), [Subagent extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/subagent/README.md), [Containerization](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/containerization.md), [Termux](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/termux.md), [Slash commands](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/slash-commands.md), [Extensions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md), [Packages](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/packages.md), [Codemode](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/codemode.md), [Sessions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/sessions.md)
- herdr: [README](https://github.com/herdrdev/herdr), [Agents](https://herdr.dev/docs/agents/), [Connecting machines](https://herdr.dev/docs/connecting-machines/), [Persistence and remote](https://herdr.dev/docs/persistence-remote/), [Work from your phone](https://herdr.dev/docs/how-to-work/#work-from-your-phone), [Agent skill](https://herdr.dev/docs/agent-skill/), [Configuration](https://herdr.dev/docs/configuration/), [Plugins](https://herdr.dev/docs/plugins/)
- Codex: [Image generation skill](https://github.com/openai/codex/blob/main/codex-rs/skills/src/assets/samples/imagegen/SKILL.md)

</details>

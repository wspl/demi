# Demi の比較

[English](comparison.md) · [简体中文](comparison.zh-CN.md) · [繁體中文](comparison.zh-TW.md) · **日本語** · [한국어](comparison.ko.md)

同種のエージェントアプリと Demi の違いを、2026 年 10 月時点で各製品のドキュメントとソースコードをもとにまとめました。製品間で違いがある項目と、まだどの製品も対応していない 1 つの機能を載せています。列は Demi に近い製品から遠い製品の順に並べています。

表では次の記号とラベルを使います:

- ✓ 対応
- ◐ 一部対応
- 空欄：非対応
- **Web**：ブラウザで使い、専用アプリはない
- **PWA**：ブラウザからアプリとしてインストールできる

次の表で、製品を項目ごとに比較します:

| | Demi | [OpenHands][openhands] | [Orca][orca] | [T3 Code][t3-code] | [DeepSeek Harness][deepseek-harness] | [opencode][opencode] | [Pi][pi] | [herdr][herdr] |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **モデルとサブスクリプション** | | | | | | | | |
| Claude サブスクリプション | ✓ | ✓ | ✓ | ✓ | ◐¹ | | ✓ | ✓ |
| ChatGPT、Grok など | ✓ | ✓ | ✓ | ✓ | ◐¹ | ✓ | ✓ | ✓ |
| 複数アカウント | ✓ | | ✓ | ✓ | | ✓ | | |
| アカウントのローテーション | ✓ | | | | | | | |
| サブスクリプションの使用量とリセット時刻 | ✓ | ◐² | ✓ | ✓ | ◐³ | | | |
| タスク途中のモデル切り替え | ✓ | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | |
| **インターフェース** | | | | | | | | |
| Web アプリ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | | |
| モバイル | PWA | Web | ✓ | ✓ | Web | Web | | |
| デスクトップアプリ | PWA | ✓ | ✓ | ✓ | ✓ | ✓ | | |
| ターミナル UI | | ✓ | | | | ✓ | ✓ | ✓ |
| 音声入力 | | ✓ | ✓ | ✓ | ◐⁴ | | | |
| スマートフォンへの通知 | ✓ | | ✓ | ✓⁵ | | | | ◐⁶ |
| 会話の検索 | ✓ | | ◐⁷ | ✓ | ◐⁸ | ◐⁸ | ✓ | |
| **実行** | | | | | | | | |
| 中央サーバー | ✓ | ◐ | ◐ | | ◐ | ◐ | | |
| 全デバイスがオフでも実行 | ✓ | ✓⁹ | ◐ | ◐ | ◐ | ◐ | | ◐ |
| 会話の同期 | ✓ | ◐ | ◐ | ◐ | | ◐ | | |
| クラウドサンドボックス | ✓ | ✓ | ◐ | | | | | |
| リモートマシン | ✓ | ◐ | ✓ | ✓ | ◐ | ◐ | ◐ | ✓ |
| 複数マシンでのタスク | ✓ | | | | | | | |
| マシンの切り替え | ✓ | | | | | | ◐ | |
| ポートフォワーディング | ✓ | | ✓¹⁰ | | | | | |
| スケジュール実行 | ✓ | ✓ | ✓ | ✓ | ✓ | ◐¹¹ | | |
| **ファイル** | | | | | | | | |
| ファイルブラウザ | ✓ | ✓ | ✓ | ✓ | ◐¹² | ✓ | | |
| ファイル管理 | ◐¹³ | ◐¹⁴ | ✓ | ◐¹⁵ | | | | |
| ファイル内容の検索 | ✓ | | ✓ | ✓ | | | | |
| Git worktree | ◐¹⁶ | ✓ | ✓ | ✓ | ◐⁴ | ✓ | | ✓ |
| 差分の行へのコメント | | | ✓ | ◐¹⁷ | | ✓ | | |
| **エージェント** | | | | | | | | |
| 独自のエージェント | ✓ | ✓ | | | ✓ | ✓ | ✓ | |
| ツールはシェルのみ | ✓ | | | | | | | |
| マルチエージェント | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ✓ |
| エージェントプロファイル | ✓ | ✓ | | | ✓ | ✓ | ◐ | |
| Skills | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ |
| MCP | | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | |
| サードパーティのプラグイン | | ✓ | ◐⁴ | | ✓ | ✓ | ✓ | ✓ |
| エージェント用のシークレット | ✓ | ✓ | | | | | | |
| ブラウザ操作 | ✓ | ✓ | ✓ | ✓ | ◐ | ◐¹⁸ | | |
| ブラウザのライブ表示と操作 | ✓ | ◐¹⁹ | ✓ | ✓ | | ◐¹⁸ | | |
| コンピューター操作 | | | ◐¹⁸ | ◐²⁰ | ◐⁴ | | | |
| **モバイルデバイス** | | | | | | | | |
| iOS シミュレーター | ✓ | | ◐¹⁸ | ✓ | | | | |
| iOS 実機 | | | | | | | | |
| Android エミュレーター | ✓ | | ◐¹⁸ | ✓ | | | | |
| Android 実機 | | | ◐¹⁸ | ✓ | | | | |
| **メディア生成** | | | | | | | | |
| 画像の生成と編集 | ✓ | ◐²¹ | ◐²¹ | ◐²¹ | ◐²¹ | | ◐²² | ◐²¹ |
| 動画の生成 | ✓ | | | | | | | |
| 音声、音楽、効果音 | ✓ | | | | | | | |
| **チーム** | | | | | | | | |
| マルチユーザー | ✓ | ◐⁹ | | | | | | |
| 管理コンソール | ✓ | ◐⁹ | | | | ◐⁹ | | |
| ユーザーごとの分離 | ✓ | ◐⁹ | | | | | | |
| 会話の共有リンク | ✓ | ◐²³ | | ◐²⁴ | ◐⁴ | | ✓ | |

表中の番号の注記は次のとおりです:

1. 起動する Claude Code と Codex のサブエージェント経由のみ。製品自身のサブスクリプションのサインインはコードにあるが、アプリに入口がない。
2. API 料金で見積もったプランの使用量。
3. DeepSeek 自身のアカウントの枠のみ。
4. 試験的な機能で、手動で有効にする。
5. T3 Connect 経由のみ。
6. Telegram に送るサンプルプラグインのみ。
7. タイトル、フォルダ、ブランチ、モデル、プレビューで検索し、全文は検索しない。
8. タイトルのみで検索。
9. 有料のクラウド版またはエンタープライズ版のみ。
10. SSH ホストからこのコンピューターへの転送のみ。ホストで待ち受けているポートは自動で見つける。
11. GitHub Actions の cron ジョブとしてのみ。
12. 自身が動くマシンのファイルのみで、Web 画面からは SSH ホストのファイルを見られない。
13. アップロード、ダウンロード、新規フォルダのみで、名前の変更、削除、移動、編集はできない。
14. ワークスペース全体のダウンロードのみ。
15. ファイルの内容の編集のみ。
16. 既存の worktree をプロジェクトの下に表示するだけで、作成はできない。
17. 任意のテキストを引用してコメントを付ける機能で、差分の行が対象ではない。
18. デスクトップアプリを動かしているコンピューターでのみ使え、Web クライアントやリモートマシンでは使えない。
19. エージェントの直近のスクリーンショットを表示するだけで、操作はできない。
20. iOS と Android のシミュレーターのみで、デスクトップは操作できない。
21. 動かしている Codex の画像ツール経由。
22. codemode のスクリプトからのみ。codemode は手動で有効にする。
23. OpenHands Cloud のバックエンドを使う場合のみ。
24. ゲストリンク。ベータ版で T3 Connect が必要。

[openhands]: https://github.com/OpenHands/OpenHands
[orca]: https://github.com/stablyai/orca
[t3-code]: https://github.com/pingdotgg/t3code
[deepseek-harness]: https://github.com/deepseek-ai/deepseek-harness
[opencode]: https://github.com/anomalyco/opencode
[pi]: https://github.com/earendil-works/pi
[herdr]: https://github.com/herdrdev/herdr

## 各項目の意味

各項目の意味は次のとおりです:

- **マシンに関わる項目**（ファイル、worktree、差分、ブラウザ、コンピューター操作、モバイルデバイス）：エージェントが SSH ホスト、別のコンピューター、クラウドマシンなどのリモートマシンで作業するときにも使える場合だけ ✓ とします。製品を動かしているコンピューターでしか使えないものは ◐ です。
- **サブスクリプション**：API キーではなくサブスクリプションでサインインすること。製品が動かす Claude Code や Codex などのエージェント経由で使えるものも対応とします。
- **複数アカウント**、**アカウントのローテーション**：製品自身が行うものに限り、製品が動かすエージェントの機能は含めません。ローテーションは、あるアカウントが上限に達したとき別のアカウントで作業を続けることです。
- **サブスクリプションの使用量とリセット時刻**：各サブスクリプションの上限をどれだけ使ったか、いつリセットされるかがわかります。
- **スマートフォンへの通知**：アプリを閉じていても通知がスマートフォンに届きます。
- **会話の検索**：すべての会話から、どのメッセージの言葉でも見つけます。
- **中央サーバー**：1 台のサーバーがエージェントとすべての会話を持ち、手元のコンピューターはそのコマンドを実行するだけです。
  - ◐：サーバーはあるが、そのサーバー自身のマシンでしか実行しない。
  - 空欄：マシンごとにエージェントが動く、またはサーバーがない。
- **全デバイスがオフでも実行**：コンピューターもスマートフォンもオフのまま、タスクが続きます。
  - ◐：常時稼働のマシンを自分で用意してインストールした場合のみ。
- **会話の同期**：どのマシンで実行した会話も 1 つの一覧にまとまり、どのデバイスでも同じ内容が見えます。
  - ◐：同じサーバーにつないだデバイスの間でのみ同期。
- **複数マシンでのタスク**：1 つの会話が複数のマシンで作業します。
- **マシンの切り替え**：会話を別のマシンに移して続けます。
- **ポートフォワーディング**：あるマシンのプログラムが別のマシンのポートにつながります。`ssh -L` と同じです。
- **スケジュール実行**：決めた時刻や間隔でタスクが自動で始まります。
- **ファイル管理**：アプリ内でファイルをアップロード、ダウンロード、作成、名前変更、削除、移動、編集します。
- **差分の行へのコメント**：変更の 1 行に付けたコメントがエージェントに渡ります。
- **独自のエージェント**：Claude Code や Codex などのエージェントを動かすのではなく、製品自身のエージェントがモデルを動かします。
- **ツールはシェルのみ**：モデルに渡すツールはシェルひとつで、すべての機能はその中のコマンドです。
- **サードパーティのプラグイン**：Skills や MCP とは別に、他の人が書いたコードを入れて製品にツールや画面を足せます。Demi の機能自体はプラグインでできていますが、他の人のプラグインは読み込みません。
- **エージェント用のシークレット**：ユーザーが名前を付けて保存したシークレットを、エージェントがコマンドで使います。
- **ブラウザのライブ表示と操作**：エージェントが使っているブラウザを作業中に見られ、自分でも操作できます。
- **コンピューター操作**：エージェントがスクリーンショット、マウス、キーボードでデスクトップを操作します。ブラウザに限りません。
- **iOS 実機**、**Android 実機**：シミュレーターではなく、コンピューターにつないだ実際のスマートフォンです。
- **会話の共有リンク**：リンクで会話を他の人に見せます。

<details>
<summary>出典</summary>

- OpenHands: [README](https://github.com/OpenHands/OpenHands), [ACP agents](https://docs.openhands.dev/openhands/usage/agent-canvas/acp-agents), [Subscription sign-in](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/openai.py), [Credentials](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/credentials.py), [Backends](https://docs.openhands.dev/openhands/usage/agent-canvas/backends), [Remote backend](https://docs.openhands.dev/openhands/usage/agent-canvas/backend-setup/remote), [Phone and tablet access](https://docs.openhands.dev/openhands/usage/agent-canvas/mobile-access), [Cloud UI](https://docs.openhands.dev/openhands/usage/cloud/cloud-ui), [Enterprise vs. open source](https://docs.openhands.dev/enterprise/enterprise-vs-oss), [Tools](https://github.com/OpenHands/software-agent-sdk/tree/main/openhands-tools/openhands/tools), [Automations](https://docs.openhands.dev/openhands/usage/agent-canvas/prebuilt-automations), [Secrets](https://github.com/OpenHands/OpenHands/blob/main/src/routes/secrets-settings.tsx), [Files](https://github.com/OpenHands/OpenHands/blob/main/src/routes/files-tab.tsx), [Workspace download](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/conversation-panel/conversation-card/conversation-card-context-menu.tsx), [Browser panel](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/browser/browser.tsx), [Public share](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/conversation/conversation-name-context-menu.tsx), [Dictation](https://github.com/OpenHands/OpenHands/blob/main/src/hooks/chat/use-dictation.ts), [Worktrees](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/chat/workspace-mode-selector.tsx), [Usage](https://github.com/OpenHands/OpenHands/blob/main/src/routes/usage-tab.tsx), [Plugins](https://github.com/OpenHands/OpenHands/blob/main/src/routes/skills-plugins.tsx)
- Orca: [README](https://github.com/stablyai/orca), [Supported agents](https://www.onorca.dev/docs/agents/supported), [Accounts](https://www.onorca.dev/docs/agents/codex-hot-swap), [Ways to run](https://www.onorca.dev/docs/ways-to-run), [Remote servers](https://www.onorca.dev/docs/remote-servers), [Mobile](https://www.onorca.dev/docs/mobile), [Computer use](https://www.onorca.dev/docs/cli/computer-use), [Browser](https://www.onorca.dev/docs/browser/overview), [File explorer](https://www.onorca.dev/docs/editing/file-explorer), [Editor](https://www.onorca.dev/docs/editing/monaco), [Session history](https://www.onorca.dev/docs/agents/session-history), [Worktrees](https://www.onorca.dev/docs/model/worktrees), [Annotate AI diffs](https://www.onorca.dev/docs/review/annotate-ai-diff), [SSH and port forwarding](https://www.onorca.dev/docs/ssh), [Automations](https://www.onorca.dev/docs/cli/automations), [Usage tracking](https://www.onorca.dev/docs/agents/usage-tracking), [Settings and plugins](https://www.onorca.dev/docs/settings), [iOS emulator skill](https://github.com/stablyai/orca/blob/main/skills/orca-emulator/SKILL.md), [Android emulator skill](https://github.com/stablyai/orca/blob/main/skills/orca-emulator-android/SKILL.md), [Client creation policy](https://github.com/stablyai/orca/blob/main/src/renderer/src/lib/client-creation-action-policy.ts)
- T3 Code: [README](https://github.com/pingdotgg/t3code), [Claude](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-claude.md), [Codex](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-codex.md), [Remote access](https://github.com/pingdotgg/t3code/blob/main/docs/user/remote-access.md), [Background service](https://github.com/pingdotgg/t3code/blob/main/docs/user/background-service.md), [Devices](https://github.com/pingdotgg/t3code/blob/main/docs/user/devices.md), [Outside agents](https://github.com/pingdotgg/t3code/blob/main/docs/user/outside-agents.md), [Usage and limits](https://github.com/pingdotgg/t3code/blob/main/docs/user/usage.md), [Composer](https://github.com/pingdotgg/t3code/blob/main/docs/user/composer.md), [Mobile notifications](https://github.com/pingdotgg/t3code/blob/main/docs/user/mobile-notifications.md), [Scheduled tasks](https://github.com/pingdotgg/t3code/blob/main/docs/user/project-settings.md), [Guest links](https://github.com/pingdotgg/t3code/blob/main/docs/user/guest-links.md), [Thread sidebar](https://github.com/pingdotgg/t3code/blob/main/docs/user/thread-sidebar.md), [Thread search](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/orchestration-v2/ThreadSearch.ts), [File browser](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/files/FileBrowserPanel.tsx), [File saving](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/files/fileSaveCoordinator.ts), [Content search](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/search/ProjectContentSearchDialog.tsx)
- DeepSeek Harness: [README](https://github.com/deepseek-ai/deepseek-harness), [Providers](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/providers.md), [Claude Code subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-claude-code/README.md), [Codex subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-codex/README.md), [Sign-in flows](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/llm/llm-pi-ai/src/login.ts), [Public deployments](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/public-deployments.md), [SSH](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/ssh.md), [Persistence](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/persistence.md), [Tool catalog](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/tool-catalog.md), [Agent presets](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/preset/agent-preset/README.md), [Browser use](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/browser-use.md), [Computer use](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/computer-use/README.md), [Desktop app](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/desktop/README.md), [Schedule](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/schedule.md), [Files sidebar](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-sidebar-files/README.md), [Workspace](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-workspace/README.md), [Plugin indexes](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/plugin-market.md), [Account quota](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-settings-account/src/client/AccountQuotaNotice.tsx), [Voice input](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/voice-input-bundle/README.md), [Liveshare](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/liveshare-protocol/README.md), [Worktree](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/worktree/README.md)
- opencode: [Providers](https://opencode.ai/docs/providers/), [Provider plugins](https://github.com/anomalyco/opencode/tree/dev/packages/core/src/plugin/provider), [Web](https://opencode.ai/docs/web/), [Server](https://opencode.ai/docs/server/), [TUI](https://opencode.ai/docs/tui/), [Agents](https://opencode.ai/docs/agents/), [MCP servers](https://opencode.ai/docs/mcp-servers/), [Browser plugin](https://github.com/anomalyco/opencode/blob/dev/packages/plugin-browser/README.md), [Enterprise](https://opencode.ai/docs/enterprise/), [GitHub](https://opencode.ai/docs/github/), [Plugins](https://opencode.ai/docs/plugins/), [Files](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/workspaces/files/model.tsx), [Workspaces](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/workspaces/submenu.tsx), [Session search](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/home/sessions/search.ts), [Review comments](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/composer/comment-note.ts), [AI library](https://github.com/anomalyco/opencode/blob/dev/packages/ai/README.md)
- Pi: [README](https://github.com/earendil-works/pi), [Providers](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md), [OAuth sign-ins](https://github.com/earendil-works/pi/tree/main/packages/ai/src/auth/oauth), [Credential store](https://github.com/earendil-works/pi/blob/main/packages/ai/src/auth/credential-store.ts), [SSH extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/ssh.ts), [Subagent extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/subagent/README.md), [Containerization](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/containerization.md), [Termux](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/termux.md), [Slash commands](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/slash-commands.md), [Extensions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md), [Packages](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/packages.md), [Codemode](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/codemode.md), [Sessions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/sessions.md)
- herdr: [README](https://github.com/herdrdev/herdr), [Agents](https://herdr.dev/docs/agents/), [Connecting machines](https://herdr.dev/docs/connecting-machines/), [Persistence and remote](https://herdr.dev/docs/persistence-remote/), [Work from your phone](https://herdr.dev/docs/how-to-work/#work-from-your-phone), [Agent skill](https://herdr.dev/docs/agent-skill/), [Configuration](https://herdr.dev/docs/configuration/), [Plugins](https://herdr.dev/docs/plugins/)
- Codex: [Image generation skill](https://github.com/openai/codex/blob/main/codex-rs/skills/src/assets/samples/imagegen/SKILL.md)

</details>

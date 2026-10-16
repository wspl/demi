# Demi の比較

[English](comparison.md) · [简体中文](comparison.zh-CN.md) · [繁體中文](comparison.zh-TW.md) · **日本語** · [한국어](comparison.ko.md)

同種のエージェントアプリと Demi の違いを、2026 年 10 月時点で各製品のドキュメントとソースコードをもとにまとめました。製品間で違いがある項目だけを載せています。

✓ 対応 · ◐ 一部対応 · ✗ 非対応 · **Web**：ブラウザで使い、専用アプリはない · **PWA**：ブラウザからアプリとしてインストールできる

| | Demi | [Pi](https://github.com/earendil-works/pi) | [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) | [OpenHands](https://github.com/OpenHands/OpenHands) | [opencode](https://github.com/anomalyco/opencode) | [Orca](https://github.com/stablyai/orca) | [herdr](https://github.com/herdrdev/herdr) | [T3 Code](https://github.com/pingdotgg/t3code) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **モデルとサブスクリプション** | | | | | | | | |
| Claude サブスクリプション | ✓ | ✓ | ◐¹ | ✓ | ✗ | ✓ | ✓ | ✓ |
| ChatGPT、Grok など | ✓ | ✓ | ◐¹ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 複数アカウント | ✓ | ✗ | ✗ | ✗ | ✓ | ✓ | ✗ | ✓ |
| アカウントのローテーション | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| タスク途中のモデル切り替え | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ✗ | ✓ |
| **インターフェース** | | | | | | | | |
| Web アプリ | ✓ | ✗ | ✓ | ✓ | ✓ | ✓ | ✗ | ✓ |
| モバイル | PWA | ✗ | Web | Web | Web | ✓ | ✗ | ✓ |
| デスクトップアプリ | PWA | ✗ | ✓ | ✓ | ✓ | ✓ | ✗ | ✓ |
| ターミナル UI | ✗ | ✓ | ✗ | ✓ | ✓ | ✗ | ✓ | ✗ |
| **実行** | | | | | | | | |
| 中央サーバー | ✓ | ✗ | ◐ | ◐ | ◐ | ◐ | ✗ | ✗ |
| 全デバイスがオフでも実行 | ✓ | ✗ | ◐ | ✓² | ◐ | ◐ | ◐ | ◐ |
| 会話の同期 | ✓ | ✗ | ✗ | ◐ | ◐ | ◐ | ✗ | ◐ |
| クラウドサンドボックス | ✓ | ✗ | ✗ | ✓ | ✗ | ◐ | ✗ | ✗ |
| リモートマシン | ✓ | ◐ | ◐ | ◐ | ◐ | ✓ | ✓ | ✓ |
| 複数マシンでのタスク | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| マシンの切り替え | ✓ | ◐ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| **エージェント** | | | | | | | | |
| 独自のエージェント | ✓ | ✓ | ✓ | ✓ | ✓ | ✗ | ✗ | ✗ |
| ツールはシェルのみ | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| マルチエージェント | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| エージェントプロファイル | ✓ | ◐ | ✓ | ✓ | ✓ | ✗ | ✗ | ✗ |
| Skills | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ✓ |
| MCP | ✗ | ✓ | ✓ | ✓ | ✓ | ◐ | ✗ | ✓ |
| ブラウザ操作 | ✓ | ✗ | ◐ | ✓ | ✓ | ✓ | ✗ | ✓ |
| コンピューター操作 | ✗ | ✗ | ◐³ | ✗ | ✗ | ✓ | ✗ | ◐⁴ |
| **チーム** | | | | | | | | |
| マルチユーザー | ✓ | ✗ | ✗ | ◐² | ✗ | ✗ | ✗ | ✗ |
| 管理コンソール | ✓ | ✗ | ✗ | ◐² | ◐² | ✗ | ✗ | ✗ |
| ユーザーごとの分離 | ✓ | ✗ | ✗ | ◐² | ✗ | ✗ | ✗ | ✗ |

1. 起動する Claude Code と Codex のサブエージェント経由のみ。製品自身のサブスクリプションのサインインはコードにあるが、アプリに入口がない。
2. 有料のクラウド版またはエンタープライズ版のみ。
3. 試験的な機能で、手動で有効にする。
4. iOS と Android のシミュレーターのみで、デスクトップは操作できない。

## 各項目の意味

- **サブスクリプション**：API キーではなくサブスクリプションでサインインすること。製品が動かす Claude Code や Codex などのエージェント経由で使えるものも対応とします。
- **複数アカウント**、**アカウントのローテーション**：製品自身が行うものに限り、製品が動かすエージェントの機能は含めません。ローテーションは、あるアカウントが上限に達したとき別のアカウントで作業を続けることです。
- **中央サーバー**：1 台のサーバーがエージェントとすべての会話を持ち、手元のコンピューターはそのコマンドを実行するだけです。◐：サーバーはあるが、そのサーバー自身のマシンでしか実行しない。✗：マシンごとにエージェントが動く、またはサーバーがない。
- **全デバイスがオフでも実行**：コンピューターもスマートフォンもオフのまま、タスクが続きます。◐：常時稼働のマシンを自分で用意してインストールした場合のみ。
- **会話の同期**：どのマシンで実行した会話も 1 つの一覧にまとまり、どのデバイスでも同じ内容が見えます。◐：同じサーバーにつないだデバイスの間でのみ同期。
- **複数マシンでのタスク**：1 つの会話が複数のマシンで作業します。**マシンの切り替え**：会話を別のマシンに移して続けます。
- **独自のエージェント**：Claude Code や Codex などのエージェントを動かすのではなく、製品自身のエージェントがモデルを動かします。
- **ツールはシェルのみ**：モデルに渡すツールはシェルひとつで、すべての機能はその中のコマンドです。
- **コンピューター操作**：エージェントがスクリーンショット、マウス、キーボードでデスクトップを操作します。ブラウザに限りません。

<details>
<summary>出典</summary>

- Pi: [README](https://github.com/earendil-works/pi), [Providers](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md), [OAuth sign-ins](https://github.com/earendil-works/pi/tree/main/packages/ai/src/auth/oauth), [Credential store](https://github.com/earendil-works/pi/blob/main/packages/ai/src/auth/credential-store.ts), [SSH extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/ssh.ts), [Subagent extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/subagent/README.md), [Containerization](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/containerization.md), [Termux](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/termux.md), [Slash commands](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/slash-commands.md)
- DeepSeek Harness: [README](https://github.com/deepseek-ai/deepseek-harness), [Providers](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/providers.md), [Claude Code subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-claude-code/README.md), [Codex subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-codex/README.md), [Sign-in flows](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/llm/llm-pi-ai/src/login.ts), [Public deployments](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/public-deployments.md), [SSH](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/ssh.md), [Persistence](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/persistence.md), [Tool catalog](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/tool-catalog.md), [Agent presets](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/preset/agent-preset/README.md), [Browser use](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/browser-use.md), [Computer use](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/computer-use/README.md), [Desktop app](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/desktop/README.md)
- OpenHands: [README](https://github.com/OpenHands/OpenHands), [ACP agents](https://docs.openhands.dev/openhands/usage/agent-canvas/acp-agents), [Subscription sign-in](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/openai.py), [Credentials](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/credentials.py), [Backends](https://docs.openhands.dev/openhands/usage/agent-canvas/backends), [Remote backend](https://docs.openhands.dev/openhands/usage/agent-canvas/backend-setup/remote), [Phone and tablet access](https://docs.openhands.dev/openhands/usage/agent-canvas/mobile-access), [Cloud UI](https://docs.openhands.dev/openhands/usage/cloud/cloud-ui), [Enterprise vs. open source](https://docs.openhands.dev/enterprise/enterprise-vs-oss), [Tools](https://github.com/OpenHands/software-agent-sdk/tree/main/openhands-tools/openhands/tools)
- opencode: [Providers](https://opencode.ai/docs/providers/), [Provider plugins](https://github.com/anomalyco/opencode/tree/dev/packages/core/src/plugin/provider), [Web](https://opencode.ai/docs/web/), [Server](https://opencode.ai/docs/server/), [TUI](https://opencode.ai/docs/tui/), [Agents](https://opencode.ai/docs/agents/), [MCP servers](https://opencode.ai/docs/mcp-servers/), [Browser plugin](https://github.com/anomalyco/opencode/blob/dev/packages/plugin-browser/README.md), [Enterprise](https://opencode.ai/docs/enterprise/)
- Orca: [README](https://github.com/stablyai/orca), [Supported agents](https://www.onorca.dev/docs/agents/supported), [Accounts](https://www.onorca.dev/docs/agents/codex-hot-swap), [Ways to run](https://www.onorca.dev/docs/ways-to-run), [Remote servers](https://www.onorca.dev/docs/remote-servers), [Mobile](https://www.onorca.dev/docs/mobile), [Computer use](https://www.onorca.dev/docs/cli/computer-use), [Browser](https://www.onorca.dev/docs/browser/overview)
- herdr: [README](https://github.com/herdrdev/herdr), [Agents](https://herdr.dev/docs/agents/), [Connecting machines](https://herdr.dev/docs/connecting-machines/), [Persistence and remote](https://herdr.dev/docs/persistence-remote/), [Work from your phone](https://herdr.dev/docs/how-to-work/#work-from-your-phone), [Agent skill](https://herdr.dev/docs/agent-skill/)
- T3 Code: [README](https://github.com/pingdotgg/t3code), [Claude](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-claude.md), [Codex](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-codex.md), [Remote access](https://github.com/pingdotgg/t3code/blob/main/docs/user/remote-access.md), [Background service](https://github.com/pingdotgg/t3code/blob/main/docs/user/background-service.md), [Devices](https://github.com/pingdotgg/t3code/blob/main/docs/user/devices.md), [Outside agents](https://github.com/pingdotgg/t3code/blob/main/docs/user/outside-agents.md)

</details>

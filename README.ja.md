<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="brand/lockup-dark.svg">
    <img alt="Demi" src="brand/lockup-light.svg" height="72">
  </picture>
</p>

<p align="center">
  <strong>チャットも、仕事も、コーディングも。セルフホスト型の AI エージェント。<br>
  どのブラウザからでも使え、クラウドとあなたのすべてのコンピューターで作業します。</strong>
</p>

<p align="center">
  <a href="#ライセンス"><img alt="License" src="https://img.shields.io/badge/license-Apache%202.0%20%2B%20Demi%20License-blue"></a>
  <a href="https://github.com/wspl/demi/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/wspl/demi"></a>
  <a href="https://github.com/wspl/demi/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/wspl/demi/actions/workflows/ci.yml/badge.svg"></a>
</p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · <b>日本語</b> · <a href="README.ko.md">한국어</a>
</p>

<!-- Screenshot -->

Demi は、どのブラウザからでも使えるセルフホスト型の AI エージェントです。ちょっとした質問から仕事の依頼まで、調査、Web サイトの操作、ファイルの処理、コーディングをひとつの会話で行えます。

すべての会話はあなたのサーバーに集約され、どのデバイスからも同じ会話を開けます。コンピューターは作業を実行するだけです。エージェントはクラウドのサンドボックスや手元の複数のマシンで作業し、タブを閉じてもタスクは動き続けます。

<p align="center">
  <img src="docs/assets/concept.webp" alt="コンピューターやスマートフォンから 1 台の Demi サーバーに接続します。サーバーはすべての会話を保持し、ノート PC、デスクトップ、HomeLab、VPS 上のランナーやクラウドのサンドボックスを通じて作業します。" width="800">
</p>

> [!NOTE]
> Demi は開発の初期段階にあります。リリース間で互換性のない変更が入ることがあります。

## クイックスタート

必要なもの:

- root 権限のある Linux サーバー。OS は次のいずれか（[詳細](docs/delivery/installation.md#distributions)）:
  - Ubuntu 22.04 以降、Debian 12 以降
  - Fedora、RHEL 10、Rocky Linux、AlmaLinux
  - Arch Linux
- ドメイン名と、サーバーの前段で提供する HTTPS（[設定方法](docs/delivery/installation.md#https)）。

サーバーで次のコマンドを実行します:

```sh
curl -fsSL https://demi.sh/install.sh | sudo bash
```

インストーラーがいくつか質問し、Demi をセットアップします。続いて:

1. すぐにドメインを開き、管理者アカウントを作成します。
2. **Settings › Providers** で、サブスクリプションのアカウントでサインインするか、API キーを追加します。
3. 会話を始めます。会話はクラウドのサンドボックスで実行されます。
4. 自分のコンピューターで作業させるには、会話のホストメニューから **Add Device** を選び、表示されたコマンドをそのコンピューターで実行します。

質問なしでのインストールを含むすべてのオプションは[インストールガイド](docs/delivery/installation.md)を、新しいリリースへの移行は[アップグレード](docs/delivery/upgrades.md)を参照してください。

## 主な機能

### チャット、仕事、コーディング

- **ひとつの会話ですべてを**：ちょっとした質問も、複数ステップの仕事も同じ場所で。モードを切り替える必要はありません。
- **コーディング**：リポジトリを編集し、ビルドやテストを実行し、すべての変更を diff で確認できます。
- **調査**：Web を検索し、Web ページ、PDF、各種ドキュメントを読みます。
- **本物のブラウザ**：Demi が Chrome を操作して、サインイン、フォームへの入力、ファイルのダウンロードを行います。サイドパネルで様子を見ながら、いつでも操作を代われます。
- **ファイルとデータ**：ドキュメント、スプレッドシート、画像を扱い、結果を添付ファイルとして受け取れます。
- **共有**：会話全体、またはひとつの回答を、読み取り専用のリンクとして共有できます。

### どこからでも、どのマシンでも

- **どのブラウザでも**：デスクトップ、タブレット、スマートフォンで使えます。インストールは不要です。
- **常時稼働**：タブを閉じても、タスクはサーバー上で動き続けます。
- **すべての会話を一か所に**：会話はサーバーに保存され、エージェントが作業したマシンに散らばりません。ノート PC で始めた会話を、スマートフォンでリアルタイムに続けられます。
- **QR コードとパスキーでサインイン**：コードをスキャンしてスマートフォンでサインインするか、Touch ID、Face ID、Windows Hello を使えます。
- **専用のクラウドサンドボックス**：ユーザーごとに、あなたのサーバー上に gVisor で隔離された Linux サンドボックスが用意されます。
- **手持ちのコンピューターを接続**：macOS、Windows、Linux のマシンをコマンドひとつで接続できます。ポートを開ける必要はありません。
- **複数マシンでのタスク**：ひとつの会話で複数のマシンを使えます。たとえば Mac でビルドし、Windows でテストできます。
- **作業中にマシンを切り替え**：会話をクラウドとコンピューターの間で移せます。最初からやり直す必要はありません。

### エージェント

- **マルチエージェント**：Demi は大きなタスクを、並行して作業するエージェントのチームに分担させ、結果をまとめます。
- **エージェントプロファイル**：リサーチャーやレビュアーなどの専門エージェントを定義し、それぞれにモデル、思考量、指示、コマンドを設定できます。
- **ワークフロー**：ある種類の仕事のために、エージェントプロファイルと指示をひとまとめにします。プロジェクトの `.demi/workflows/` に置いてメッセージ入力欄で選び、リポジトリを通じてチームと共有できます。
- **スキル**：スキルをインストールして、Demi に新しい仕事を教えられます。
- **安全な認証情報の管理**：API キーやパスワードは安全なフォームで求められ、会話には表示されません。

### モデルとチーム

- **手持ちのサブスクリプションを利用**：Claude、ChatGPT、Grok Build のアカウントでサインインできます。サブスクリプションごとに複数のアカウントを追加でき、利用上限に達すると Demi がアカウントを切り替えます。
- **あらゆる API**：Anthropic、OpenAI、Gemini、および互換エンドポイントに対応します。
- **いつでもモデルを切り替え**：タスクの途中でも、モデルや思考量を変更できます。
- **マルチユーザー**：ユーザーごとに専用のサンドボックス、デバイス、会話があり、管理者のサブスクリプションも自分のサブスクリプションも使えます。
- **管理ツール**：ユーザーの追加、ロールの管理、パスワードのリセット、スマートフォン用サインインコードの送信を行えます。
- **コマンドひとつでインストールとアップグレード**：コマンドひとつでサーバーをセットアップし、もうひとつでアップグレードします。

## シェルの上に築いたハーネス

Demi はアプリであり、エージェントのハーネス（実行基盤）でもあります。Claude Code や Codex をラップするのではなく、独自のランタイムがすべてのモデルを動かします。Claude や ChatGPT のサブスクリプションは、モデルの利用にだけ使います。

多くのハーネスはモデルに長いツールの一覧を渡しますが、Demi が渡すのはシェルひとつだけです。ブラウザ、他のマシン、サブエージェントなど、それ以外の機能はすべてシェルの中の `demi` コマンドです:

```sh
demi browser open http://localhost:5173 && demi browser screenshot t1 | demi file view
demi host shell --host windows 'cargo test'
demi agent spawn --profile research --description "Compare pricing pages" < brief.md
```

- **組み合わせ自在**：コマンドはパイプや `&&` でつなげられるので、通常なら何回ものツール呼び出しが必要な作業を 1 ステップで行えます。
- **簡潔なプロンプト**：モデルに見えるのはコマンドグループごとに 1 行だけで、必要なときに `--help` を読みます。機能が増えても、リクエストは肥大化しません。
- **どこでも同じ**：どのマシンも、Rust で書かれた同じ bash 互換シェルと GNU ツール（[brush](https://github.com/reubeno/brush)、[uutils](https://github.com/uutils/coreutils)、ripgrep、[jaq](https://github.com/01mf02/jaq)）を実行します。クラウドで動くスクリプトは、macOS や Windows でもそのまま動きます。
- **どのモデルでも同じ**：どのプロバイダーのモデルにも同じひとつのツールが見えるので、モデルを切り替えてもエージェントにできることは変わりません。
- **長時間のジョブに対応**：コマンドはバックグラウンドで動き続け、終わると報告するので、開発サーバーやテストスイートが会話を止めません。

> [!NOTE]
> **予定**：Demi のハーネスを単独の TypeScript ライブラリとして提供し、Demi アプリなしで独自のエージェントを構築できるようにします。

## アーキテクチャ

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

- **エージェントはサーバーで、作業はランナーで**：会話、エージェント、設定はサーバーにあります。各マシンの小さなランナーは、ファイル、コマンド、ブラウザだけを扱います。ランナーからサーバーへ接続するので、NAT やファイアウォールの内側でも動作します。
- **どこでも同じランナー**：クラウドのサンドボックスもあなたのコンピューターも同じランナーを実行するので、すべての機能がどのマシンでも使えます。
- **帯域を節約**：大きな出力やファイルは、それを生成したマシンに残ります。ブラウザは開いたときにだけ読み込みます。
- **ネイティブ**：サーバー、ランナー、エージェントのツールは Rust で書かれています。Web アプリは Vue と TypeScript で作られています。

[設計ドキュメント](docs/README.md)は[概要](docs/overview.md)から読み始めてください（英語）。

## 比較

2026 年 10 月時点、各製品のドキュメントに基づきます。

| | Demi | [Pi](https://github.com/earendil-works/pi) | [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) | [OpenHands](https://github.com/OpenHands/OpenHands) | [opencode](https://github.com/anomalyco/opencode) | [Orca](https://github.com/stablyai/orca) | [herdr](https://github.com/herdrdev/herdr) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| ソ⁠ー⁠ス⁠コ⁠ー⁠ド | Apache-2.0。サーバーと Web アプリは [Demi License](LICENSE-DEMI) | MIT | MIT | MIT | MIT | MIT | Apache-2.0 |
| ハ⁠ー⁠ネ⁠ス | 独自 | 独自 | 独自 | 独自、または ACP エージェントをラップ | 独自 | Claude Code、Codex、OpenCode、Pi などをラップ | Claude Code、Codex、OpenCode、Pi などをラップ |
| イ⁠ン⁠タ⁠ー⁠フ⁠ェ⁠ー⁠ス | 複数ユーザーで使う Web アプリ、どのデバイスでも | ターミナル | ローカルの Web アプリとターミナル | Web アプリ | ターミナル、Web、デスクトップ | デスクトップアプリ、モバイル版コンパニオン | ターミナル UI |
| 会⁠話⁠の⁠保⁠存⁠先 | あなたのサーバーに一元化 | 各マシン | 各マシン | 各バックエンド | 各マシンまたはサーバー | 各マシン | 各マシン |
| タ⁠ス⁠ク⁠の⁠実⁠行⁠場⁠所 | クラウドのサンドボックス、またはあなたのどのコンピューターでも | 動かしているマシン | 動かしているマシン | ローカル、Docker、VM、クラウド | 動かしているマシン | ローカルのワークツリー、SSH ホスト、あなたのクラウド VM | ローカルまたは SSH 接続のマシン |
| ひ⁠と⁠つ⁠の⁠会⁠話⁠で⁠複⁠数⁠の⁠マ⁠シ⁠ン | 対応 | — | — | — | 非対応 | 非対応 | 非対応 |
| ツ⁠ー⁠ル⁠設⁠計 | シェルのみ | ツール呼び出し、コードモードも選択可 | ツール呼び出し、コードモードも選択可 | ツール呼び出し | ツール呼び出し、MCP ツールにはコードモード | ラップしたエージェントによる | ラップしたエージェントによる |
| モ⁠デ⁠ル | 任意。Claude、ChatGPT、Grok のサブスクリプションにも対応 | 多数のプロバイダーとサブスクリプション | DeepSeek ほか、API キーで利用 | 任意 | 任意 | ラップしたエージェントによる | ラップしたエージェントによる |
| ブ⁠ラ⁠ウ⁠ザ⁠の⁠自⁠動⁠操⁠作 | あなたのどのマシンでも | 拡張機能で | 実験的なプラグイン | SDK で提供 | — | 内蔵ブラウザ | — |
| チ⁠ー⁠ム⁠と⁠管⁠理 | 対応 | — | 共有アクセストークン | Enterprise 版 | 共有パスワード | — | — |

— は製品のドキュメントに記載がないことを示します。

<details>
<summary>出典</summary>

- Pi: [README](https://github.com/earendil-works/pi), [Sessions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/sessions.md), [Providers](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md), [Settings](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/settings.md)
- DeepSeek Harness: [README](https://github.com/deepseek-ai/deepseek-harness), [Architecture](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/architecture.md), [Providers](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/providers.md), [Tool catalog](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/tool-catalog.md), [Public deployments](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/public-deployments.md)
- OpenHands: [README](https://github.com/OpenHands/OpenHands), [Backends](https://docs.openhands.dev/openhands/usage/agent-canvas/backends), [Enterprise vs. open source](https://docs.openhands.dev/enterprise/enterprise-vs-oss), [Browser use](https://docs.openhands.dev/sdk/guides/agent-browser-use)
- opencode: [Web](https://opencode.ai/docs/web/), [Server](https://opencode.ai/docs/server/), [Tools](https://opencode.ai/docs/tools/), [Providers](https://opencode.ai/docs/providers/)
- Orca: [README](https://github.com/stablyai/orca), [Supported agents](https://www.onorca.dev/docs/agents/supported), [Ways to run](https://www.onorca.dev/docs/ways-to-run), [Browser](https://www.onorca.dev/docs/browser/overview)
- herdr: [README](https://github.com/herdrdev/herdr), [Agents](https://herdr.dev/docs/agents/), [Connecting machines](https://herdr.dev/docs/connecting-machines/), [Session state](https://herdr.dev/docs/session-state/)

</details>

## ドキュメント

- [設計ドキュメント](docs/README.md)：Demi の各部分の仕組み。
- [コントリビューション](CONTRIBUTING.md)：開発環境、チェック、プロバイダーの追加方法。
- [セキュリティ](SECURITY.md)：脆弱性の報告方法と、データの保存場所。

これらのドキュメントは英語です。

## ライセンス

Demi の大部分は [Apache License 2.0](LICENSE) で公開されており、どのような用途にも使えます。対象は、エージェントのランタイム、モデルプロバイダー、あなたのマシンにインストールするランナー、コマンド、プラグインとその SDK、UI コンポーネントライブラリとそのギャラリー、プロトコル、ドキュメントです。

サーバーと Web アプリ（`crates/backend*`、`crates/server`、`crates/machine-manager`、`packages/web`）は [Demi License](LICENSE-DEMI) で公開されています。これは Apache License 2.0 に次の条件を加えたものです:

- **無料で利用可能**：社内で自分たちのチームのために使うことも含め、利用、改変、実行できます。
- **サポートの提供は自由**：他者が自ら運用する Demi の導入、統合、サポートを有償で提供できます。
- **サービスとしての再販は不可**：有償のホスティングサービスやマルチテナントサービスとして他者に提供すること、また実質的に Demi である製品として提供することはできません。
- **ブランドの保持**：Web アプリ内の Demi の名称、ロゴ、著作権表示を削除または変更しないでください。

その他の用途については [support@demi.sh](mailto:support@demi.sh) までお問い合わせください。

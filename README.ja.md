<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/brand/lockup-dark.svg">
    <img alt="Demi" src="docs/assets/brand/lockup-light.svg" height="72">
  </picture>
</p>

<p align="center">
  <strong>あなたのエージェントを、すべてのマシンで。<br>
  自分のサーバーでホストする、Web ベースの AI エージェント。</strong>
</p>

<p align="center">
  <a href="#ライセンス"><img alt="License" src="https://img.shields.io/badge/license-Apache%202.0-blue"></a>
  <a href="https://github.com/demi-sh/demi/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/demi-sh/demi"></a>
  <a href="https://github.com/demi-sh/demi/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/demi-sh/demi/actions/workflows/ci.yml/badge.svg"></a>
</p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.de.md">Deutsch</a> · <a href="README.es.md">Español</a> · <a href="README.fr.md">Français</a> · <a href="README.pt-BR.md">Português</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · <b>日本語</b> · <a href="README.ko.md">한국어</a>
</p>

<p align="center">
  <img src="docs/assets/product.webp" alt="ブラウザの Demi：エージェントが iPhone アプリを 2 列レイアウトにし、その返信にはコメントとサイドの質問、下には返信の候補。サイドパネルには半分折りたたんだ iPhone Duo で動くアプリ、スマートフォンには同じ会話。" width="800">
</p>

Demi は、自分のサーバーでホストする Web ベースの AI エージェントです。どのブラウザからでも開け、調査、Web サイトの操作、ファイル、コードと、何でも任せられます。会話はあなたのサーバーに残り、作業はクラウドのサンドボックスや手元のマシンで動き、タブを閉じても続きます。

> [!NOTE]
> Demi は開発の初期段階にあります。リリース間で互換性のない変更が入ることがあります。

## クイックスタート

数クリックで Demi をデプロイ：

<p>
  <a href="docs/delivery/railway.md"><img alt="Deploy on Railway" src="https://railway.com/button.svg" height="40"></a>
  <a href="docs/delivery/fly.md"><img alt="Deploy on Fly.io" src="docs/assets/deploy/deploy-on-fly.svg" height="40"></a>
  <a href="docs/delivery/docker-compose.md"><img alt="Run with Docker Compose" src="docs/assets/deploy/run-with-docker-compose.svg" height="40"></a>
</p>

または、ドメイン名のある自分の Linux サーバーにインストール：

```sh
curl -fsSL https://demi.sh/install.sh | sudo bash
```

または、Claude Code や Codex などのコーディングエージェントにインストールを任せます：

```text
https://raw.githubusercontent.com/demi-sh/demi/main/docs/delivery/install-with-an-agent.md に従って、私のサーバーに Demi をインストールしてください
```

インストール後は Demi を開いて管理者アカウントを作り、**Settings › Providers** でモデルを追加します。対応システム、HTTPS、Canary チャンネル、各方法の費用は[インストールガイド](docs/delivery/installation.md)にあります。

<p align="center">
  <img src="docs/assets/concept.webp" alt="コンピューターやスマートフォンから 1 台の Demi サーバーに接続します。サーバーはすべての会話を保持し、ノート PC、デスクトップ、HomeLab、VPS 上のランナーやクラウドのサンドボックスを通じて作業します。" width="800">
</p>

## 主な機能

### チャット、仕事、コーディング

- **ひとつの会話ですべてを**：ちょっとした質問も、複数ステップの仕事も。
- **コーディング**：リポジトリを編集し、ビルドとテストを実行し、変更を diff で確認。
- **調査**：Web を検索し、Web ページ、PDF、ドキュメントを読みます。
- **本物のブラウザ**：Demi が Chrome を操作。見守りながら、いつでも操作を代われます。
- **スマートフォンアプリ**：iOS シミュレーターと Android エミュレーターでアプリをインストールしてテスト。
- **ファイルとデータ**：ドキュメント、スプレッドシート、画像を扱い、添付ファイルで返します。
- **画像、動画、音声**：追加したモデルプロバイダーで作れます。
- **コメントと質問**：返信の文字を選んで質問できます。
- **共有と検索**：読み取り専用リンクで共有し、すべてのメッセージを検索。

### どこからでも、どのマシンでも

- **どのブラウザでも**：コンピューター、タブレット、スマートフォン、折りたたみ端末で、インストール不要。
- **常時稼働**：タブを閉じてもタスクは続き、終わると通知します。
- **会話の置き場所はひとつ**：ノート PC で始めて、スマートフォンで続けます。
- **専用のクラウドサンドボックス**：ユーザーごとに専用の Linux サンドボックス。
- **手持ちのコンピューター**：macOS、Windows、Linux をコマンドひとつで接続。
- **複数マシンでひとつのタスク**：Mac でビルド、Windows でテスト、作業中の切り替えも。
- **すべてのマシンのファイル**：各マシンのファイルを閲覧、プレビュー、検索。
- **ポートフォワーディング**：あるマシンから別のマシンのポートにつなげます。
- **好きな方法でサインイン**：QR コードまたはパスキー、9 つの言語に対応。

### エージェント

- **マルチエージェント**：エージェントのチームが大きなタスクを分担し、並行して作業。
- **プロファイルとワークフロー**：専門エージェントを、リポジトリを通じて共有。
- **スケジュール**：タイマー、cron、変化をきっかけに会話を起こします。
- **会話どうしの連携**：会話から別の会話にメッセージを送ったり、読んだりできます。
- **スキル**：Demi に新しい仕事を教えます。
- **安全な認証情報**：秘密はフォームで受け取り、会話には出ません。

### モデルとチーム

- **手持ちのサブスクリプション**：Claude、ChatGPT、Grok Build。上限に達するとアカウントを切り替えます。
- **あらゆる API**：Anthropic、OpenAI、Gemini、Kimi・GLM・MiniMax などのコーディングプラン、ローカルモデル。
- **いつでもモデルを切り替え**、タスクの途中でも。
- **使用量と費用**：各サブスクリプションの上限と、作業にかかった費用を確認。
- **マルチユーザー**：ユーザーごとに専用のサンドボックス、デバイス、会話。

## シェルの上に築いたエージェント

Demi は Claude Code や Codex のラッパーではなく、独自のランタイムがすべてのモデルを動かします。多くのエージェントはモデルに長いツールの一覧を渡しますが、Demi が渡すのはシェルひとつです。ブラウザ、ほかのマシン、サブエージェントは、その中の `demi` コマンドです：

```sh
T=$(demi browser tab new) && demi browser open $T http://localhost:5173 && demi browser screenshot $T | demi file view
demi host shell --host windows 'cargo test'
demi agent spawn --profile research --description "Compare pricing pages" < brief.md
```

- **組み合わせられる**：コマンドはパイプでつながり、ひとつのステップで複数のツール呼び出し分の仕事をします。
- **軽いプロンプト**：モデルはコマンドの `--help` を必要なときだけ読みます。
- **どこでも同じシェル**：Rust で書かれた bash 互換のシェルが、すべてのマシンで動きます。
- **どのモデルでも同じ**：モデルを変えても、エージェントにできることは変わりません。
- **長い仕事のために**：コマンドはバックグラウンドで動き、終わると報告します。

> [!NOTE]
> **予定**：Demi サーバーの OpenAPI。あなたのプログラムから、会話、エージェント、クラウドのサンドボックスを使えるようにします。

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
  │                │  │ Linux)                       │
  └────────────────┘  └──────────────────────────────┘
```

- **サーバーがエージェントを動かし、ランナーが作業する**：各ランナーは外向きに接続するので、NAT やファイアウォールの内側でも動きます。
- **どこでも同じランナー**：すべての機能がすべてのマシンで使えます。
- **帯域にやさしい**：大きなファイルは作られたマシンに残り、開いたときだけ読み込みます。
- **ネイティブ**：サーバー、ランナー、ツールは Rust、Web アプリは Vue と TypeScript。

[設計ドキュメント](docs/README.md)は[概要](docs/overview.md)から読み始めてください（英語）。

## 比較

OpenHands、Orca、T3 Code、DeepSeek Harness、opencode、Pi、herdr と Demi の機能ごとの比較は [Demi の比較](docs/comparison.ja.md)をご覧ください。

## ドキュメント

- [設計ドキュメント](docs/README.md)：Demi の各部分の仕組み
- [コントリビューション](CONTRIBUTING.md)：開発環境、チェック、プロバイダーの追加方法
- [セキュリティ](SECURITY.md)：脆弱性の報告方法と、データの保存場所

これらのドキュメントは英語です。

## ライセンス

Demi は [Apache License 2.0](LICENSE) で公開されています。

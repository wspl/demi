<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/brand/lockup-dark.svg">
    <img alt="Demi" src="docs/assets/brand/lockup-light.svg" height="72">
  </picture>
</p>

<p align="center">
  <strong>你的 Agent，遍及你所有的机器。<br>
  部署在你自己服务器上的网页版 AI Agent。</strong>
</p>

<p align="center">
  <a href="#许可证"><img alt="License" src="https://img.shields.io/badge/license-Apache%202.0-blue"></a>
  <a href="https://github.com/demi-sh/demi/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/demi-sh/demi"></a>
  <a href="https://github.com/demi-sh/demi/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/demi-sh/demi/actions/workflows/ci.yml/badge.svg"></a>
</p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.de.md">Deutsch</a> · <a href="README.es.md">Español</a> · <a href="README.fr.md">Français</a> · <a href="README.pt-BR.md">Português</a> · <b>简体中文</b> · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a>
</p>

<p align="center">
  <img src="docs/assets/product.webp" alt="浏览器里的 Demi：智能体为一个 iPhone App 做了双栏布局，回复上有一条评论和一个追问，下面是回复建议；侧边面板里是半折叠的 iPhone Duo 上运行的 App，手机上是同一个对话。" width="800">
</p>

Demi 是部署在你自己服务器上的网页版 AI Agent，在任何浏览器中都能打开，什么事都可以交给它：查资料、操作网站、处理文件、写代码。对话都保存在你的服务器上，工作在云端沙箱或你自己的机器上运行，关掉网页也照常进行。

> [!NOTE]
> Demi 目前处于早期开发阶段，版本之间可能存在不兼容的变更。

## 快速开始

点几下即可部署 Demi：

<p>
  <a href="docs/delivery/railway.md"><img alt="Deploy on Railway" src="https://railway.com/button.svg" height="36"></a>
  <a href="https://render.com/deploy?repo=https://github.com/demi-sh/demi"><img alt="Deploy on Render" src="docs/assets/deploy/deploy-on-render.svg" height="36"></a>
  <a href="docs/delivery/fly.md"><img alt="Deploy on Fly.io" src="docs/assets/deploy/deploy-on-fly.svg" height="36"></a>
  <a href="docs/delivery/docker-compose.md"><img alt="Run with Docker Compose" src="docs/assets/deploy/run-with-docker-compose.svg" height="36"></a>
</p>

或者安装到你自己的 Linux 服务器上（需要一个域名）：

```sh
curl -fsSL https://demi.sh/install.sh | sudo bash
```

或者让 Claude Code、Codex 等编程 Agent 帮你安装：

```text
按照 https://raw.githubusercontent.com/demi-sh/demi/main/docs/delivery/install-with-an-agent.md 在我的服务器上安装 Demi
```

装好后打开 Demi，创建管理员账号，在 **Settings › Providers** 中添加模型。支持的系统、HTTPS、Canary 渠道和各方案的费用见[安装指南](docs/delivery/installation.md)。

<p align="center">
  <img src="docs/assets/concept.webp" alt="在电脑或手机上连到同一台 Demi 服务器，所有对话都保存在这里；它通过笔记本、台式机、HomeLab、VPS 上的 Runner 或云端沙箱完成工作。" width="800">
</p>

## 核心特性

### 聊天、办事、写代码

- **一个对话搞定所有事**：简单问题和多步骤任务都在这里。
- **编程**：修改代码仓库，运行构建和测试，每处改动都能以 diff 查看。
- **调研**：搜索网页，阅读网页、PDF 和各类文档。
- **真实浏览器**：Demi 会操作 Chrome，你可以实时观看，随时接手。
- **手机应用**：在 iOS 模拟器和 Android 模拟器上安装、测试应用。
- **文件与数据**：处理文档、表格和图片，结果以附件发回。
- **图片、视频与音频**：通过你添加的模型服务生成。
- **评论与提问**：在回复里选中文字，就能就此提问。
- **分享与搜索**：分享只读链接，搜索任意一条消息。

### 随时随地，任意机器

- **浏览器即用**：电脑、平板、手机、折叠屏都行，无需安装。
- **后台运行**：关掉网页后任务继续执行，完成时通知你。
- **对话集中管理**：在笔记本上开始，在手机上继续。
- **专属云端沙箱**：每个用户都有独立的 Linux 沙箱。
- **接入你自己的电脑**：一条命令连接 macOS、Windows 和 Linux。
- **多机协作**：在 Mac 上构建、在 Windows 上测试，任务中途也能切换机器。
- **每台机器的文件**：浏览、预览和搜索每台机器上的文件。
- **端口转发**：从一台机器访问另一台机器的端口。
- **登录方式随你选**：扫码或 Passkey，支持九种语言。

### Agent 能力

- **多 Agent 协作**：一组 Agent 拆分大任务，并行工作。
- **Agent Profile 与工作流**：各司其职的专家 Agent，可随仓库共享。
- **定时任务**：按时间、cron 或在出现变化时唤醒对话。
- **对话协作**：一个对话可以给另一个对话发消息，或读取它的记录。
- **Skills**：教 Demi 学会新任务。
- **安全的凭据管理**：密钥通过表单索取，不会出现在对话中。

### 模型与团队

- **使用已有订阅**：Claude、ChatGPT 和 Grok Build，额度用完自动轮换账号。
- **任意 API**：Anthropic、OpenAI、Gemini，Kimi、GLM、MiniMax 等 Coding Plan，以及本地模型。
- **随时切换模型**，任务进行中也可以。
- **用量与费用**：查看每个订阅的额度和任务花费。
- **多用户**：每个用户拥有独立的沙箱、设备和对话。

## 基于 Shell 的 Agent

Demi 不套壳 Claude Code 或 Codex，而是由自己的运行时驱动所有模型。大多数 Agent 会给模型一长串工具，Demi 只给一个：Shell。浏览器、其他机器、子 Agent 都是这个 Shell 里的 `demi` 命令：

```sh
T=$(demi browser tab new) && demi browser open $T http://localhost:5173 && demi browser screenshot $T | demi file view
demi host shell --host windows 'cargo test'
demi agent spawn --profile research --description "Compare pricing pages" < brief.md
```

- **可组合**：命令可以用管道串联，一步完成通常需要多次工具调用的事。
- **精简的提示词**：模型只在需要时才读命令的 `--help`。
- **处处相同的 Shell**：每台机器都运行同一个用 Rust 编写、兼容 bash 的 Shell。
- **所有模型一致**：换模型不会改变 Agent 能做的事。
- **为长任务而生**：命令在后台运行，完成后汇报。

> [!NOTE]
> **计划中**：为 Demi 服务器提供 OpenAPI，让你自己的程序也能使用它的对话、Agent 和云端沙箱。

## 架构

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

- **服务器运行 Agent，Runner 负责干活**：每个 Runner 主动向外连接，因此在 NAT 和防火墙后也能工作。
- **处处同一个 Runner**：每个功能在每台机器上都能用。
- **节省带宽**：大文件留在生成它的机器上，打开时才加载。
- **原生**：服务器、Runner 和工具用 Rust 编写，网页应用用 Vue 和 TypeScript。

设计文档见 [docs/](docs/README.md)，建议从[概览](docs/overview.md)开始阅读（英文）。

## 对比

Demi 与 OpenHands、Orca、T3 Code、DeepSeek Harness、opencode、Pi、herdr 的逐项对比，见 [Demi 对比](docs/comparison.zh-CN.md)。

## 文档

- [设计文档](docs/README.md)：Demi 各部分的工作原理
- [参与贡献](CONTRIBUTING.md)：开发环境、检查项，以及如何添加模型提供方
- [安全](SECURITY.md)：漏洞报告方式与数据存储说明

以上文档均为英文。

## 许可证

Demi 采用 [Apache License 2.0](LICENSE) 发布。

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="brand/lockup-dark.svg">
    <img alt="Demi" src="brand/lockup-light.svg" height="72">
  </picture>
</p>

<p align="center">
  <strong>自托管的 AI Agent，聊天、办事、写代码都能胜任。<br>
  在任何浏览器中使用，在云端和你所有的电脑上工作。</strong>
</p>

<p align="center">
  <a href="#许可证"><img alt="License" src="https://img.shields.io/badge/license-Apache%202.0%20%2B%20Demi%20License-blue"></a>
  <a href="https://github.com/wspl/demi/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/wspl/demi"></a>
  <a href="https://github.com/wspl/demi/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/wspl/demi/actions/workflows/ci.yml/badge.svg"></a>
</p>

<p align="center">
  <a href="README.md">English</a> · <b>简体中文</b>
</p>

<!-- Screenshot -->

Demi 是一个自托管的 AI Agent，在任何浏览器中都能使用。随口问个问题，或者把一整件事交给它：查资料、操作网站、处理文件、写代码，都在同一个对话里完成。

所有对话都集中保存在你的服务器上，所有设备看到的是同一份。你的电脑只负责执行：Agent 在云端沙箱或你自己的多台机器上工作，关掉网页后任务照常进行。

<p align="center">
  <img src="docs/assets/concept.webp" alt="在电脑或手机上连到同一台 Demi 服务器，所有对话都保存在这里；它通过笔记本、台式机、HomeLab、VPS 上的 Runner 或云端沙箱完成工作。" width="800">
</p>

> [!NOTE]
> Demi 目前处于早期开发阶段，版本之间可能存在不兼容的变更。

## 快速开始

你需要：

- 一台有 root 权限的 Linux 服务器，系统为以下之一（[详细说明](docs/delivery/installation.md#distributions)）：
  - Ubuntu 22.04 及以上、Debian 12 及以上
  - Fedora、RHEL 10、Rocky Linux、AlmaLinux
  - Arch Linux
- 一个域名，并在服务器前提供 HTTPS（[配置方法](docs/delivery/installation.md#https)）。

然后在服务器上运行：

```sh
curl -fsSL https://demi.sh/install.sh | sudo bash
```

安装程序会问几个问题，然后完成安装。接下来：

1. 立即打开你的域名，创建管理员账号。
2. 进入 **Settings › Providers**，用订阅账号登录或添加 API 密钥。
3. 开始一个对话，它会在你的云端沙箱中运行。
4. 如需在自己的电脑上工作，在对话的 Host 菜单中选择 **Add Device**，然后在那台电脑上运行显示的命令。

全部选项（包括免交互安装）请参阅[安装指南](docs/delivery/installation.md)，升级方法请参阅[升级说明](docs/delivery/upgrades.md)。

## 核心特性

### 聊天、办事、写代码

- **一个对话搞定所有事**：简单问题直接回答，复杂任务也在同一个对话里完成，无需切换模式。
- **编程**：修改代码仓库，运行构建和测试，每处改动都能以 diff 形式查看。
- **调研**：搜索网页，阅读网页、PDF 和各类文档。
- **真实浏览器**：Demi 会操作 Chrome 完成登录、填写表单、下载文件。你可以在侧边面板中实时观看，随时接手。
- **文件与数据**：处理文档、表格和图片，结果以附件形式发回给你。

### 随时随地，任意机器

- **浏览器即用**：电脑、平板、手机均可使用，无需安装任何软件。
- **后台运行**：关掉网页后，任务仍在服务器上继续执行。
- **对话集中管理**：对话统一保存在服务器上，不会散落在 Agent 工作过的各台机器里。在笔记本上开始，在手机上继续，实时同步。
- **扫码与 Passkey 登录**：手机扫码即可登录，也支持 Touch ID、Face ID 和 Windows Hello。
- **专属云端沙箱**：每个用户在你的服务器上拥有独立的 Linux 沙箱，基于 gVisor 隔离。
- **接入你自己的电脑**：一条命令即可连接 macOS、Windows 和 Linux 机器，无需开放端口。
- **多机协作**：一个对话可以同时使用多台机器，比如在 Mac 上构建、在 Windows 上测试。
- **随时切换机器**：对话可以在云端和你的电脑之间迁移，无需从头开始。
- **分享预览**：通过临时公网链接，分享任意机器上运行的开发服务器。

### Agent 能力

- **多 Agent 协作**：Demi 会把大任务拆给一组并行工作的 Agent，最后汇总它们的结果。
- **Agent Profile**：定义各司其职的专家 Agent，比如调研员、审查员，分别设置模型、思考强度、指令和命令。
- **工作流**：把一组 Agent Profile 和工作指令打包成预设，放在项目的 `.demi/workflows/` 中，在输入框里一键选用，并随仓库与团队共享。
- **Skills**：安装 Skill，让 Demi 学会新的任务。
- **安全的凭据管理**：API 密钥和密码通过安全表单索取，不会出现在对话中。

### 模型与团队

- **使用已有订阅**：用 Claude、ChatGPT 或 Grok Build 账号直接登录。每种订阅可添加多个账号并随时切换，各账号的用量额度一目了然。
- **任意 API**：支持 Anthropic、OpenAI、Gemini 以及任何兼容端点。
- **随时切换模型**：随时更换模型或思考强度，任务进行中也可以。
- **多用户**：每个用户拥有独立的沙箱、模型、设备和对话。
- **管理后台**：添加用户、管理角色、重置密码、发放手机登录码。
- **一键安装与升级**：一条命令部署服务器，一条命令完成升级。

## 基于 Shell 的 Harness

Demi 既是 Agent 的界面，也是 Agent 的 Harness（运行框架）。它不套壳 Claude Code 或 Codex，而是由自己的运行时驱动所有模型；用 Claude 或 ChatGPT 订阅登录时，只调用订阅中的模型。

大多数 Harness 会给模型一长串工具，Demi 只给一个：Shell。浏览器、其他机器、子 Agent 等所有能力，都是这个 Shell 里的 `demi` 命令：

```sh
demi browser open http://localhost:5173 && demi browser screenshot t1 | demi file view
demi host shell --host windows 'cargo test'
demi agent spawn --profile research --description "Compare pricing pages" < brief.md
```

- **可组合**：命令之间可以用管道和 `&&` 串联，通常需要多次工具调用的事，一步就能完成。
- **提示词精简**：每组命令在提示词中只占一行，模型需要时再读 `--help`。能力再多，也不会让每次请求变得臃肿。
- **处处一致**：每台机器都运行同一套用 Rust 编写的 bash 兼容 Shell 和 GNU 工具（[brush](https://github.com/reubeno/brush)、[uutils](https://github.com/uutils/coreutils)、ripgrep、[jaq](https://github.com/01mf02/jaq)）。在云端能跑的脚本，在 macOS 和 Windows 上同样能跑。
- **模型无关**：所有模型面对的都是同一个工具，换模型不会改变 Agent 的能力。
- **适合长任务**：命令在后台持续运行，结束后自动汇报，开发服务器和测试套件不会阻塞对话。

> [!NOTE]
> **规划中**：Demi Harness 将作为独立的 TypeScript 库提供，你可以不依赖 Demi 应用，直接基于它构建自己的 Agent。

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
  │ (gVisor)       │  │ Linux)                       │
  └────────────────┘  └──────────────────────────────┘
```

- **服务器运行 Agent，Runner 负责执行**：对话、Agent 和设置都在服务器上。每台机器上的轻量 Runner 只负责文件、命令和浏览器操作。Runner 主动连接服务器，在 NAT 和防火墙后也能正常工作。
- **同一个 Runner**：云端沙箱和你的电脑运行同一个 Runner，所有功能在每台机器上都可用。
- **节省带宽**：大段输出和文件保留在产生它们的机器上，只在你打开时才加载。
- **原生实现**：服务器、Runner 和 Agent 的工具均由 Rust 编写，网页应用基于 Vue 和 TypeScript。

设计文档见 [docs/](docs/README.md)，建议从[概览](docs/overview.md)开始阅读（英文）。

## 对比

截至 2026 年 10 月，依据各产品的官方文档整理。

| | Demi | [Pi](https://github.com/earendil-works/pi) | [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) | [OpenHands](https://github.com/OpenHands/OpenHands) | [opencode](https://github.com/anomalyco/opencode) | [Orca](https://github.com/stablyai/orca) | [herdr](https://github.com/herdrdev/herdr) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 源⁠代⁠码 | Apache-2.0；服务器和网页应用为 [Demi License](LICENSE-DEMI) | MIT | MIT | MIT | MIT | MIT | Apache-2.0 |
| Harness | 自研 | 自研 | 自研 | 自研，或套壳 ACP Agent | 自研 | 套壳 Claude Code、Codex、OpenCode、Pi 等 | 套壳 Claude Code、Codex、OpenCode、Pi 等 |
| 使⁠用⁠界⁠面 | 多用户网页应用，任意设备 | 终端 | 本地网页与终端 | 网页 | 终端、网页、桌面 | 桌面应用，配套手机应用 | 终端界面 |
| 对⁠话⁠存⁠放 | 集中在你的服务器上 | 各台机器各自保存 | 各台机器各自保存 | 各个后端各自保存 | 各台机器或服务器各自保存 | 各台机器各自保存 | 各台机器各自保存 |
| 运⁠行⁠位⁠置 | 云端沙箱或你的任意电脑 | 运行它的那台机器 | 运行它的那台机器 | 本地、Docker、虚拟机或云端 | 运行它的那台机器 | 本地工作树、SSH 主机或你的云虚拟机 | 本地或 SSH 机器 |
| 一⁠个⁠对⁠话⁠操⁠作⁠多⁠台⁠机⁠器 | 支持 | — | — | — | 不支持 | 不支持 | 不支持 |
| 工⁠具⁠方⁠案 | 纯 Shell | 工具调用，可选代码模式 | 工具调用，可选代码模式 | 工具调用 | 工具调用，MCP 工具用代码模式 | 取决于被套壳的 Agent | 取决于被套壳的 Agent |
| 模⁠型 | 任意，支持 Claude、ChatGPT、Grok 订阅 | 多家提供商及订阅 | DeepSeek 等，使用 API 密钥 | 任意 | 任意 | 取决于被套壳的 Agent | 取决于被套壳的 Agent |
| 浏⁠览⁠器 | 在你的任意机器上 | 通过扩展 | 实验性插件 | SDK 中提供 | — | 内置浏览器 | — |
| 团⁠队⁠与⁠管⁠理 | 支持 | — | 共享访问令牌 | Enterprise 版 | 共享密码 | — | — |

— 表示该产品的文档中未提及。

<details>
<summary>来源</summary>

- Pi: [README](https://github.com/earendil-works/pi), [Sessions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/sessions.md), [Providers](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md), [Settings](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/settings.md)
- DeepSeek Harness: [README](https://github.com/deepseek-ai/deepseek-harness), [Architecture](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/architecture.md), [Providers](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/providers.md), [Tool catalog](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/tool-catalog.md), [Public deployments](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/public-deployments.md)
- OpenHands: [README](https://github.com/OpenHands/OpenHands), [Backends](https://docs.openhands.dev/openhands/usage/agent-canvas/backends), [Enterprise vs. open source](https://docs.openhands.dev/enterprise/enterprise-vs-oss), [Browser use](https://docs.openhands.dev/sdk/guides/agent-browser-use)
- opencode: [Web](https://opencode.ai/docs/web/), [Server](https://opencode.ai/docs/server/), [Tools](https://opencode.ai/docs/tools/), [Providers](https://opencode.ai/docs/providers/)
- Orca: [README](https://github.com/stablyai/orca), [Supported agents](https://www.onorca.dev/docs/agents/supported), [Ways to run](https://www.onorca.dev/docs/ways-to-run), [Browser](https://www.onorca.dev/docs/browser/overview)
- herdr: [README](https://github.com/herdrdev/herdr), [Agents](https://herdr.dev/docs/agents/), [Connecting machines](https://herdr.dev/docs/connecting-machines/), [Session state](https://herdr.dev/docs/session-state/)

</details>

## 文档

- [设计文档](docs/README.md)：Demi 各部分的工作原理。
- [参与贡献](CONTRIBUTING.md)：开发环境、检查项，以及如何添加模型提供方。
- [安全](SECURITY.md)：漏洞报告方式与数据存储说明。

以上文档均为英文。

## 许可证

Demi 的大部分代码采用 [Apache License 2.0](LICENSE) 发布，包括 Agent 运行时、模型提供方、装在你机器上的 Runner、命令、插件及其 SDK、UI 组件库及其图库、各类协议和文档，可以用于任何用途。

服务器和网页应用（`crates/backend*`、`crates/server`、`crates/machine-manager` 和 `packages/web`）采用 [Demi License](LICENSE-DEMI) 发布，即在 Apache License 2.0 基础上附加了以下条件：

- **免费使用**：可以使用、修改和运行，包括在公司内部供自己的团队使用。
- **可提供商业支持**：可以为他人自行运行的 Demi 提供收费的部署、集成或支持服务。
- **不得转售为服务**：不得将其作为收费的托管服务或多租户服务提供给他人，也不得提供实质上就是 Demi 的产品。
- **保留品牌**：不得删除或修改网页应用中的 Demi 名称、Logo 或版权声明。

其他用途请联系 [support@demi.sh](mailto:support@demi.sh)。

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/brand/lockup-dark.svg">
    <img alt="Demi" src="docs/assets/brand/lockup-light.svg" height="72">
  </picture>
</p>

<p align="center">
  <strong>你的智慧體，遍及你所有的機器。<br>
  架設在你自己伺服器上的網頁版 AI 智慧體。</strong>
</p>

<p align="center">
  <a href="#授權條款"><img alt="License" src="https://img.shields.io/badge/license-Apache%202.0-blue"></a>
  <a href="https://github.com/demi-sh/demi/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/demi-sh/demi"></a>
  <a href="https://github.com/demi-sh/demi/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/demi-sh/demi/actions/workflows/ci.yml/badge.svg"></a>
</p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.de.md">Deutsch</a> · <a href="README.es.md">Español</a> · <a href="README.fr.md">Français</a> · <a href="README.pt-BR.md">Português</a> · <a href="README.zh-CN.md">简体中文</a> · <b>繁體中文</b> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a>
</p>

<p align="center">
  <img src="docs/assets/product.webp" alt="瀏覽器裡的 Demi：智慧代理為一個 iPhone App 做了雙欄版面，回覆上有一則評論和一個追問，下方是回覆建議；側邊面板裡是半摺疊的 iPhone Duo 上執行的 App，手機上是同一個對話。" width="800">
</p>

Demi 是架設在你自己伺服器上的網頁版 AI 智慧體，在任何瀏覽器中都能開啟，什麼事都可以交給它：查資料、操作網站、處理檔案、寫程式。對話都儲存在你的伺服器上，工作在雲端沙箱或你自己的機器上執行，關掉網頁也照常進行。

> [!NOTE]
> Demi 目前處於早期開發階段，版本之間可能有不相容的變更。

## 快速開始

點幾下即可部署 Demi：

<p>
  <a href="docs/delivery/railway.md"><img alt="Deploy on Railway" src="https://railway.com/button.svg" height="36"></a>
  <a href="https://render.com/deploy?repo=https://github.com/demi-sh/demi"><img alt="Deploy on Render" src="docs/assets/deploy/deploy-on-render.svg" height="36"></a>
  <a href="docs/delivery/fly.md"><img alt="Deploy on Fly.io" src="docs/assets/deploy/deploy-on-fly.svg" height="36"></a>
  <a href="docs/delivery/docker-compose.md"><img alt="Run with Docker Compose" src="docs/assets/deploy/run-with-docker-compose.svg" height="36"></a>
</p>

或者安裝到你自己的 Linux 伺服器上（需要一個網域名稱）：

```sh
curl -fsSL https://demi.sh/install.sh | sudo bash
```

或者讓 Claude Code、Codex 等程式設計智慧體幫你安裝：

```text
依照 https://raw.githubusercontent.com/demi-sh/demi/main/docs/delivery/install-with-an-agent.md 在我的伺服器上安裝 Demi
```

裝好後開啟 Demi，建立管理員帳號，在 **Settings › Providers** 中新增模型。支援的系統、HTTPS、Canary 頻道和各方案的費用請見[安裝指南](docs/delivery/installation.md)。

<p align="center">
  <img src="docs/assets/concept.webp" alt="從電腦或手機連到同一台 Demi 伺服器，所有對話都儲存在這裡；它透過筆電、桌機、HomeLab、VPS 上的執行器或雲端沙箱完成工作。" width="800">
</p>

## 主要特色

### 聊天、辦事、寫程式

- **一個對話搞定所有事**：簡單問題和多步驟任務都在這裡。
- **寫程式**：修改程式碼儲存庫，執行建置和測試，每處變更都能以 diff 檢視。
- **研究**：搜尋網路，閱讀網頁、PDF 和各類文件。
- **真實瀏覽器**：Demi 會操作 Chrome，你可以即時觀看，隨時接手。
- **手機應用程式**：在 iOS 模擬器和 Android 模擬器上安裝、測試應用程式。
- **檔案與資料**：處理文件、試算表和圖片，結果以附件傳回。
- **圖片、影片與音訊**：透過你新增的模型服務生成。
- **留言與提問**：在回覆裡選取文字，就能就此提問。
- **分享與搜尋**：分享唯讀連結，搜尋任意一則訊息。

### 隨時隨地，任何機器

- **瀏覽器即可使用**：電腦、平板、手機、摺疊機都行，不需安裝。
- **背景執行**：關掉網頁後任務繼續執行，完成時通知你。
- **對話集中管理**：在筆電上開始，在手機上繼續。
- **專屬雲端沙箱**：每位使用者都有獨立的 Linux 沙箱。
- **連接你自己的電腦**：一行指令連接 macOS、Windows 和 Linux。
- **多機協作**：在 Mac 上建置、在 Windows 上測試，任務中途也能切換機器。
- **每台機器的檔案**：瀏覽、預覽和搜尋每台機器上的檔案。
- **連接埠轉送**：從一台機器存取另一台機器的連接埠。
- **登入方式隨你選**：掃碼或通行密鑰，支援九種語言。

### 智慧體能力

- **多智慧體協作**：一組智慧體拆分大型任務，並行工作。
- **智慧體設定檔與工作流程**：各司其職的專家智慧體，可隨儲存庫共享。
- **排程任務**：按時間、cron 或在出現變化時喚醒對話。
- **對話協作**：一個對話可以傳訊息給另一個對話，或讀取它的紀錄。
- **技能**：教 Demi 學會新任務。
- **安全的憑證管理**：金鑰透過表單索取，不會出現在對話中。

### 模型與團隊

- **使用現有訂閱**：Claude、ChatGPT 和 Grok Build，額度用完自動輪換帳號。
- **任何 API**：Anthropic、OpenAI、Gemini，Kimi、GLM、MiniMax 等 Coding Plan，以及本機模型。
- **隨時切換模型**，任務進行中也可以。
- **用量與費用**：查看每個訂閱的額度和任務花費。
- **多使用者**：每位使用者都有獨立的沙箱、裝置和對話。

## 建立在 Shell 上的智慧體

Demi 不是 Claude Code 或 Codex 的外殼，而是由自己的執行環境驅動所有模型。大多數智慧體會給模型一長串工具，Demi 只給一個：Shell。瀏覽器、其他機器、子智慧體都是這個 Shell 裡的 `demi` 指令：

```sh
T=$(demi browser tab new) && demi browser open $T http://localhost:5173 && demi browser screenshot $T | demi file view
demi host shell --host windows 'cargo test'
demi agent spawn --profile research --description "Compare pricing pages" < brief.md
```

- **可組合**：指令可以用管線串接，一步完成通常需要多次工具呼叫的事。
- **精簡的提示詞**：模型只在需要時才讀指令的 `--help`。
- **處處相同的 Shell**：每台機器都執行同一個以 Rust 撰寫、相容 bash 的 Shell。
- **所有模型一致**：換模型不會改變智慧體能做的事。
- **為長任務而生**：指令在背景執行，完成後回報。

> [!NOTE]
> **規劃中**：為 Demi 伺服器提供 OpenAPI，讓你自己的程式也能使用它的對話、智慧體和雲端沙箱。

## 架構

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

- **伺服器執行智慧體，執行器負責做事**：每個執行器主動向外連線，因此在 NAT 和防火牆後也能運作。
- **處處同一個執行器**：每項功能在每台機器上都能用。
- **節省頻寬**：大型檔案留在產生它的機器上，開啟時才載入。
- **原生**：伺服器、執行器和工具以 Rust 撰寫，網頁應用程式以 Vue 和 TypeScript 撰寫。

設計文件見 [docs/](docs/README.md)，建議從[概覽](docs/overview.md)開始閱讀（英文）。

## 比較

Demi 與 OpenHands、Orca、T3 Code、DeepSeek Harness、opencode、Pi、herdr 的逐項比較，請見 [Demi 比較](docs/comparison.zh-TW.md)。

## 文件

- [設計文件](docs/README.md)：Demi 各部分的運作原理
- [參與貢獻](CONTRIBUTING.md)：開發環境、檢查項目，以及如何新增模型供應商
- [安全性](SECURITY.md)：漏洞回報方式與資料儲存說明

以上文件均為英文。

## 授權條款

Demi 以 [Apache License 2.0](LICENSE) 發布。

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/brand/lockup-dark.svg">
    <img alt="Demi" src="docs/assets/brand/lockup-light.svg" height="72">
  </picture>
</p>

<p align="center">
  <strong>내 모든 컴퓨터에서 일하는 나의 에이전트.<br>
  내 서버에서 호스팅하는 웹 기반 AI 에이전트.</strong>
</p>

<p align="center">
  <a href="#라이선스"><img alt="License" src="https://img.shields.io/badge/license-Apache%202.0-blue"></a>
  <a href="https://github.com/demi-sh/demi/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/demi-sh/demi"></a>
  <a href="https://github.com/demi-sh/demi/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/demi-sh/demi/actions/workflows/ci.yml/badge.svg"></a>
</p>

<p align="center">
  <a href="README.md">English</a> · <a href="README.de.md">Deutsch</a> · <a href="README.es.md">Español</a> · <a href="README.fr.md">Français</a> · <a href="README.pt-BR.md">Português</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="README.zh-TW.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <b>한국어</b>
</p>

<p align="center">
  <img src="docs/assets/product.webp" alt="브라우저의 Demi: 에이전트가 iPhone 앱을 2열 레이아웃으로 만들고, 답변에는 댓글과 곁질문이, 아래에는 추천 답장이 있습니다. 사이드 패널에는 반쯤 접힌 iPhone Duo에서 실행 중인 앱이, 휴대폰에는 같은 대화가 보입니다." width="800">
</p>

Demi는 내 서버에서 호스팅하는 웹 기반 AI 에이전트입니다. 어느 브라우저에서나 열 수 있고, 자료 조사, 웹사이트, 파일, 코드까지 무엇이든 맡길 수 있습니다. 대화는 내 서버에 남고, 작업은 클라우드 샌드박스나 내 컴퓨터에서 실행되며 탭을 닫아도 계속됩니다.

> [!NOTE]
> Demi는 초기 개발 단계입니다. 릴리스 사이에 호환되지 않는 변경이 있을 수 있습니다.

## 빠른 시작

몇 번의 클릭으로 Demi 배포:

<p>
  <a href="docs/delivery/railway.md"><img alt="Deploy on Railway" src="https://railway.com/button.svg" height="36"></a>
  <a href="https://render.com/deploy?repo=https://github.com/demi-sh/demi"><img alt="Deploy on Render" src="docs/assets/deploy/deploy-on-render.svg" height="36"></a>
  <a href="docs/delivery/fly.md"><img alt="Deploy on Fly.io" src="docs/assets/deploy/deploy-on-fly.svg" height="36"></a>
  <a href="docs/delivery/docker-compose.md"><img alt="Run with Docker Compose" src="docs/assets/deploy/run-with-docker-compose.svg" height="36"></a>
</p>

또는 도메인 이름이 있는 내 Linux 서버에 설치:

```sh
curl -fsSL https://demi.sh/install.sh | sudo bash
```

또는 Claude Code나 Codex 같은 코딩 에이전트에게 설치를 맡기세요:

```text
https://raw.githubusercontent.com/demi-sh/demi/main/docs/delivery/install-with-an-agent.md 을 따라 내 서버에 Demi를 설치해 줘
```

설치가 끝나면 Demi를 열어 관리자 계정을 만들고, **Settings › Providers**에서 모델을 추가합니다. 지원 시스템, HTTPS, Canary 채널, 방법별 비용은 [설치 가이드](docs/delivery/installation.md)를 참고하세요.

<p align="center">
  <img src="docs/assets/concept.webp" alt="컴퓨터나 휴대폰에서 하나의 Demi 서버에 접속합니다. 서버는 모든 대화를 보관하고, 노트북, 데스크톱, 홈랩, VPS의 러너나 클라우드 샌드박스를 통해 작업합니다." width="800">
</p>

## 주요 기능

### 채팅, 업무, 코딩

- **하나의 대화로 모든 일을**: 간단한 질문도, 여러 단계의 작업도.
- **코딩**: 저장소를 수정하고 빌드와 테스트를 실행하며, 모든 변경을 diff로 확인.
- **리서치**: 웹을 검색하고 웹 페이지, PDF, 문서를 읽습니다.
- **실제 브라우저**: Demi가 Chrome을 조작합니다. 지켜보다가 언제든 이어받으세요.
- **휴대폰 앱**: iOS 시뮬레이터와 Android 에뮬레이터에서 앱을 설치하고 테스트.
- **파일과 데이터**: 문서, 스프레드시트, 이미지를 다루고 첨부 파일로 돌려줍니다.
- **이미지, 동영상, 오디오**: 추가한 모델 공급자로 만듭니다.
- **댓글과 질문**: 답변에서 글자를 선택해 질문합니다.
- **공유와 검색**: 읽기 전용 링크로 공유하고 모든 메시지를 검색.

### 어디서든, 어느 컴퓨터에서든

- **어느 브라우저에서나**: 컴퓨터, 태블릿, 휴대폰, 폴더블에서 설치 없이.
- **항상 실행**: 탭을 닫아도 작업이 계속되고, 끝나면 알려 줍니다.
- **대화는 한곳에**: 노트북에서 시작하고 휴대폰에서 이어 갑니다.
- **전용 클라우드 샌드박스**: 사용자마다 전용 Linux 샌드박스.
- **내 컴퓨터 연결**: macOS, Windows, Linux를 명령 하나로 연결.
- **여러 컴퓨터, 하나의 작업**: Mac에서 빌드하고 Windows에서 테스트하며, 작업 중에 옮길 수도 있습니다.
- **모든 컴퓨터의 파일**: 각 컴퓨터의 파일을 둘러보고 미리 보고 검색.
- **포트 포워딩**: 한 컴퓨터에서 다른 컴퓨터의 포트에 접속.
- **원하는 방식으로 로그인**: QR 코드나 패스키, 9개 언어 지원.

### 에이전트

- **멀티 에이전트**: 에이전트 팀이 큰 작업을 나눠 병렬로 처리합니다.
- **프로필과 워크플로**: 전문 에이전트를 저장소로 공유합니다.
- **일정**: 타이머, cron, 변화에 맞춰 대화를 깨웁니다.
- **대화끼리 협업**: 한 대화가 다른 대화에 메시지를 보내거나 읽습니다.
- **스킬**: Demi에게 새로운 일을 가르칩니다.
- **안전한 자격 증명**: 비밀 값은 양식으로 받고, 대화에는 나타나지 않습니다.

### 모델과 팀

- **내 구독**: Claude, ChatGPT, Grok Build. 한도에 이르면 계정을 바꿔 가며 씁니다.
- **어떤 API든**: Anthropic, OpenAI, Gemini, Kimi·GLM·MiniMax 같은 코딩 플랜, 로컬 모델.
- **언제든 모델 전환**, 작업 도중에도.
- **사용량과 비용**: 구독별 한도와 작업 비용을 확인.
- **다중 사용자**: 사용자마다 전용 샌드박스, 기기, 대화.

## 셸 위에 만든 에이전트

Demi는 Claude Code나 Codex를 감싼 도구가 아니라, 자체 런타임이 모든 모델을 구동합니다. 대부분의 에이전트는 모델에 긴 도구 목록을 주지만, Demi는 셸 하나만 줍니다. 브라우저, 다른 컴퓨터, 하위 에이전트는 그 안의 `demi` 명령입니다:

```sh
T=$(demi browser tab new) && demi browser open $T http://localhost:5173 && demi browser screenshot $T | demi file view
demi device shell --device windows 'cargo test'
demi agent spawn --profile research --description "Compare pricing pages" < brief.md
```

- **조합 가능**: 명령을 파이프로 이어, 한 단계로 여러 번의 도구 호출이 할 일을 합니다.
- **가벼운 프롬프트**: 모델은 필요할 때만 명령의 `--help`를 읽습니다.
- **어디서나 같은 셸**: Rust로 작성한 bash 호환 셸이 모든 컴퓨터에서 실행됩니다.
- **모든 모델에 동일**: 모델을 바꿔도 에이전트가 할 수 있는 일은 그대로입니다.
- **긴 작업을 위해**: 명령은 백그라운드에서 실행되고 끝나면 보고합니다.

> [!NOTE]
> **예정**: Demi 서버용 OpenAPI. 내 프로그램에서 대화, 에이전트, 클라우드 샌드박스를 쓸 수 있게 합니다.

## 아키텍처

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

- **서버는 에이전트를 실행하고, 러너는 작업을 합니다**: 각 러너가 밖으로 연결하므로 NAT와 방화벽 뒤에서도 동작합니다.
- **어디서나 같은 러너**: 모든 기능이 모든 컴퓨터에서 동작합니다.
- **대역폭 절약**: 큰 파일은 만들어진 컴퓨터에 남아 있다가 열 때만 불러옵니다.
- **네이티브**: 서버, 러너, 도구는 Rust, 웹 앱은 Vue와 TypeScript.

[설계 문서](docs/README.md)는 [개요](docs/overview.md)부터 읽어 보세요(영문).

## 비교

OpenHands, Orca, T3 Code, DeepSeek Harness, opencode, Pi, herdr와 Demi의 기능별 비교는 [Demi 비교](docs/comparison.ko.md)를 참고하세요.

## 문서

- [설계 문서](docs/README.md): Demi의 각 부분이 동작하는 방식
- [기여하기](CONTRIBUTING.md): 개발 환경, 검사, 모델 제공자 추가 방법
- [보안](SECURITY.md): 취약점 신고 방법과 데이터 저장 위치

이 문서들은 영문으로 작성되어 있습니다.

## 라이선스

Demi는 [Apache License 2.0](LICENSE)으로 공개됩니다.

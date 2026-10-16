# Demi 비교

[English](comparison.md) · [简体中文](comparison.zh-CN.md) · [繁體中文](comparison.zh-TW.md) · [日本語](comparison.ja.md) · **한국어**

같은 종류의 에이전트 앱과 Demi의 차이를 2026년 10월 기준, 각 제품의 문서와 소스 코드를 바탕으로 정리했습니다. 제품 간에 차이가 있는 항목만 실었습니다.

✓ 지원 · ◐ 일부 지원 · ✗ 지원 안 함 · **Web**: 브라우저에서 사용하며 전용 앱은 없음 · **PWA**: 브라우저에서 앱으로 설치

| | Demi | [Pi](https://github.com/earendil-works/pi) | [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) | [OpenHands](https://github.com/OpenHands/OpenHands) | [opencode](https://github.com/anomalyco/opencode) | [Orca](https://github.com/stablyai/orca) | [herdr](https://github.com/herdrdev/herdr) | [T3 Code](https://github.com/pingdotgg/t3code) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **모델과 구독** | | | | | | | | |
| Claude 구독 | ✓ | ✓ | ◐¹ | ✓ | ✗ | ✓ | ✓ | ✓ |
| ChatGPT, Grok 등 구독 | ✓ | ✓ | ◐¹ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 여러 계정 | ✓ | ✗ | ✗ | ✗ | ✓ | ✓ | ✗ | ✓ |
| 계정 순환 | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| 작업 중 모델 전환 | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ✗ | ✓ |
| **인터페이스** | | | | | | | | |
| 웹 앱 | ✓ | ✗ | ✓ | ✓ | ✓ | ✓ | ✗ | ✓ |
| 모바일 | PWA | ✗ | Web | Web | Web | ✓ | ✗ | ✓ |
| 데스크톱 앱 | PWA | ✗ | ✓ | ✓ | ✓ | ✓ | ✗ | ✓ |
| 터미널 UI | ✗ | ✓ | ✗ | ✓ | ✓ | ✗ | ✓ | ✗ |
| **실행** | | | | | | | | |
| 중앙 서버 | ✓ | ✗ | ◐ | ◐ | ◐ | ◐ | ✗ | ✗ |
| 모든 기기가 꺼져도 실행 | ✓ | ✗ | ◐ | ✓² | ◐ | ◐ | ◐ | ◐ |
| 대화 동기화 | ✓ | ✗ | ✗ | ◐ | ◐ | ◐ | ✗ | ◐ |
| 클라우드 샌드박스 | ✓ | ✗ | ✗ | ✓ | ✗ | ◐ | ✗ | ✗ |
| 원격 컴퓨터 | ✓ | ◐ | ◐ | ◐ | ◐ | ✓ | ✓ | ✓ |
| 여러 컴퓨터를 쓰는 작업 | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| 컴퓨터 전환 | ✓ | ◐ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| **에이전트** | | | | | | | | |
| 자체 에이전트 | ✓ | ✓ | ✓ | ✓ | ✓ | ✗ | ✗ | ✗ |
| 셸 하나만 도구로 | ✓ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ |
| 멀티 에이전트 | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 에이전트 프로필 | ✓ | ◐ | ✓ | ✓ | ✓ | ✗ | ✗ | ✗ |
| Skills | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ✓ |
| MCP | ✗ | ✓ | ✓ | ✓ | ✓ | ◐ | ✗ | ✓ |
| 브라우저 자동화 | ✓ | ✗ | ◐ | ✓ | ✓ | ✓ | ✗ | ✓ |
| 컴퓨터 사용 | ✗ | ✗ | ◐³ | ✗ | ✗ | ✓ | ✗ | ◐⁴ |
| **팀** | | | | | | | | |
| 다중 사용자 | ✓ | ✗ | ✗ | ◐² | ✗ | ✗ | ✗ | ✗ |
| 관리 콘솔 | ✓ | ✗ | ✗ | ◐² | ◐² | ✗ | ✗ | ✗ |
| 사용자별 격리 | ✓ | ✗ | ✗ | ◐² | ✗ | ✗ | ✗ | ✗ |

1. 직접 실행하는 Claude Code와 Codex 하위 에이전트를 통해서만 가능합니다. 자체 구독 로그인은 코드에 있지만 앱에 진입점이 없습니다.
2. 유료 클라우드 버전이나 엔터프라이즈 버전에서만 가능합니다.
3. 실험 기능이며 직접 켜야 합니다.
4. iOS와 Android 시뮬레이터만 조작하며, 데스크톱은 조작하지 못합니다.

## 각 항목의 의미

- **구독**: API 키 대신 구독 계정으로 로그인합니다. 제품이 실행하는 Claude Code나 Codex 같은 에이전트를 통해 쓰는 구독도 지원으로 봅니다.
- **여러 계정**, **계정 순환**: 제품이 직접 하는 경우만 해당하며, 제품이 실행하는 에이전트의 기능은 포함하지 않습니다. 순환은 한 계정이 한도에 이르면 다른 계정으로 작업을 이어 가는 것입니다.
- **중앙 서버**: 서버 한 대가 에이전트와 모든 대화를 가지고, 내 컴퓨터들은 그 명령을 실행하기만 합니다. ◐: 서버는 있지만 자기 컴퓨터에서만 실행합니다. ✗: 컴퓨터마다 에이전트가 따로 돌거나 서버가 없습니다.
- **모든 기기가 꺼져도 실행**: 컴퓨터와 휴대폰을 모두 꺼도 작업이 계속됩니다. ◐: 늘 켜져 있는 컴퓨터를 직접 마련해 설치한 경우에만 됩니다.
- **대화 동기화**: 어느 컴퓨터에서 실행한 대화든 모두 한 목록에 있고, 어느 기기에서나 똑같이 보입니다. ◐: 같은 서버에 연결한 기기끼리만 동기화됩니다.
- **여러 컴퓨터를 쓰는 작업**: 대화 하나가 여러 컴퓨터에서 작업합니다. **컴퓨터 전환**: 대화를 다른 컴퓨터로 옮겨 이어 갑니다.
- **자체 에이전트**: Claude Code나 Codex 같은 에이전트를 실행하는 대신 제품 자체의 에이전트가 모델을 구동합니다.
- **셸 하나만 도구로**: 모델이 받는 도구는 셸 하나이고, 모든 기능은 그 안의 명령입니다.
- **컴퓨터 사용**: 에이전트가 스크린샷, 마우스, 키보드로 데스크톱을 조작합니다. 브라우저에 한정되지 않습니다.

<details>
<summary>출처</summary>

- Pi: [README](https://github.com/earendil-works/pi), [Providers](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md), [OAuth sign-ins](https://github.com/earendil-works/pi/tree/main/packages/ai/src/auth/oauth), [Credential store](https://github.com/earendil-works/pi/blob/main/packages/ai/src/auth/credential-store.ts), [SSH extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/ssh.ts), [Subagent extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/subagent/README.md), [Containerization](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/containerization.md), [Termux](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/termux.md), [Slash commands](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/slash-commands.md)
- DeepSeek Harness: [README](https://github.com/deepseek-ai/deepseek-harness), [Providers](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/providers.md), [Claude Code subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-claude-code/README.md), [Codex subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-codex/README.md), [Sign-in flows](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/llm/llm-pi-ai/src/login.ts), [Public deployments](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/public-deployments.md), [SSH](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/ssh.md), [Persistence](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/persistence.md), [Tool catalog](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/tool-catalog.md), [Agent presets](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/preset/agent-preset/README.md), [Browser use](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/browser-use.md), [Computer use](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/computer-use/README.md), [Desktop app](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/desktop/README.md)
- OpenHands: [README](https://github.com/OpenHands/OpenHands), [ACP agents](https://docs.openhands.dev/openhands/usage/agent-canvas/acp-agents), [Subscription sign-in](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/openai.py), [Credentials](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/credentials.py), [Backends](https://docs.openhands.dev/openhands/usage/agent-canvas/backends), [Remote backend](https://docs.openhands.dev/openhands/usage/agent-canvas/backend-setup/remote), [Phone and tablet access](https://docs.openhands.dev/openhands/usage/agent-canvas/mobile-access), [Cloud UI](https://docs.openhands.dev/openhands/usage/cloud/cloud-ui), [Enterprise vs. open source](https://docs.openhands.dev/enterprise/enterprise-vs-oss), [Tools](https://github.com/OpenHands/software-agent-sdk/tree/main/openhands-tools/openhands/tools)
- opencode: [Providers](https://opencode.ai/docs/providers/), [Provider plugins](https://github.com/anomalyco/opencode/tree/dev/packages/core/src/plugin/provider), [Web](https://opencode.ai/docs/web/), [Server](https://opencode.ai/docs/server/), [TUI](https://opencode.ai/docs/tui/), [Agents](https://opencode.ai/docs/agents/), [MCP servers](https://opencode.ai/docs/mcp-servers/), [Browser plugin](https://github.com/anomalyco/opencode/blob/dev/packages/plugin-browser/README.md), [Enterprise](https://opencode.ai/docs/enterprise/)
- Orca: [README](https://github.com/stablyai/orca), [Supported agents](https://www.onorca.dev/docs/agents/supported), [Accounts](https://www.onorca.dev/docs/agents/codex-hot-swap), [Ways to run](https://www.onorca.dev/docs/ways-to-run), [Remote servers](https://www.onorca.dev/docs/remote-servers), [Mobile](https://www.onorca.dev/docs/mobile), [Computer use](https://www.onorca.dev/docs/cli/computer-use), [Browser](https://www.onorca.dev/docs/browser/overview)
- herdr: [README](https://github.com/herdrdev/herdr), [Agents](https://herdr.dev/docs/agents/), [Connecting machines](https://herdr.dev/docs/connecting-machines/), [Persistence and remote](https://herdr.dev/docs/persistence-remote/), [Work from your phone](https://herdr.dev/docs/how-to-work/#work-from-your-phone), [Agent skill](https://herdr.dev/docs/agent-skill/)
- T3 Code: [README](https://github.com/pingdotgg/t3code), [Claude](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-claude.md), [Codex](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-codex.md), [Remote access](https://github.com/pingdotgg/t3code/blob/main/docs/user/remote-access.md), [Background service](https://github.com/pingdotgg/t3code/blob/main/docs/user/background-service.md), [Devices](https://github.com/pingdotgg/t3code/blob/main/docs/user/devices.md), [Outside agents](https://github.com/pingdotgg/t3code/blob/main/docs/user/outside-agents.md)

</details>

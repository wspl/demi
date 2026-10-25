# Demi 비교

[English](comparison.md) · [简体中文](comparison.zh-CN.md) · [繁體中文](comparison.zh-TW.md) · [日本語](comparison.ja.md) · **한국어**

같은 종류의 에이전트 앱과 Demi의 차이를 2026년 10월 기준, 각 제품의 문서와 소스 코드를 바탕으로 정리했습니다. 제품 간에 차이가 있는 항목과, 아직 어느 제품도 지원하지 않는 기능 하나를 실었습니다. 열은 Demi와 가까운 제품부터 먼 제품 순으로 놓았습니다.

표에서는 다음 기호와 라벨을 사용합니다:

- ✓ 지원
- ◐ 일부 지원
- 빈칸: 지원 안 함
- **Web**: 브라우저에서 사용하며 전용 앱은 없음
- **PWA**: 브라우저에서 앱으로 설치

다음 표는 제품을 항목별로 비교합니다:

| | Demi | [OpenHands][openhands] | [Orca][orca] | [T3 Code][t3-code] | [DeepSeek Harness][deepseek-harness] | [opencode][opencode] | [Pi][pi] | [herdr][herdr] |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **모델과 구독** | | | | | | | | |
| Claude 구독 | ✓ | ✓ | ✓ | ✓ | ◐¹ | | ✓ | ✓ |
| ChatGPT, Grok 등 구독 | ✓ | ✓ | ✓ | ✓ | ◐¹ | ✓ | ✓ | ✓ |
| 여러 계정 | ✓ | | ✓ | ✓ | | ✓ | | |
| 계정 순환 | ✓ | | | | | | | |
| 구독 사용량과 초기화 시각 | ✓ | ◐² | ✓ | ✓ | ◐³ | | | |
| 작업 중 모델 전환 | ✓ | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | |
| **인터페이스** | | | | | | | | |
| 웹 앱 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | | |
| 모바일 | PWA | Web | ✓ | ✓ | Web | Web | | |
| 데스크톱 앱 | PWA | ✓ | ✓ | ✓ | ✓ | ✓ | | |
| 터미널 UI | | ✓ | | | | ✓ | ✓ | ✓ |
| 음성 입력 | | ✓ | ✓ | ✓ | ◐⁴ | | | |
| 휴대폰 알림 | ✓ | | ✓ | ✓⁵ | | | | ◐⁶ |
| 대화 검색 | ✓ | | ◐⁷ | ✓ | ◐⁸ | ◐⁸ | ✓ | |
| **실행** | | | | | | | | |
| 중앙 서버 | ✓ | ◐ | ◐ | | ◐ | ◐ | | |
| 모든 기기가 꺼져도 실행 | ✓ | ✓⁹ | ◐ | ◐ | ◐ | ◐ | | ◐ |
| 대화 동기화 | ✓ | ◐ | ◐ | ◐ | | ◐ | | |
| 클라우드 샌드박스 | ✓ | ✓ | ◐ | | | | | |
| 원격 컴퓨터 | ✓ | ◐ | ✓ | ✓ | ◐ | ◐ | ◐ | ✓ |
| 여러 컴퓨터를 쓰는 작업 | ✓ | | | | | | | |
| 컴퓨터 전환 | ✓ | | | | | | ◐ | |
| 포트 포워딩 | ✓ | | ✓¹⁰ | | | | | |
| 예약 작업 | ✓ | ✓ | ✓ | ✓ | ✓ | ◐¹¹ | | |
| **파일** | | | | | | | | |
| 파일 브라우저 | ✓ | ✓ | ✓ | ✓ | ◐¹² | ✓ | | |
| 파일 관리 | ◐¹³ | ◐¹⁴ | ✓ | ◐¹⁵ | | | | |
| 파일 내용 검색 | ✓ | | ✓ | ✓ | | | | |
| Git worktree | ◐¹⁶ | ✓ | ✓ | ✓ | ◐⁴ | ✓ | | ✓ |
| diff 줄 단위 댓글 | | | ✓ | ◐¹⁷ | | ✓ | | |
| **에이전트** | | | | | | | | |
| 자체 에이전트 | ✓ | ✓ | | | ✓ | ✓ | ✓ | |
| 셸 하나만 도구로 | ✓ | | | | | | | |
| 멀티 에이전트 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ✓ |
| 에이전트 프로필 | ✓ | ✓ | | | ✓ | ✓ | ◐ | |
| Skills | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ |
| MCP | | ✓ | ◐ | ✓ | ✓ | ✓ | ✓ | |
| 서드파티 플러그인 | | ✓ | ◐⁴ | | ✓ | ✓ | ✓ | ✓ |
| 에이전트용 시크릿 | ✓ | ✓ | | | | | | |
| 브라우저 자동화 | ✓ | ✓ | ✓ | ✓ | ◐ | ◐¹⁸ | | |
| 브라우저 실시간 보기와 조작 | ✓ | ◐¹⁹ | ✓ | ✓ | | ◐¹⁸ | | |
| 컴퓨터 사용 | | | ◐¹⁸ | ◐²⁰ | ◐⁴ | | | |
| **모바일 기기** | | | | | | | | |
| iOS 시뮬레이터 | ✓ | | ◐¹⁸ | ✓ | | | | |
| iOS 실기기 | | | | | | | | |
| Android 에뮬레이터 | ✓ | | ◐¹⁸ | ✓ | | | | |
| Android 실기기 | | | ◐¹⁸ | ✓ | | | | |
| **미디어 생성** | | | | | | | | |
| 이미지 생성과 편집 | ✓ | ◐²¹ | ◐²¹ | ◐²¹ | ◐²¹ | | ◐²² | ◐²¹ |
| 동영상 생성 | ✓ | | | | | | | |
| 음성, 음악, 효과음 | ✓ | | | | | | | |
| **팀** | | | | | | | | |
| 다중 사용자 | ✓ | ◐⁹ | | | | | | |
| 관리 콘솔 | ✓ | ◐⁹ | | | | ◐⁹ | | |
| 사용자별 격리 | ✓ | ◐⁹ | | | | | | |
| 대화 공유 링크 | ✓ | ◐²³ | | ◐²⁴ | ◐⁴ | | ✓ | |

표의 번호 주석은 다음과 같습니다:

1. 직접 실행하는 Claude Code와 Codex 하위 에이전트를 통해서만 가능합니다. 자체 구독 로그인은 코드에 있지만 앱에 진입점이 없습니다.
2. API 가격으로 추정한 요금제 사용량입니다.
3. DeepSeek 자체 계정의 한도만 보여 줍니다.
4. 실험 기능이며 직접 켜야 합니다.
5. T3 Connect를 통해서만 가능합니다.
6. Telegram으로 보내는 예제 플러그인만 있습니다.
7. 제목, 폴더, 브랜치, 모델, 미리보기로 검색하며 전체 내용은 검색하지 않습니다.
8. 제목으로만 검색합니다.
9. 유료 클라우드 버전이나 엔터프라이즈 버전에서만 가능합니다.
10. SSH 호스트에서 이 컴퓨터로만 전달하며, 호스트에서 대기 중인 포트를 자동으로 찾아 줍니다.
11. GitHub Actions의 cron 작업으로만 가능합니다.
12. 자신이 실행되는 컴퓨터의 파일만 보이며, 웹 화면에서는 SSH 호스트의 파일을 볼 수 없습니다.
13. 업로드, 다운로드, 새 폴더만 되며 이름 바꾸기, 삭제, 이동, 편집은 안 됩니다.
14. 작업 공간 전체 다운로드만 가능합니다.
15. 파일 내용 편집만 가능합니다.
16. 기존 worktree를 프로젝트 아래에 보여 줄 뿐 새로 만들지는 못합니다.
17. 아무 텍스트나 인용해 댓글을 다는 기능이며, diff의 줄이 대상은 아닙니다.
18. 데스크톱 앱을 실행하는 컴퓨터에서만 쓸 수 있으며, 웹 클라이언트나 원격 컴퓨터에서는 쓸 수 없습니다.
19. 에이전트의 마지막 스크린샷만 보여 주며 조작할 수 없습니다.
20. iOS와 Android 시뮬레이터만 조작하며, 데스크톱은 조작하지 못합니다.
21. 실행하는 Codex의 이미지 도구를 통해서만 가능합니다.
22. codemode 스크립트에서만 가능하며, codemode는 직접 켜야 합니다.
23. OpenHands Cloud 백엔드를 쓸 때만 가능합니다.
24. 게스트 링크이며, 베타 기능이라 T3 Connect가 필요합니다.

[openhands]: https://github.com/OpenHands/OpenHands
[orca]: https://github.com/stablyai/orca
[t3-code]: https://github.com/pingdotgg/t3code
[deepseek-harness]: https://github.com/deepseek-ai/deepseek-harness
[opencode]: https://github.com/anomalyco/opencode
[pi]: https://github.com/earendil-works/pi
[herdr]: https://github.com/herdrdev/herdr

## 각 항목의 의미

각 항목의 의미는 다음과 같습니다:

- **컴퓨터와 관련된 항목**(파일, worktree, diff, 브라우저, 컴퓨터 사용, 모바일 기기): 에이전트가 SSH 호스트, 다른 컴퓨터, 클라우드 컴퓨터 같은 원격 컴퓨터에서 작업할 때도 쓸 수 있어야 ✓로 봅니다. 제품을 실행하는 컴퓨터에서만 쓸 수 있으면 ◐입니다.
- **구독**: API 키 대신 구독 계정으로 로그인합니다. 제품이 실행하는 Claude Code나 Codex 같은 에이전트를 통해 쓰는 구독도 지원으로 봅니다.
- **여러 계정**, **계정 순환**: 제품이 직접 하는 경우만 해당하며, 제품이 실행하는 에이전트의 기능은 포함하지 않습니다. 순환은 한 계정이 한도에 이르면 다른 계정으로 작업을 이어 가는 것입니다.
- **구독 사용량과 초기화 시각**: 구독마다 한도를 얼마나 썼는지, 언제 초기화되는지 보여 줍니다.
- **휴대폰 알림**: 앱을 닫아 두어도 알림이 휴대폰에 옵니다.
- **대화 검색**: 모든 대화에서 어느 메시지의 단어든 찾습니다.
- **중앙 서버**: 서버 한 대가 에이전트와 모든 대화를 가지고, 내 컴퓨터들은 그 명령을 실행하기만 합니다.
  - ◐: 서버는 있지만 자기 컴퓨터에서만 실행합니다.
  - 빈칸: 컴퓨터마다 에이전트가 따로 돌거나 서버가 없습니다.
- **모든 기기가 꺼져도 실행**: 컴퓨터와 휴대폰을 모두 꺼도 작업이 계속됩니다.
  - ◐: 늘 켜져 있는 컴퓨터를 직접 마련해 설치한 경우에만 됩니다.
- **대화 동기화**: 어느 컴퓨터에서 실행한 대화든 모두 한 목록에 있고, 어느 기기에서나 똑같이 보입니다.
  - ◐: 같은 서버에 연결한 기기끼리만 동기화됩니다.
- **여러 컴퓨터를 쓰는 작업**: 대화 하나가 여러 컴퓨터에서 작업합니다.
- **컴퓨터 전환**: 대화를 다른 컴퓨터로 옮겨 이어 갑니다.
- **포트 포워딩**: 한 컴퓨터의 프로그램이 다른 컴퓨터의 포트에 접속합니다. `ssh -L`과 같습니다.
- **예약 작업**: 정해 둔 시각이나 간격에 작업이 저절로 시작됩니다.
- **파일 관리**: 앱에서 파일을 업로드, 다운로드, 생성, 이름 변경, 삭제, 이동, 편집합니다.
- **diff 줄 단위 댓글**: 변경 사항의 한 줄에 단 댓글이 에이전트에게 전달됩니다.
- **자체 에이전트**: Claude Code나 Codex 같은 에이전트를 실행하는 대신 제품 자체의 에이전트가 모델을 구동합니다.
- **셸 하나만 도구로**: 모델이 받는 도구는 셸 하나이고, 모든 기능은 그 안의 명령입니다.
- **서드파티 플러그인**: Skills나 MCP와 별개로, 다른 사람이 만든 코드를 설치해 제품에 도구나 화면을 더합니다. Demi의 기능 자체는 플러그인으로 되어 있지만 다른 사람의 플러그인은 불러오지 않습니다.
- **에이전트용 시크릿**: 사용자가 이름을 붙여 저장한 시크릿을 에이전트가 명령에서 씁니다.
- **브라우저 실시간 보기와 조작**: 에이전트가 쓰는 브라우저를 작업 중에 보고 직접 조작할 수 있습니다.
- **컴퓨터 사용**: 에이전트가 스크린샷, 마우스, 키보드로 데스크톱을 조작합니다. 브라우저에 한정되지 않습니다.
- **iOS 실기기**, **Android 실기기**: 시뮬레이터가 아니라 컴퓨터에 연결한 실제 휴대폰입니다.
- **대화 공유 링크**: 링크로 대화를 다른 사람에게 보여 줍니다.

<details>
<summary>출처</summary>

- OpenHands: [README](https://github.com/OpenHands/OpenHands), [ACP agents](https://docs.openhands.dev/openhands/usage/agent-canvas/acp-agents), [Subscription sign-in](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/openai.py), [Credentials](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/llm/auth/credentials.py), [Backends](https://docs.openhands.dev/openhands/usage/agent-canvas/backends), [Remote backend](https://docs.openhands.dev/openhands/usage/agent-canvas/backend-setup/remote), [Phone and tablet access](https://docs.openhands.dev/openhands/usage/agent-canvas/mobile-access), [Cloud UI](https://docs.openhands.dev/openhands/usage/cloud/cloud-ui), [Enterprise vs. open source](https://docs.openhands.dev/enterprise/enterprise-vs-oss), [Tools](https://github.com/OpenHands/software-agent-sdk/tree/main/openhands-tools/openhands/tools), [Automations](https://docs.openhands.dev/openhands/usage/agent-canvas/prebuilt-automations), [Secrets](https://github.com/OpenHands/OpenHands/blob/main/src/routes/secrets-settings.tsx), [Files](https://github.com/OpenHands/OpenHands/blob/main/src/routes/files-tab.tsx), [Workspace download](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/conversation-panel/conversation-card/conversation-card-context-menu.tsx), [Browser panel](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/browser/browser.tsx), [Public share](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/conversation/conversation-name-context-menu.tsx), [Dictation](https://github.com/OpenHands/OpenHands/blob/main/src/hooks/chat/use-dictation.ts), [Worktrees](https://github.com/OpenHands/OpenHands/blob/main/src/components/features/chat/workspace-mode-selector.tsx), [Usage](https://github.com/OpenHands/OpenHands/blob/main/src/routes/usage-tab.tsx), [Plugins](https://github.com/OpenHands/OpenHands/blob/main/src/routes/skills-plugins.tsx)
- Orca: [README](https://github.com/stablyai/orca), [Supported agents](https://www.onorca.dev/docs/agents/supported), [Accounts](https://www.onorca.dev/docs/agents/codex-hot-swap), [Ways to run](https://www.onorca.dev/docs/ways-to-run), [Remote servers](https://www.onorca.dev/docs/remote-servers), [Mobile](https://www.onorca.dev/docs/mobile), [Computer use](https://www.onorca.dev/docs/cli/computer-use), [Browser](https://www.onorca.dev/docs/browser/overview), [File explorer](https://www.onorca.dev/docs/editing/file-explorer), [Editor](https://www.onorca.dev/docs/editing/monaco), [Session history](https://www.onorca.dev/docs/agents/session-history), [Worktrees](https://www.onorca.dev/docs/model/worktrees), [Annotate AI diffs](https://www.onorca.dev/docs/review/annotate-ai-diff), [SSH and port forwarding](https://www.onorca.dev/docs/ssh), [Automations](https://www.onorca.dev/docs/cli/automations), [Usage tracking](https://www.onorca.dev/docs/agents/usage-tracking), [Settings and plugins](https://www.onorca.dev/docs/settings), [iOS emulator skill](https://github.com/stablyai/orca/blob/main/skills/orca-emulator/SKILL.md), [Android emulator skill](https://github.com/stablyai/orca/blob/main/skills/orca-emulator-android/SKILL.md), [Client creation policy](https://github.com/stablyai/orca/blob/main/src/renderer/src/lib/client-creation-action-policy.ts)
- T3 Code: [README](https://github.com/pingdotgg/t3code), [Claude](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-claude.md), [Codex](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-codex.md), [Remote access](https://github.com/pingdotgg/t3code/blob/main/docs/user/remote-access.md), [Background service](https://github.com/pingdotgg/t3code/blob/main/docs/user/background-service.md), [Devices](https://github.com/pingdotgg/t3code/blob/main/docs/user/devices.md), [Outside agents](https://github.com/pingdotgg/t3code/blob/main/docs/user/outside-agents.md), [Usage and limits](https://github.com/pingdotgg/t3code/blob/main/docs/user/usage.md), [Composer](https://github.com/pingdotgg/t3code/blob/main/docs/user/composer.md), [Mobile notifications](https://github.com/pingdotgg/t3code/blob/main/docs/user/mobile-notifications.md), [Scheduled tasks](https://github.com/pingdotgg/t3code/blob/main/docs/user/project-settings.md), [Guest links](https://github.com/pingdotgg/t3code/blob/main/docs/user/guest-links.md), [Thread sidebar](https://github.com/pingdotgg/t3code/blob/main/docs/user/thread-sidebar.md), [Thread search](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/orchestration-v2/ThreadSearch.ts), [File browser](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/files/FileBrowserPanel.tsx), [File saving](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/files/fileSaveCoordinator.ts), [Content search](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/search/ProjectContentSearchDialog.tsx)
- DeepSeek Harness: [README](https://github.com/deepseek-ai/deepseek-harness), [Providers](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/providers.md), [Claude Code subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-claude-code/README.md), [Codex subagent](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/subagent/subagent-codex/README.md), [Sign-in flows](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/llm/llm-pi-ai/src/login.ts), [Public deployments](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/public-deployments.md), [SSH](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/ssh.md), [Persistence](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/persistence.md), [Tool catalog](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/tool-catalog.md), [Agent presets](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/preset/agent-preset/README.md), [Browser use](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/browser-use.md), [Computer use](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/computer-use/README.md), [Desktop app](https://github.com/deepseek-ai/deepseek-harness/blob/master/apps/desktop/README.md), [Schedule](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/schedule.md), [Files sidebar](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-sidebar-files/README.md), [Workspace](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-workspace/README.md), [Plugin indexes](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/plugin-market.md), [Account quota](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-settings-account/src/client/AccountQuotaNotice.tsx), [Voice input](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/voice-input-bundle/README.md), [Liveshare](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/liveshare-protocol/README.md), [Worktree](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/experimental/worktree/README.md)
- opencode: [Providers](https://opencode.ai/docs/providers/), [Provider plugins](https://github.com/anomalyco/opencode/tree/dev/packages/core/src/plugin/provider), [Web](https://opencode.ai/docs/web/), [Server](https://opencode.ai/docs/server/), [TUI](https://opencode.ai/docs/tui/), [Agents](https://opencode.ai/docs/agents/), [MCP servers](https://opencode.ai/docs/mcp-servers/), [Browser plugin](https://github.com/anomalyco/opencode/blob/dev/packages/plugin-browser/README.md), [Enterprise](https://opencode.ai/docs/enterprise/), [GitHub](https://opencode.ai/docs/github/), [Plugins](https://opencode.ai/docs/plugins/), [Files](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/workspaces/files/model.tsx), [Workspaces](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/workspaces/submenu.tsx), [Session search](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/home/sessions/search.ts), [Review comments](https://github.com/anomalyco/opencode/blob/dev/packages/app/src/composer/comment-note.ts), [AI library](https://github.com/anomalyco/opencode/blob/dev/packages/ai/README.md)
- Pi: [README](https://github.com/earendil-works/pi), [Providers](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md), [OAuth sign-ins](https://github.com/earendil-works/pi/tree/main/packages/ai/src/auth/oauth), [Credential store](https://github.com/earendil-works/pi/blob/main/packages/ai/src/auth/credential-store.ts), [SSH extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/ssh.ts), [Subagent extension](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/examples/extensions/subagent/README.md), [Containerization](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/containerization.md), [Termux](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/termux.md), [Slash commands](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/slash-commands.md), [Extensions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md), [Packages](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/packages.md), [Codemode](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/codemode.md), [Sessions](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/sessions.md)
- herdr: [README](https://github.com/herdrdev/herdr), [Agents](https://herdr.dev/docs/agents/), [Connecting machines](https://herdr.dev/docs/connecting-machines/), [Persistence and remote](https://herdr.dev/docs/persistence-remote/), [Work from your phone](https://herdr.dev/docs/how-to-work/#work-from-your-phone), [Agent skill](https://herdr.dev/docs/agent-skill/), [Configuration](https://herdr.dev/docs/configuration/), [Plugins](https://herdr.dev/docs/plugins/)
- Codex: [Image generation skill](https://github.com/openai/codex/blob/main/codex-rs/skills/src/assets/samples/imagegen/SKILL.md)

</details>

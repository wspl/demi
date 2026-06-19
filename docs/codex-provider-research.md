# Codex Provider 调研记录

| | |
|---|---|
| 日期 | 2026-06-19 |
| 状态 | 调研完成，首版实现已落地 |
| 范围 | `codex` provider、官方 Codex 鉴权复用、Responses 传输、tool/thinking 映射 |

## 1. 目标

下一个 provider 是 `codex`。目标直接按最终态设计：尽量贴近官方 Codex 的 provider/auth/Responses 实现，同时参考 pi 的 TypeScript 适配经验。

约束：

- 复用 Codex 的已有鉴权材料，默认使用 `$CODEX_HOME/auth.json`，未设置 `CODEX_HOME` 时为 `~/.codex/auth.json`。
- 不实现 Demi 自己的登录流程；用户需要登录时应使用官方 Codex。
- 鉴权、请求、stream event、tool call、thinking、usage/cache 都必须是 provider 边界的一等能力，不能只验证 mock 自洽。
- 所有 secret 只在内存和请求头中使用，日志、错误、auth status、测试快照都必须脱敏。

## 2. 调研过程

已更新本地参考仓库：

- `/Users/plutonist/Projects/References/codex`：`git fetch --prune origin` 后快进到 `origin/main`。
- `/Users/plutonist/Projects/References/pi-mono`：`git fetch --prune origin` 后快进到 `origin/main`。

读取的官方 Codex 关键文件：

- `codex-rs/login/src/auth/storage.rs`
- `codex-rs/login/src/auth/manager.rs`
- `codex-rs/login/src/token_data.rs`
- `codex-rs/app-server-protocol/src/protocol/common.rs`
- `codex-rs/model-provider-info/src/lib.rs`
- `codex-rs/model-provider/src/auth.rs`
- `codex-rs/model-provider/src/bearer_auth_provider.rs`
- `codex-rs/codex-api/src/common.rs`
- `codex-rs/codex-api/src/endpoint/responses.rs`
- `codex-rs/codex-api/src/sse/responses.rs`
- `codex-rs/core/src/client.rs`

读取的 pi 关键文件：

- `packages/ai/src/providers/openai-codex-responses.ts`
- `packages/ai/src/providers/openai-responses-shared.ts`
- `packages/ai/src/utils/oauth/openai-codex.ts`
- `packages/coding-agent/src/core/auth-storage.ts`

本机 `~/.codex/auth.json` 只检查了结构，未记录 token 值。当前结构为：

```json
{
  "auth_mode": "string",
  "OPENAI_API_KEY": null,
  "tokens": {
    "id_token": "string",
    "access_token": "string",
    "refresh_token": "string",
    "account_id": "string"
  },
  "last_refresh": "string"
}
```

这对应官方 Codex 的 ChatGPT token auth 路径。

## 3. 官方 Codex 鉴权事实

官方 `$CODEX_HOME/auth.json` 结构是 `AuthDotJson`：

- `auth_mode`
- `OPENAI_API_KEY`
- `tokens`
- `last_refresh`
- `agent_identity`
- `personal_access_token`
- `bedrock_api_key`

官方 `AuthMode` 包含：

- `apiKey`
- `chatgpt`
- `chatgptAuthTokens`
- `agentIdentity`
- `personalAccessToken`
- `bedrockApiKey`

官方存储模式包含 file、keyring、auto、ephemeral。`auto` 会优先 keyring，再 fallback 到 file。用户明确要求复用 `~/.codex/auth.json`，所以 file backend 是必须支持的路径；但最终态设计不能把 auth 实现写死成 file-only，否则后续无法对齐官方 `AutoAuthStorage`。

ChatGPT token 数据：

- `tokens.access_token` 是发请求的 bearer token。
- `tokens.refresh_token` 用于刷新 access token。
- `tokens.account_id` 或 JWT claim 中的 `chatgpt_account_id` 用于 `ChatGPT-Account-ID`。
- `id_token` 会被官方解析出 email、plan、account、fedramp 等元数据。
- access token 的 JWT `exp` 用于判断是否临近过期。

官方 refresh 行为：

- refresh endpoint：`https://auth.openai.com/oauth/token`
- client id：`app_EMoamEEZ73f0CkXaXp7hrann`
- request body：`client_id`、`grant_type: refresh_token`、`refresh_token`
- access token 距过期 5 分钟内，或 `last_refresh` 超过 8 天，会主动 refresh。
- ChatGPT auth 遇到 401 时会 reload auth，再 refresh，并重试。

结论：refresh 不是登录流程，而是复用官方 auth 的必要维护动作。最终态 `codex` provider 应实现官方兼容 refresh，并在写回 auth store 时保留未知字段、使用文件锁/原子写/`0600` 权限，避免破坏 Codex 自己的 auth 文件。

## 4. Backend 路由与请求头

官方路由规则：

- `chatgpt` / `chatgptAuthTokens` / `agentIdentity` / `personalAccessToken` 走 Codex backend：`https://chatgpt.com/backend-api/codex`。
- `apiKey` 走 OpenAI Responses endpoint：`https://api.openai.com/v1`。
- `bedrockApiKey` 不属于 OpenAI Codex provider 路径。

官方 bearer auth headers：

- `Authorization: Bearer <token>`
- `ChatGPT-Account-ID: <account_id>`
- `X-OpenAI-Fedramp: true`，仅 fedramp account 需要。

官方 Responses session headers：

- `session-id`
- `thread-id`
- `x-client-request-id`
- `x-codex-installation-id`
- `x-codex-turn-state`
- `x-codex-turn-metadata`
- `x-codex-parent-thread-id`
- `x-codex-window-id`

pi 的 Codex provider 额外使用：

- `OpenAI-Beta: responses=experimental` 用于 SSE。
- WebSocket 使用 `OpenAI-Beta: responses_websockets=2026-02-06`。
- `originator: pi` 和 pi 自己的 `User-Agent`。

Demi 不能照抄 pi 的身份头。最终态应使用 Demi 自己的 user agent / originator，并保持 beta header 可配置，因为官方 Codex 当前 HTTP path 的 beta header 逻辑与 pi 不完全一致。

## 5. Responses 请求形状

官方 `ResponsesApiRequest` 核心字段：

- `model`
- `instructions`
- `input`
- `tools`
- `tool_choice: "auto"`
- `parallel_tool_calls`
- `reasoning`
- `store`
- `stream`
- `include`
- `service_tier`
- `prompt_cache_key`
- `text`
- `client_metadata`

Codex/ChatGPT backend 需要：

- `stream: true`
- `store: false`
- `include: ["reasoning.encrypted_content"]`
- `prompt_cache_key` 稳定绑定到 session/thread。
- thinking 打开时发送 `reasoning.effort` 和 `reasoning.summary`。
- tool 使用 Responses function tool 格式。

当前 Demi `InferenceRequest` 没有显式 `sessionId` / `threadId` / `turnId`。这对 Codex provider 是架构缺口：官方 Codex 和 pi 都依赖稳定 session id 做 request id、prompt cache key、WebSocket continuation 和 cache 亲和。最终态应把稳定 agent session identity 传给 provider，而不是让 provider 自己猜测或生成不可恢复的随机 id。

## 6. Stream 事件映射

官方 SSE parser 将 Responses event 映射为：

- `response.created` → created
- `response.output_item.added` → output item start
- `response.output_text.delta` → text delta
- `response.reasoning_summary_text.delta` → reasoning summary delta
- `response.reasoning_text.delta` → reasoning content delta
- `response.function_call_arguments.delta` / `done` → function call args
- `response.output_item.done` → finalized reasoning/message/function call
- `response.completed` → response id、usage、end turn
- `response.failed` / `response.incomplete` / `error` → provider error

Demi `ProviderEvent` 映射目标：

- reasoning item added → `thinking_start`
- reasoning summary/content delta → `thinking_delta`
- reasoning item done → `thinking_signature`，signature 存完整 reasoning item JSON，用于后续 replay `encrypted_content`
- message text delta → `text_delta`
- function call done → `tool_call_requested`
- completed → `response`
- failed/incomplete/error → `error`

pi 的关键经验是 tool id 使用组合 ID：`${call_id}|${item.id}`。Demi 当前 `toolUseId` 只有一个字符串字段，因此应沿用这个组合格式：

- assistant tool replay：拆成 `call_id` 和 `item.id`，生成 Responses `function_call`。
- tool result replay：只用 `call_id` 生成 `function_call_output`。

这能避免改大 `InferenceItem` schema，同时保留 Responses 对 function call item id 的 pairing 需求。

## 7. Transcript Replay 与 Thinking

Codex reasoning replay 不能靠可读 thinking 文本重建。必须保留服务端返回的 reasoning item，尤其是 `encrypted_content`。

最终态规则：

- `assistant_thinking.signature` 对 Codex provider 存 serialized Responses reasoning item。
- replay 时解析 signature，还原 reasoning item。
- signature 为空的本地 thinking 不能作为 Codex reasoning item replay。
- text item replay 应保留 message id/phase；如果当前 `InferenceItem` 无法保存该 metadata，需要扩展 signature 或 provider-local encoding。
- tool replay 使用 `${call_id}|${item.id}` 组合 id。

这和之前 Claude Code provider 的教训一致：本地 transcript 能容纳的 thinking block，不一定能无损喂回具体 provider。provider replay 必须按 provider 的真实 schema 做严格转换。

## 8. 传输目标

官方 Codex 同时支持 HTTP SSE 和 WebSocket。pi 的 TypeScript 实现默认尝试 WebSocket，若连接在 stream start 前失败则 fallback 到 SSE；如果已经开始产出事件，则不切换，避免重复输出。

最终态 `codex` provider 应支持：

- HTTP SSE streaming。
- WebSocket streaming / continuation。
- WebSocket connect timeout。
- SSE header timeout，避免请求卡在无响应 header 阶段。
- retry/backoff，区分 terminal quota/auth/context 错误与可重试 429/5xx/overloaded。
- AbortSignal 贯穿 fetch、reader、WebSocket 和 retry sleep。
- session-scoped transport/cache，不能跨 agent session 污染。

实现可以按提交拆分，但目标设计不能把 SSE-only 当成产品边界。

## 9. pi 可复用与不可复用部分

可复用思路：

- TypeScript Responses request conversion。
- SSE parser 和 header timeout。
- WebSocket fallback 形状。
- tool id 使用 `${call_id}|${item.id}`。
- reasoning item JSON 作为 thinking signature。
- usage 映射：`input_tokens_details.cached_tokens` → `cacheRead`，input token 要扣除 cached tokens。
- fake transport / fake SSE 测试方法。

不可复用为 Demi 设计事实：

- pi 自己的 OAuth login/device flow。Demi 不做登录。
- pi 自己的 `~/.pi/agent/auth.json` storage format。Demi 要复用 Codex auth storage。
- `originator: pi`、pi user agent、pi provider id。
- pi 的模型目录不能直接成为 Demi 的唯一真相；Demi 应允许用户传入 model id，并用真实 backend smoke 验证。

## 10. Demi 最终态设计结论

### 10.1 包边界

新增 provider 应是 Node-only provider 子包，例如 `packages/provider-codex`。它不能进入平台无关根入口的静态依赖闭包。

导出建议：

- `CodexProvider`
- `createCodexProviderDefinition`
- `readCodexAuthState`
- `CodexAuthStore`
- `CodexResponsesTransport`
- request/event conversion helpers，供测试直接断言。

### 10.2 Config

Provider config 必须是可序列化白名单：

- `codexHome?`
- `authStorageMode?`
- `baseUrl?`
- `transport?`
- `headers?`
- `userAgent?`
- `maxRetries?`
- `timeoutMs?`
- `websocketConnectTimeoutMs?`

测试注入 fake fetch/WebSocket/auth store 不能经协议 config 暴露，只能走构造函数 options 或测试-only helper。

### 10.3 Auth

最终态 `CodexAuthStore`：

- 默认读取 `$CODEX_HOME/auth.json` / `~/.codex/auth.json`。
- 兼容官方 `AuthDotJson` schema。
- 支持 ChatGPT token auth、ChatGPT external token auth、OpenAI API key、personal access token、agent identity。
- 对 Bedrock auth 返回明确 unsupported。
- refresh ChatGPT token：主动 refresh + 401 recovery。
- 写回时锁文件、原子替换、权限 `0600`、保留未知字段。
- status 只返回 account label / mode / expiry 概况，不返回 secret。

### 10.4 Request

Provider 每次 run：

- 从 `InferenceRequest` 转 Responses `input`。
- system prompt 放 `instructions`。
- tools 转 Responses function tools。
- thinking 转 `reasoning`。
- 添加 `include: ["reasoning.encrypted_content"]`。
- 使用稳定 `prompt_cache_key`。
- 根据 auth mode 选择 Codex backend 或 OpenAI API backend。

需要补齐的 agent/provider contract：

- provider 可见稳定 `agentSessionId`。
- provider 可见 per-turn request id。
- provider 可见 optional thread/window metadata，或至少有扩展字段承载 Codex headers。

### 10.5 Event

Provider 必须产出真实 provider events：

- text delta 不能等 completed 后一次性补。
- thinking delta 和 signature 要按 stream 事件产生。
- tool call 要等待 args 完整后再请求 agent 执行。
- response usage 要保留 cache read。
- abort 要关闭底层请求并产出可恢复状态。

### 10.6 Error

错误分类至少覆盖：

- auth expired / unauthorized
- context length exceeded
- rate limit / quota exceeded
- server overloaded / retryable transport
- invalid request
- stream protocol error
- abort

这些错误不应把 token、headers 或 auth file 内容带进 transcript。

## 11. 测试要求

默认自动化测试：

- auth parser：ChatGPT、API key、PAT、AgentIdentity、Bedrock unsupported、unknown/malformed。
- auth refresh：near-expiry refresh、401 refresh retry、refresh failure、file lock/atomic write/unknown field preservation。
- redaction：status/error/snapshot 不包含 token。
- request conversion：user text/image、assistant text、assistant thinking signature、tool_use、tool_result、system prompt、tools、reasoning、cache key。
- SSE parser：text、thinking summary、raw reasoning text、function call args delta/done、completed usage、failed/incomplete/error。
- WebSocket fallback：connect-before-start failure fallback SSE；after-start failure does not duplicate stream。
- provider state machine：tool call 后暂停，下一 run 必须带 matching tool_result；missing tool_result fails cleanly。
- abort/retry：fetch reader/WebSocket/retry sleep 都响应 AbortSignal。
- AgentSession integration：真实 shell tools + fake Codex stream 完整 tool roundtrip。

Gated 真实验收：

- 使用本机官方 Codex auth，不做 Demi login。
- 至少覆盖 text、thinking、tool use、usage/cache。
- 覆盖 ChatGPT backend route 和 account header。
- 覆盖 token refresh 前后的真实请求。
- 覆盖长程 session、compact 后继续 tool call、cache usage 不丢。

## 12. 待落地的架构影响

调研结论会影响现有 `docs/agent-rewrite-plan.md`：

- provider contract 需要稳定 session/turn identity。
- Codex provider 需要 Node-only 子包，不能污染平台无关入口。
- Codex auth 是 provider 级能力，不进入 Agent Loop。
- WebSocket/SSE transport 是 provider 内部机制，不进入 `InferenceRequest`。
- Responses reasoning/tool replay 的 provider-specific signature 需要作为 transcript replay 的长期约束。

## 13. 落地记录

已新增 `packages/provider-codex`，并把 TUI provider 选择扩展为 `claude-code` / `codex`。当前实现按调研结论覆盖：

- 复用官方 `$CODEX_HOME/auth.json` / `~/.codex/auth.json`，支持 ChatGPT token、API key、PAT、可用的 agent identity 记录；Bedrock 明确 unsupported。
- ChatGPT token near-expiry refresh、401 force refresh retry、原子写回、未知字段保留、`0600` 权限和 secret redaction。
- Responses request conversion：stable `sessionId`/`requestId`、`prompt_cache_key`、reasoning include、signed thinking replay、tool id 组合、tool result replay。
- SSE + WebSocket + auto fallback transport；WebSocket 使用 `responses_websockets=2026-02-06` beta header，并在 `response.completed` / failed / incomplete / error 事件后主动结束 stream。
- Responses stream 映射为 Demi `ProviderEvent`，覆盖 text、thinking、tool call、usage/cache、failed/incomplete/error。
- `AgentSession` + shell tools 集成测试，确保 Codex function call 能执行工具并把 `tool_result` 回灌到下一轮 provider request。

测试覆盖记录见 `docs/testing.md#531-codex-provider`。真实 Codex 网络验收提供 gated `real-codex.e2e.test.ts` 入口，需要显式环境变量开启，不进入默认 `bun run test`；本机已用官方 Codex auth 跑通 text、medium thinking、cache read 和 shell tool roundtrip，SSE 与默认 `auto` transport 均通过。

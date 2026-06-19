# Provider Reasoning 与 Speed Controls 调研

| | |
|---|---|
| 日期 | 2026-06-20 |
| 状态 | 调研与方案记录 |
| 范围 | provider 模型 reasoning 控制、Claude Code effort 元数据、Codex reasoning levels、Codex fast mode |

## 1. 问题

- Demi 应该从哪里获取每个模型支持的 thinking / reasoning 等级？
- provider 返回未来新增的 reasoning effort 字符串时，Demi 是否应该丢弃？
- Codex fast mode 是 reasoning 的一部分，还是单独的模型运行选项？

## 2. 已验证来源

- 已将 `~/Projects/References/codex` 快进到 `c83618ab2`。
- 已阅读 Codex app-server model schema：
  - `codex-rs/app-server-protocol/schema/typescript/v2/Model.ts`
  - `codex-rs/app-server/README.md`
- 已阅读 Codex core 处理：
  - `codex-rs/protocol/src/openai_models.rs`
  - `codex-rs/protocol/src/config_types.rs`
  - `codex-rs/core/src/session/turn_context.rs`
  - `codex-rs/core/src/client.rs`
  - `codex-rs/tui/src/service_tier_resolution.rs`
- 已用本机官方 Codex auth 请求真实 Codex backend：
  - `https://chatgpt.com/backend-api/codex/models?client_version=0.130.0`
- 已请求真实 `models.dev`：
  - `https://models.dev/api.json`
- 已检查 Claude Code CLI：
  - `claude --help`
  - `claude --settings '{"alwaysThinkingEnabled":false}' --version`
  - `claude --settings '{"fastMode":true}' --version`
- 已参考 `~/Projects/References/t3code` 中 Claude CLI options 与 Codex service tier forwarding。

## 3. 调研结论

### 3.1 Reasoning effort id 必须是 provider 广告的字符串

Codex 官方 schema 把 `ReasoningEffort` 定义为模型广告的非空字符串。Codex app-server README 也明确要求 client 保留 `supportedReasoningEfforts` 的原始顺序，不能从 effort 名称推导顺序。

Demi 当前是固定枚举：

```ts
'none' | 'minimal' | 'low' | 'medium' | 'high' | 'max' | 'xhigh'
```

这不是最终态。provider catalog 未来可能返回新 effort id，Demi 应该保留并透传 provider 广告的字符串。已知 id 只用于显示 label，不应该用于决定能力边界。

### 3.2 Codex reasoning levels 来自 backend model catalog

真实 Codex backend 当前返回：

```json
{
  "slug": "gpt-5.5",
  "default_reasoning_level": "medium",
  "supported_reasoning_levels": [
    { "effort": "low", "description": "Fast responses with lighter reasoning" },
    { "effort": "medium", "description": "Balances speed and reasoning depth for everyday tasks" },
    { "effort": "high", "description": "Greater reasoning depth for complex problems" },
    { "effort": "xhigh", "description": "Extra high reasoning depth for complex problems" }
  ]
}
```

结论：

- Codex provider 应直接映射 `supported_reasoning_levels[]`。
- `default_reasoning_level` 不能写入 Demi 的运行默认字段；除非后续有独立 raw metadata 字段，否则当前实现忽略它。
- description 应保留。
- 未知 effort 字符串不能被丢弃。
- Demi 的默认 reasoning 选择固定为 `null`：用户没有显式选择 effort 时，请求里不发送 reasoning effort。

### 3.3 Claude Code reasoning levels 来自 `models.dev` 与 CLI 可控能力

`models.dev` 的 Anthropic catalog 包含 `reasoning_options`。

真实 catalog 示例：

```json
{
  "id": "claude-opus-4-8",
  "reasoning": true,
  "reasoning_options": [
    { "type": "effort", "values": ["low", "medium", "high", "xhigh", "max"] }
  ]
}
```

```json
{
  "id": "claude-sonnet-4-6",
  "reasoning": true,
  "reasoning_options": [
    { "type": "effort", "values": ["low", "medium", "high", "max"] },
    { "type": "budget_tokens", "min": 1024 }
  ]
}
```

Claude Code CLI help 当前公开：

```text
--effort <level>  Effort level for the current session (low, medium, high, xhigh, max)
```

结论：

- Claude Code provider 应把 `reasoning_options[type="effort"].values[]` 映射为 effort options。
- Claude Code provider 不应从 `models.dev` 推断默认 effort；catalog 没给默认值，而且 Demi 的默认运行语义就是不请求显式 effort。
- `budget_tokens` 是模型元数据，但目前不是已确认的 Claude Code CLI request control。在验证 CLI 真实模型行为前，不应作为 Demi 可选运行时控制暴露。
- Claude CLI parser 接受 `--settings '{"alwaysThinkingEnabled":false}'` 和 `--settings '{"fastMode":true}'`，t3code 也这样使用；但它们不是 `claude --help` 中的一等 flag。产品化前需要单独做真实路径验证。

### 3.4 Codex fast mode 是 service tier，不是 reasoning

Codex 官方把 fast mode 建模为 service tier：

```json
{
  "service_tiers": [
    { "id": "priority", "name": "Fast", "description": "1.5x speed, increased usage" }
  ],
  "default_service_tier": null,
  "additional_speed_tiers": ["fast"]
}
```

关键点：

- `service_tiers` 是当前 catalog 字段。
- `additional_speed_tiers` 是 deprecated legacy metadata。
- 用户可见旧值 `fast` 会被 Codex config 归一化成请求值 `priority`。
- Responses request 字段是 `service_tier`。
- 官方 Codex request building 会过滤不支持的 service tier，并省略 `default` sentinel。
- fast mode 和 reasoning effort 是独立选项。

结论：

- Demi 应把 service tier 独立于 thinking / reasoning 表示。
- Codex fast mode 的真实 wire value 是 `priority`。
- UI 可以显示 `Fast`，provider request 必须发送 `priority`。
- 未选择 service tier 时应省略 `service_tier`，不能发送 `"default"`。

### 3.5 默认值与外部进程边界

Reasoning effort 的默认选择不来自 provider catalog，也不来自外部 CLI 探测。Demi 的默认值是固定的 `null`，语义是“不请求显式 effort”。如果 provider 自身有隐含默认策略，应由 provider 服务端或官方 transport 自行处理，Demi 不把它复制成请求参数，也不把 provider default 写入当前运行默认字段。

模型目录、能力发现、auth 状态和运行时状态应优先使用协议/API/文件状态。除真正的 provider transport 外，不应通过调用外部 CLI 来发现模型目录、版本、auth 或 capability。Claude Code 当前真实请求路径仍需要走官方 provider transport；如果后续迁移到 RPC，也应把 RPC 作为 transport 边界，而不是增加独立 CLI preflight。

## 4. 最终态设计方向

### 4.1 Provider model metadata

当前 provider model catalog 里的字段太窄：

```ts
supportedThinkingEfforts: ThinkingEffort[] | null
defaultThinkingEffort: ThinkingEffort | null
```

应改为 provider 广告的 option metadata：

```ts
interface ProviderReasoningEffortOption {
  id: string
  label: string
  description?: string
}

interface ProviderReasoningCapability {
  type: 'effort'
  options: ProviderReasoningEffortOption[]
}

interface ProviderBudgetReasoningCapability {
  type: 'budget_tokens'
  minBudgetTokens: number | null
  maxBudgetTokens: number | null
  defaultBudgetTokens: number | null
}

interface ProviderServiceTier {
  id: string
  label: string
  description?: string
}
```

核心规则：`id` 是 provider wire value，label 只用于显示。

### 4.2 Core model selection

Core 不应再要求 reasoning effort 是固定 enum。选择值应是字符串：

```ts
type ReasoningEffortId = string
```

已知 effort id 可以有更好的 display label，但校验必须基于当前模型 catalog，而不是本地 enum。

Core 不保存自动默认 effort。没有用户显式选择时，`thinking` 必须是 `null`，provider request builder 不能从 catalog 的 provider default 补出 effort。

Model selection 还应携带 service tier：

```ts
interface ModelSelection {
  providerId: string
  model: Model
  thinking: ThinkingConfig | null
  serviceTierId: string | null
}
```

`serviceTierId: null` 表示标准路由，不发送 service tier。

### 4.3 Provider request

`InferenceRequest` 应包含 service tier 选择：

```ts
serviceTierId?: string | null
```

TUI / model resolver 只在 selected model catalog 包含该 service tier 时允许选择；Codex provider 对非空 `serviceTierId` 原样映射到 Responses `service_tier`。

不支持的 service tier 不能被静默改写成另一个 tier。稳定 UI 路径应阻止无效选择；provider request building 对 `null` 应省略字段。如果 catalog metadata 不可用，Demi 不应暴露或发送 service tier 控制。

### 4.4 TUI 行为

- 不维护本地固定 thinking level 列表。
- catalog 可用时，按 provider 返回顺序展示 effort options。
- 模型没有 effort options 时，不显示 effort 可用。
- Claude Code effort options 来自 `models.dev` `reasoning_options`。
- Codex effort options 来自 backend `supported_reasoning_levels`。
- Codex 模型有 `service_tiers` 时，单独展示 service tier 选择。
- service tier 不能叫 thinking，也不能并入 thinking settings。

## 5. Provider 映射

### 5.1 Claude Code

Catalog mapping：

- `reasoning_options[type="effort"].values[]` -> effort options，保留顺序。
- `reasoning_options[type="budget_tokens"]` -> 只有验证 CLI runtime mapping 后才记录为可用运行时控制。
- `reasoning: true` 但缺少 `reasoning_options` -> reasoning 支持存在，但具体控制未知。

Request mapping：

- selected effort id -> `claude --effort <id>`
- no selected effort -> 不传 `--effort`
- 不接受 effort alias
- 在验证前不暴露 budget tokens 控制

### 5.2 Codex

Catalog mapping：

- `supported_reasoning_levels[].effort` -> effort option id。
- `supported_reasoning_levels[].description` -> effort option description。
- `default_reasoning_level` -> 当前忽略；后续若增加独立 raw metadata 字段，可以保留，但不能映射成运行默认。
- `service_tiers[]` -> service tier options。
- `default_service_tier` -> 当产品策略决定使用 catalog default 时，作为默认 service tier id。
- `additional_speed_tiers` -> 新行为忽略。

Request mapping：

- selected effort id -> Responses `reasoning.effort`
- no selected effort -> 省略 `reasoning.effort`
- selected service tier id -> Responses `service_tier`
- `null` service tier -> 省略 `service_tier`
- 不引入 `fast` legacy alias 作为 Demi 面向用户的值；使用 `priority`

## 6. 测试要求

- Claude `models.dev` fixture：
  - effort-only model：`["low", "medium", "high", "xhigh", "max"]`
  - effort + budget model：`["low", "medium", "high", "max"]` 加 `budget_tokens`
  - budget-only model
  - `reasoning: true` 但没有 options
- Codex backend fixture：
  - reasoning options 和 descriptions
  - 未知 future reasoning effort string
  - `service_tiers: [{ id: "priority", name: "Fast" }]`
  - `additional_speed_tiers: ["fast"]` 被忽略
- TUI / model resolver tests：
  - 保留 provider effort order
  - 未显式选择 effort 时，model resolver 生成 `thinking: null`
  - catalog-backed selection 拒绝不在 catalog 内的 effort
  - 不把 service tier 显示成 thinking
  - service tier 选择与省略行为确定
- Provider request tests：
  - Claude 只在选中 effort 时传 `--effort <id>`
  - Claude 未选中 effort 时不传 `--effort`
  - Codex 独立写入 `reasoning.effort` 与 `service_tier`
  - Codex 未选中 effort 时不写入 `reasoning.effort`
  - Codex 标准路由省略 `service_tier`
  - catalog 广告的未知 reasoning effort 能透传
- Gated smoke：
  - Claude Code 使用 catalog 广告的 effort 跑真实模型。
  - Codex 在广告 Fast 的模型上用 `service_tier: "priority"` 跑真实模型。

## 7. 当前落地状态

- Demi 已把 reasoning effort id 放宽为 provider wire string，Codex 和 Claude catalog 都会保留未知 future effort id。
- Claude Code catalog 已映射 `models.dev` 的 `reasoning_options[type="effort"].values[]`。
- Codex catalog 已映射 `service_tiers[]`，并忽略 legacy `additional_speed_tiers` alias。
- `ModelSelection` / `InferenceRequest` 已携带可选 `serviceTierId`；Codex request 只在选中时写入 Responses `service_tier`。
- TUI 当前把 `--no-thinking` 表示为“不请求显式 effort”，banner 显示 `thinking: not requested`。

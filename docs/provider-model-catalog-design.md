# Provider Model Catalog 设计

| | |
|---|---|
| 日期 | 2026-06-20 |
| 状态 | 方案 |
| 范围 | Claude Code / Codex provider 的模型列表、模型过滤、模型元数据与 REPL 选择 |

## 1. 目标

模型列表是 provider 能力，不属于 REPL 硬编码逻辑。

目标：

- 上层只使用 provider 支持的 full model id，不做 `opus`、`sonnet` 这类本地别名映射。
- REPL / AgentClient 从 provider catalog 获取模型列表、默认选择和能力元数据。
- Codex 模型列表复用官方 Codex 鉴权材料直接请求 Codex backend，不走 Demi 登录流程。
- Claude Code 模型列表使用 `models.dev` 的 Anthropic catalog，避免在仓库里维护 Claude 模型表。
- catalog 拉取失败不能伪造模型；可以使用带 stale 标记的缓存，也可以要求用户显式传 full id。
- reasoning effort 与 Codex service tier / fast mode 的补充调研见 `docs/provider-reasoning-speed-controls-research.md`。

运行边界：

- 模型目录、能力发现、auth 状态和运行时状态不应依赖外部 CLI preflight。
- provider 真正发起模型请求所需的官方 transport 是独立边界；目录和状态查询不能为了方便复用它去跑额外命令。
- 对外部接口的具体方案必须先用真实 API、协议或文件状态验证能跑通，再写成设计。

非目标：

- 不用模型列表证明账号一定有调用权限。真实调用失败仍由 provider error 处理。
- 不把模型目录做成产品推荐系统。排序和过滤只服务可用性与可读性。
- 不为了兼容旧命令保留别名解析。

## 2. 统一契约

Provider definition 增加可选模型目录能力：

```ts
interface ProviderModel {
  providerId: string
  id: string
  displayName: string
  description?: string
  contextWindow: number | null
  outputLimit: number | null
  supportsTools: boolean | null
  supportsAttachments: boolean | null
  supportsReasoning: boolean | null
  supportedThinkingEfforts: string[] | null
  defaultThinkingEffort: string | null
  serviceTiers?: Array<{
    id: string
    label: string
    description?: string
  }> | null
  defaultServiceTierId?: string | null
  cost?: {
    input: number | null
    output: number | null
    cacheRead: number | null
    cacheWrite: number | null
  }
  sourceFetchedAt: string
  stale: boolean
}

interface ProviderModelList {
  providerId: string
  models: ProviderModel[]
  defaultModelId: string | null
  warnings: string[]
  sourceFetchedAt: string
  stale: boolean
}
```

契约规则：

- `id` 必须是请求 provider 时实际传入的 full model id。
- `displayName` 只用于展示，不能参与请求转换。
- 缺失的能力字段用 `null`，不能从模型名臆测。
- `defaultModelId` 必须来自 catalog 中的模型；没有可靠默认值时返回 `null`。
- 手动传入 `--model` 时不做别名 rewrite；如果值不是 full id，应直接报错并提示从模型列表选择。
- 如果用户传入 catalog 中不存在但格式像 full id 的模型，可以允许继续并给 warning；provider 真实响应是最终裁决。

## 3. Codex 模型目录

Codex 使用官方 auth 文件直接请求模型列表接口。

数据源：

- 默认读取 `$CODEX_HOME/auth.json`，未设置时读取 `~/.codex/auth.json`。
- ChatGPT / Codex auth 走 `https://chatgpt.com/backend-api/codex/models?client_version=<version>`。
- `client_version` 必须显式发送。默认值使用 Demi 维护并验证过的 catalog protocol client version；不调用 `codex --version` 做运行时发现。配置覆盖只用于测试或紧急兼容。
- OpenAI API key auth 不请求 ChatGPT Codex backend。后续可以接 OpenAI 模型列表，但不能把 ChatGPT catalog 套到 API key auth 上。

鉴权规则：

- 请求头使用 `Authorization: Bearer <access_token>`。
- 有 `account_id` 时发送 `ChatGPT-Account-ID`。
- access token 临近过期或接口返回 401 时，按官方 refresh 语义刷新并写回 auth store。
- 写回 auth 文件必须保留未知字段、原子替换、权限 `0600`，错误和日志不得包含 secret。

返回处理：

- 保留 Codex backend 返回的模型 full id；当前接口字段是 `slug`，例如 `gpt-5.5`、`gpt-5.4-mini`。
- 保留 backend 返回顺序，除非接口明确给出排序字段。
- capability 字段只从接口返回值映射；没有字段就设为 `null`。
- cache key 至少包含 provider、base URL、account id、client version 和 auth mode。
- Codex catalog cache TTL 应短于静态 catalog；建议 5 到 30 分钟。401、403、quota、网络失败要区分，不用旧缓存掩盖 auth 错误。

## 4. Claude Code 模型目录

Claude Code 的模型目录使用 `models.dev`，不读取 Claude Code 内部硬编码 picker。

数据源：

- 请求 `https://models.dev/api.json`。
- 读取 `.anthropic.models`。
- 每个 entry 的 key 是 Claude full model id，例如 `claude-opus-4-8`。
- provider 请求时使用 key 本身，不加 `anthropic/` 前缀。

默认过滤策略：

- 只展示 `claude-` 开头的 Anthropic 模型。
- 只展示 `modelVersion >= minimumModelVersion` 的模型。
- 默认 `minimumModelVersion = "4.6"`。
- 默认不按 family、latest、生命周期或能力做额外排除；生命周期只作为展示/告警元数据，除非未来显式增加用户配置。
- 不把当前模型名写成仓库内常量。

版本解析规则：

- 新式 Claude id：`claude-<family>-<major>-<minor>`，如 `claude-opus-4-8` 解析为 `4.8`。
- 带日期快照的新式 id：`claude-opus-4-5-20251101` 解析为 `4.5`，日期只作为 snapshot 后缀，不参与版本比较。
- 单 major 新式 id：`claude-fable-5` 解析为 `5.0`。
- 旧式 Claude id：`claude-3-5-sonnet-20241022` 解析为 `3.5`。
- `claude-sonnet-4-20250514` 解析为 `4.0`。
- 无法解析版本的模型不进入默认列表，并产生 warning。

比较规则：

- `major > minMajor` 时通过。
- `major === minMajor && minor >= minMinor` 时通过。
- 默认阈值 `4.6` 因此包含 `4.6`、`4.7`、`4.8`、`5.0`、未来 `5.x`。
- 默认阈值 `4.6` 会排除 `claude-haiku-4-5`、`claude-sonnet-4-5`、`claude-opus-4-5`、Claude 3.x。

当前阈值示例：

```text
include: claude-opus-4-6
include: claude-opus-4-7
include: claude-opus-4-8
include: claude-sonnet-4-6
include: claude-fable-5
exclude: claude-haiku-4-5
exclude: claude-sonnet-4-5
exclude: claude-opus-4-5
exclude: claude-3-5-sonnet-20241022
```

能力映射：

- `limit.context` → `contextWindow`
- `limit.output` → `outputLimit`
- `reasoning` → `supportsReasoning`
- `attachment` → `supportsAttachments`
- `cost.input/output/cache_read/cache_write` → `cost`
- thinking effort options 只有 catalog 明确提供时才填充；否则设为 `null`。
- catalog 中的 provider default effort 不写入运行默认字段；REPL / agent 默认选择固定为不请求显式 effort。

缓存：

- `models.dev` catalog 可缓存 24 小时，并使用 ETag / Last-Modified 条件请求。
- 网络失败时可以返回 stale cache，但必须设置 `stale: true` 和 warning。
- 没有缓存且请求失败时返回错误；不要退回仓库内模型表。

## 5. REPL 与上层行为

REPL 不再拥有 provider-specific model defaults。

启动行为：

- provider 初始化后先请求 `listModels()`。
- 如果 catalog 有 `defaultModelId`，选中该模型。
- 如果没有 `defaultModelId`，优先使用用户上次选择的 full id；仍没有时选中列表第一项，但把选择来源标成 catalog selection，不称为 provider default。
- 非交互命令如果没有模型列表且没有显式 `--model`，应失败并提示原因。

手动输入：

- `--model` 必须是 full id。
- `opus`、`sonnet`、`haiku` 这类 alias 直接报错。
- 显式 full id 不需要先拉 catalog；如果没有 catalog 元数据，上层按未知 context/capability 处理，真实 provider 响应是最终裁决。
- `--service-tier` 需要 catalog-backed model selection；显式 `--model` 路径不发送 service tier，避免在没有 catalog metadata 时暴露未验证控制。

请求行为：

- `InferenceRequest.modelId` 永远保存 full id。
- transcript 中的 `modelId` 永远保存 full id。
- thinking capability 来自当前模型元数据；如果缺失，provider 仍可尝试请求，但 UI 不能假装有完整能力信息。

## 6. 测试要求

默认自动化测试：

- `models.dev` fixture 包含 Claude 3.x、4.0、4.5、4.6、4.8、5.0、日期快照和无法解析 id。
- 断言默认 `minimumModelVersion = "4.6"` 包含所有 `>= 4.6`，排除 `< 4.6`，并包含未来 `5.x`。
- 断言过滤逻辑不按 family 白名单实现：新增未知 family 的 `claude-newfamily-5` 应进入列表。
- 断言 key 作为 full id 输出，不加 `anthropic/` 前缀。
- 断言缺失 capability 字段映射为 `null`，不从名字臆测。
- 断言 stale cache 带 warning，网络失败且无 cache 不返回硬编码模型。
- Codex fixture 覆盖默认静态 `client_version`、显式 override、ChatGPT auth headers、`slug` id 映射、401 refresh retry、cache key、server order 保留。
- Codex fixture 覆盖 `service_tiers` 映射为 provider wire id，且不从 `additional_speed_tiers` 生成 legacy `fast` alias。
- REPL / CLI 测试覆盖 alias 被拒绝、full id 透传、catalog default 或 first selection 不落回硬编码。

Gated 真实验收：

- Codex 使用本机 `~/.codex/auth.json` 拉真实模型列表，验证静态 catalog client version、auth header、refresh 后重试。
- Claude 使用真实 `models.dev` 拉取并应用 `minimumModelVersion = "4.6"`，验证列表中不出现 4.5 / 3.x 模型。
- 使用列表中的 full id 发起真实 provider smoke，确认请求没有别名 rewrite。

## 7. 外部接口验证记录

验证日期：2026-06-20。

这些验证只证明外部目录接口和鉴权路径当前可跑通；默认自动化测试仍需要用 fixture 固化转换规则。

### 7.1 Codex backend models

验证命令：

```bash
node - <<'NODE'
const fs = require('node:fs/promises')
const os = require('node:os')
const path = require('node:path')

;(async () => {
  const authPath = path.join(process.env.CODEX_HOME || path.join(os.homedir(), '.codex'), 'auth.json')
  const auth = JSON.parse(await fs.readFile(authPath, 'utf8'))
  const version = '0.130.0'
  const headers = {
    authorization: `Bearer ${auth.tokens.access_token}`,
    accept: 'application/json',
  }
  if (auth.tokens?.account_id) headers['chatgpt-account-id'] = auth.tokens.account_id

  const res = await fetch(`https://chatgpt.com/backend-api/codex/models?client_version=${version}`, { headers })
  const json = await res.json()
  console.log(JSON.stringify({
    status: res.status,
    clientVersion: version,
    count: json.models?.length ?? null,
    models: (json.models ?? []).map((m) => ({
      slug: m.slug,
      displayName: m.display_name,
      contextWindow: m.context_window,
      maxContextWindow: m.max_context_window,
      defaultReasoningLevel: m.default_reasoning_level,
      supportedReasoningLevels: (m.supported_reasoning_levels ?? []).map((level) => level.effort),
      inputModalities: m.input_modalities,
    })),
  }, null, 2))
})()
NODE
```

结果摘要：

```json
{
  "status": 200,
  "clientVersion": "0.130.0",
  "count": 5,
  "models": [
    { "slug": "gpt-5.5", "displayName": "GPT-5.5", "contextWindow": 272000, "maxContextWindow": 272000 },
    { "slug": "gpt-5.4", "displayName": "GPT-5.4", "contextWindow": 272000, "maxContextWindow": 1000000 },
    { "slug": "gpt-5.4-mini", "displayName": "GPT-5.4-Mini", "contextWindow": 272000, "maxContextWindow": 272000 },
    { "slug": "gpt-5.3-codex-spark", "displayName": "GPT-5.3-Codex-Spark", "contextWindow": 128000, "maxContextWindow": 128000 },
    { "slug": "codex-auto-review", "displayName": "Codex Auto Review", "contextWindow": 272000, "maxContextWindow": 1000000 }
  ]
}
```

结论：

- 直接读取 `~/.codex/auth.json` 并请求 Codex backend models 接口可跑通。
- 静态 `client_version=0.130.0` 可用，不需要调用 `codex --version`。
- 当前模型 id 字段是 `slug`，不是 `model` 或 `id`。
- 接口返回 `context_window`、`max_context_window`、`default_reasoning_level`、`supported_reasoning_levels`、`input_modalities` 等能力字段。

### 7.2 models.dev Anthropic catalog

验证命令：

```bash
node - <<'NODE'
function parseClaudeVersion(id) {
  if (!id.startsWith('claude-')) return null
  const parts = id.slice('claude-'.length).split('-')
  const isInt = (s) => /^\d+$/.test(s)
  const isDate = (s) => /^\d{8}$/.test(s)
  if (isInt(parts[0])) return { major: Number(parts[0]), minor: isInt(parts[1]) ? Number(parts[1]) : 0 }
  if (!isInt(parts[1])) return null
  return {
    major: Number(parts[1]),
    minor: parts[2] && isInt(parts[2]) && !isDate(parts[2]) ? Number(parts[2]) : 0,
  }
}
function gte(version, min) {
  return version.major > min.major || (version.major === min.major && version.minor >= min.minor)
}

;(async () => {
  const res = await fetch('https://models.dev/api.json', { headers: { accept: 'application/json' } })
  const json = await res.json()
  const min = { major: 4, minor: 6 }
  const models = Object.entries(json.anthropic?.models ?? {})
    .filter(([id]) => id.startsWith('claude-'))
    .map(([id, model]) => ({ id, name: model.name, version: parseClaudeVersion(id), limit: model.limit, reasoning: model.reasoning, attachment: model.attachment }))
  const included = models.filter((m) => m.version && gte(m.version, min))
  const excluded = models.filter((m) => !m.version || !gte(m.version, min))
  console.log(JSON.stringify({
    status: res.status,
    claudeModelCount: models.length,
    minimumModelVersion: '4.6',
    included: included.map((m) => ({ id: m.id, name: m.name, version: `${m.version.major}.${m.version.minor}`, context: m.limit?.context, output: m.limit?.output })),
    hasExcluded45: excluded.some((m) => m.id === 'claude-haiku-4-5'),
    hasIncluded5x: included.some((m) => m.version.major >= 5),
  }, null, 2))
})()
NODE
```

结果摘要：

```json
{
  "status": 200,
  "claudeModelCount": 25,
  "minimumModelVersion": "4.6",
  "included": [
    { "id": "claude-opus-4-7", "name": "Claude Opus 4.7", "version": "4.7", "context": 1000000, "output": 128000 },
    { "id": "claude-opus-4-8", "name": "Claude Opus 4.8", "version": "4.8", "context": 1000000, "output": 128000 },
    { "id": "claude-fable-5", "name": "Claude Fable 5", "version": "5.0", "context": 1000000, "output": 128000 },
    { "id": "claude-opus-4-6", "name": "Claude Opus 4.6", "version": "4.6", "context": 1000000, "output": 128000 },
    { "id": "claude-sonnet-4-6", "name": "Claude Sonnet 4.6", "version": "4.6", "context": 1000000, "output": 64000 }
  ],
  "hasExcluded45": true,
  "hasIncluded5x": true
}
```

结论：

- `models.dev/api.json` 的 Anthropic catalog 可跑通。
- `.anthropic.models` 的 key 是 Claude full model id，可直接作为 provider model id。
- 默认 `minimumModelVersion = "4.6"` 当前会展示 5 个模型，排除 `claude-haiku-4-5` 和 3.x，并包含 `claude-fable-5`。

## 8. 落地影响

需要删除或迁移的现有行为：

- REPL 中的 provider-specific 默认模型硬编码。
- REPL 中的 `opus -> claude-opus-4-8` 映射。
- REPL 中按 provider 写死的 context window 和 thinking efforts。

需要新增的边界：

- provider package 暴露模型目录能力。
- model catalog 缓存位于 provider 层或 provider-adapter 层，不进入 Agent Loop。
- 模型列表失败是 provider/catalog 状态，不应污染当前 session transcript。

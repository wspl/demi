# Demi 整体评估与改造方案

- 日期：2026-07-06
- 基线：`main` @ 5bc6c11（feat/nested-command-tree 合并后）
- 验证状态：`bun run typecheck` 干净；`bun run test` 510 pass / 11 skip / 0 fail
- 定位：本文档是一次全量代码审读的结论与后续改造的总纲。每个改造阶段动工前，以本文档对应章节为设计输入；若实现与本文冲突，先修订本文再继续。

## 执行状态（2026-07-06 更新）

| 阶段 | 分支 | 状态 |
| --- | --- | --- |
| 1 转录 patch 管线 | `feat/transcript-patch-pipeline` | 已合并（journal patch、append_text op、revision 同步、落盘节流） |
| 2 恢复一致性 + 接管 | `fix/session-restore-and-takeover` | 已合并（单一 state 对象、SessionOwnershipRegistry 接管语义） |
| 3 回合级重试 | `feat/turn-retry-policy` | 已合并（TurnRetryPolicy、retry_scheduled 事件、Retry-After 采纳、压缩摘要复用） |
| 4 shell 记账清理 | `refactor/shell-stream-accounting` | 已合并（死状态整层删除而非封装——原 24 字段中约 15 个为只写状态；保留名改为推导式；助手函数并入 utils） |
| 5 命令双投影 | `feat/command-native-tool-projection` | 已合并（RegisteredCommandOperation + AgentServerOptions.commandTools，默认关闭） |
| 6 估算与默认值 | `fix/token-estimate-and-defaults` | 已合并（usage 锚定估算、多模态计权、anthropic effort→budget 映射与 max_tokens 派生、store 目录迁 XDG、replay 截断可配） |
| 7a agent-eval | — | 未启动（按 §5.7 与既有内部计划推进；双投影跑分依赖它给出 commandTools 默认值结论） |
| 7b fork 治理 | `docs/fork-policy-and-execution-status` | 已合并（`docs/just-bash-fork-policy.md`） |

其余未执行项：repl/index.ts 拆分（§5.6-4，纯整理，无行为变化）。§5.8-3 的 submodule 指针核实为工作区落后而非超前，已通过 `git submodule update` 对齐，无需落账。

---

## 1. 总体评估

Demi 的分层架构、边界契约（`docs/package-boundaries.md` + `platform-entrypoints` 边界测试）、以及 agent 运行时的回合/steer/yield/压缩语义，都已达到"最终态"质量，不需要推倒重来。主要差距集中在三类：

1. **热路径性能**：转录提交路径在流式增量下是 O(n²)（问题 P0-1），这是推向通用库前的第一优先级。
2. **可靠性与一致性**：快照恢复的 state 分裂（P0-2）、同 sessionId 并发打开无互斥（P0-3）、provider 无重试策略（P1-2）。
3. **通用化缺口**：shell-only 工具面缺少"注册命令 → 原生工具"的双投影选项（§5.5）、token 估算忽略多模态内容（P1-1）、若干默认值与产品语义矛盾（P1-4 等）。

### 分模块结论速览

| 模块 | 评价 | 主要行动 |
| --- | --- | --- |
| `core` / `utils` / `provider` | 优秀，保持 | 清理陈旧注释；重复助手并入 utils |
| `shell` | 设计好、实现需重构 | 输出记账收敛（§5.4）；去重/死代码清理 |
| `host-local` | 合格 | store 默认目录迁出 tmpdir（§5.6） |
| `agent` | 项目最高质量 | 转录热路径重构（§5.1）；恢复一致性（§5.2）；重试层（§5.3） |
| `coding-agent` | 合格偏好 | 双投影后受益；State 泛型需实证 |
| `provider-*` | 结构一致 | anthropic thinking/max_tokens 补全（§5.6） |
| `repl` / `web` / `web-ui` | 合格 | repl 拆分（低优先级） |
| `just-bash` fork | 最大隐性负债 | 同步策略文档 + 裁剪评估（§5.8） |
| `docs` | 优秀 | 持续作为契约维护 |

---

## 2. 问题登记表

优先级定义：P0 = 正确性/可扩展性阻断；P1 = 设计缺口；P2 = 卫生问题。

### P0-1 转录提交路径 O(n²)

- 证据：`packages/agent/src/provider-turn-loop.ts` `applyProviderEvent` 对每个 provider 事件（含每个 `text_delta`）调用 `commitTranscript()`；`packages/agent/src/session.ts` `commitTranscript` 每次 `structuredClone` 全部 blocks 并整份 `store.saveSnapshot`；`packages/agent/src/server.ts` 每次 `transcript_changed` 用 `diffTranscriptBlocks`（逐块深比较，见 `packages/agent/src/patch.ts`）反推 patch。
- 影响：单个流式 token 的处理成本随会话历史线性增长；长会话（数千块）下 clone + 深比较 + JSON 写盘 × 每 delta，CPU 与 IO 都不可接受。
- 处置：§5.1。

### P0-2 快照恢复的 state 分裂

- 证据：`packages/agent/src/server.ts` `open()` 中 `const state = agent.initialState()` 被 harness 的 `host()` / `commands()` 闭包捕获，且 `runtime.initialState: () => state` 永远返回这一对象；而恢复路径 `AgentSession.fromSnapshot` 使用 `structuredClone(snapshot.state)`。恢复会话时 harness 侧与 session 侧持有两个互不相通的 state 对象。
- 影响：当前 `CodingState = {}` 掩盖问题；任何有状态 harness（State 泛型的存在意义）在恢复后会出现静默的状态错位。
- 处置：§5.2。

### P0-3 同 sessionId 并发打开无互斥

- 证据：`sessionId` 由客户端拥有（`frames.ts` open frame）；两个连接以同一 id `open`，各自创建 `AgentSession` 与 `HostAgentSessionStore`，写同一个 `agent-sessions/<id>/snapshot.json`，最后写者赢。
- 影响：断线重连（旧连接未及时探测关闭）即可触发；转录可能被交错快照损坏。
- 处置：§5.2。

### P1-1 token 估算忽略多模态内容

- 证据：`packages/agent/src/transcript.ts` `estimateBlockText` 对 image 只计 mediaType/url 字符串、对 document 只计文件名；即上下文估算中图片≈0 token。
- 影响：图片密集会话不触发 preflight 压缩，直至真实请求超限，多付一轮被拒绝的请求（依赖 auto-recover 兜底）。
- 处置：§5.6。

### P1-2 provider 无重试策略

- 证据：`packages/provider/src/http.ts` 已将错误分类为 `rate_limit` / `overloaded` / `auth_expired` / `context_length_exceeded`，但没有任何消费方做退避重试；`ProviderTurnLoop.streamProviderOnce` 收到 error 事件即抛 `ProviderStreamError` 终结回合。
- 影响：一次瞬时 429/529 终结整个回合，需要人工 retry；与"长时运行 agent"定位矛盾。
- 处置：§5.3。

### P1-3 anthropic provider thinking/max_tokens 映射不完整

- 证据：`packages/provider-anthropic-api/src/provider.ts` `buildAnthropicMessagesBody` 只映射 `thinking.type === 'budget'`，`effort` / `adaptive` 配置被静默丢弃；`max_tokens` 固定默认 4096，未从模型 `outputLimit` 推导。
- 影响：用户在 UI 选择 effort 档位后请求实际不带 thinking；coding 场景 4096 输出上限过低。
- 处置：§5.6。

### P1-4 LocalHost store 默认落在 os tmpdir

- 证据：`packages/host-local/src/local-host.ts` `defaultStoreRoot` = `join(tmpdir(), 'demi-host-local-store', hash)`。
- 影响："持久化会话历史 / 跨设备 resume 列表"的产品语义与"重启即失忆"的默认存储矛盾。
- 处置：§5.6。

### P2-1 重复助手函数（违反自家复用规则）

- `packages/shell/src/environment.ts` 底部重新实现 `utf8Bytes` / `utf8Slice`（`@demicodes/utils/bytes` 已有）。
- `packages/agent/src/tools.ts` 私有 `asRecord` / `optionalString`（utils 已有 `asRecord` / `asString`）。
- `packages/agent/src/transcript.ts` 与 `packages/agent/src/server.ts` 各一份 `safeStringify`。
- `packages/host-local/src/local-host.ts` 与 `local-store.ts` 各一份 `isNotFound`。
- 处置：统一并入 `@demicodes/utils`（`safeStringify`、`isNotFoundError`），随 §5.4 一并清理。

### P2-2 shell 实现卫生

- `environment.ts` `collectAborted` 计算 `snapshot` 后 `void snapshot`（死代码）；文件末行 `void (undefined as unknown as InterpreterContext)` import 保活 hack。
- `ForegroundProcess`（`environment-state.ts`）24 个裸字段，其中 ~15 个是输出记账计数器组合。
- 输出以 string 存储、字节数学反复整段重编码；`snapshotCommand` 每次轮询 fire-and-forget 写一次 artifact JSON。
- 处置：§5.4。

### P2-3 RESERVED_COMMAND_NAMES 口径随意

- `packages/shell/src/command.ts` 硬编码清单（有 cargo/docker/python，无 go/ruby/rustc）。
- 处置：改为推导式：`DEMI_PORTABLE_COMMANDS ∪ fork builtin 名 ∪ 少量显式黑名单`，随 §5.4。

### P2-4 其他

- `packages/core/src/index.ts` 首行"对照 Rust alloy-generated"陈旧注释。
- `transcript.ts` `boundText` 8k/8k 截断常数硬编码不可配。
- `repl/src/index.ts` 846 行单文件。
- 根仓库存在未提交的 just-bash submodule 指针前移（496c3d7 → ea06eb1），按提交规则应单独提交推送。

---

## 3. 关键设计裁定

改造前先固定三个方向性决定，避免各阶段实现时反复：

### 裁定 A：保留"shell 为中心的工具面"，增加双投影

shell-only 工具面（`shell_exec` + 注册命令）换来了统一审计、输出预算与命令可组合性，是项目的设计中心，保留。但对结构化输入强的场景（弱模型、heredoc 易错），提供**同一 `CommandSpec` 的第二投影：原生 AgentTool**。命令实现只有一份，投影方式由 embedder 选择。详见 §5.5。

### 裁定 B：Transcript 成为 patch 的唯一来源

服务器不再靠深比较反推 patch；`Transcript` 的每个变更方法直接产出 wire patch。协议 pre-1.0，直接演进 patch op 集合（不留兼容层，符合最终态原则）。详见 §5.1。

### 裁定 C：重试属于 agent 运行时，不属于 provider

provider 保持单发语义（分类错误码即可）；退避重试由 `ProviderTurnLoop` 统一实现，从而对所有 provider 生效且策略可配置。详见 §5.3。

---

## 4. 阶段总览与顺序

| 阶段 | 内容 | 前置 | 分支建议 |
| --- | --- | --- | --- |
| 1 | 转录热路径重构（P0-1） | 无 | `feat/transcript-patch-pipeline` |
| 2 | 恢复一致性 + 会话互斥（P0-2/3） | 无（与 1 并行可） | `fix/session-restore-and-takeover` |
| 3 | provider 重试层（P1-2） | 阶段 1（error 事件路径变动小，弱依赖） | `feat/turn-retry-policy` |
| 4 | shell 记账重构 + 卫生清理（P2-1/2/3） | 无 | `refactor/shell-stream-accounting` |
| 5 | 命令双投影（裁定 A） | 无 | `feat/command-native-tool-projection` |
| 6 | 估算与默认值修正（P1-1/3/4，P2-4 部分） | 阶段 1 | `fix/token-estimate-and-defaults` |
| 7 | agent-eval 落地 + just-bash 治理 | 阶段 1–6 后收益最大，可提前 | 按既有 `agent-evaluation-plan.md` |

每阶段独立分支、独立验收；阶段内检查点按仓库规则自动提交并推送。

---

## 5. 各阶段设计

### 5.1 阶段 1：转录热路径重构

目标：单个流式增量的处理成本 O(delta)，与历史长度无关；快照落盘频率与流式速率解耦。

设计：

1. **patch op 扩展**（`frames.ts`）：

   ```ts
   type TranscriptPatch =
     | { op: 'add'; path: ['blocks', number]; value: Block }
     | { op: 'remove'; path: ['blocks', number] }
     | { op: 'replace'; path: ['blocks', number]; value: Block }   // 新增：单块替换
     | { op: 'append_text'; path: ['blocks', number]; field: 'text'; delta: string } // 新增：流式追加
     | { op: 'replace'; path: ['blocks']; value: Block[] }          // 保留：全量重置
   ```

   `applyTranscriptPatches`（client 与 web-ui 共用）同步支持新 op。

2. **Transcript 产出 patch**：`Transcript` 内部维护 `revision: number` 与自上次取走以来的 patch journal；所有变更方法（push*/applyProviderEvent/completeToolCall/insertCompactionBoundary/splice 重写等）在变更的同时记录对应 patch。`text_delta`/`thinking_delta` 合并进上一个同类型块时记录 `append_text`。`executeRetry` 这类整段 splice 记录若干 remove/add，或直接一条全量 replace（块数大时更划算，阈值同现有 `canSpliceTail` 逻辑）。
3. **SessionEvent 改造**：`transcript_changed` 携带 `{ patches, revision }`，不再携带全量快照；需要全量的场景（open 时的 snapshot 帧、store 落盘）调用 `transcript.snapshot()` 按需生成。`server.ts` 删除 `diffTranscriptBlocks` 反推路径与 `lastTranscriptBlocks` 副本（`patch.ts` 的深比较仅保留给测试断言使用，或整体删除）。
4. **落盘节流**：`commitTranscript` 拆成两个职责：
   - `emitPatches()`：每次变更即时发（保持 UI 流式体验）；
   - `persistSnapshot()`：仅在回合边界（用户块入列、工具完成、回合结束、压缩完成、abort 记录）+ 流式期间按最大间隔（默认 1s，可配 `AgentSessionOptions.persistIntervalMs`）落盘；`dispose`/abort 强制 flush。
   落盘时的 `structuredClone` 保留（快照语义需要隔离），但频率从每 delta 降到每边界。
5. **一致性保障**：client 侧对 `revision` 做单调校验，检测到跳号（理论上不应发生，防御传输 bug）时主动请求全量 snapshot——为此在 ClientFrame 增加 `{ type: 'sync_transcript' }`，server 回 `transcript_snapshot`。

验收标准：

- 新增基准测试（`packages/agent/src/__tests__/`）：4,000 块历史 + 10,000 次 text_delta，处理时间与 500 块历史场景同数量级（线性差异 < 2×）。
- 现有 session/server/transport/web-ui 测试全绿；`patch.test.ts` 更新为新 op 语义。
- store 写次数断言：一次含 3 个工具调用的回合，`saveSnapshot` 调用次数 ≤ 边界数 + 流式间隔数。

### 5.2 阶段 2：恢复一致性与会话接管

**P0-2 修复**：重排 `server.ts open()` 的构建顺序为"先加载快照、后构建 harness 上下文"：

```
snapshot = await store.loadSnapshot()
state    = snapshot?.harnessName === agent.name ? structuredClone(snapshot.state) : agent.initialState()
harnessContext = { state, cwd }
host / commands / runtime 闭包全部基于这一个 state 对象
session  = snapshot ? AgentSession.fromSnapshot(...) : new AgentSession(...)
```

配套：`AgentSession` 构造签名中 `params.state` 改为**引用接管**（不再 `structuredClone`）——克隆职责上移到唯一调用方 `AgentServer`；`fromSnapshot` 同理。`AgentHarness.initialState()` 恢复为真正的工厂语义（每次调用返回新对象），`runtime.initialState` 不再闭包缓存。

**P0-3 修复（接管语义）**：`AgentServer` 维护 `activeSessions: Map<sessionId, AgentTransportBindingImpl>`。`open` 一个已活跃的 sessionId 时：

- 旧 binding 收到 `{ type: 'closed' }` 后被服务端关闭（session dispose → 快照 flush）；
- 新 binding 正常走恢复路径。

选择接管而非拒绝：断线重连是这一场景的主用例，旧连接大概率已死；拒绝会把用户锁在门外。接管过程对同一 store key 的写入天然串行（先 dispose flush，再 load）。

验收标准：

- 新测试：有状态 stub harness（State 非空）保存→恢复后，harness `host()` 闭包观察到的 state 与 `session.state()` 是同一对象、且为快照值。
- 新测试：两个 in-process client 先后 open 同一 sessionId，先者收到 closed，后者恢复出完整转录；store 中快照不交错。

### 5.3 阶段 3：回合级重试策略

位置：`ProviderTurnLoop.streamProviderOnce` 外层。

```ts
interface TurnRetryPolicy {
  maxAttempts: number        // 默认 4（1 次原始 + 3 次重试）
  baseDelayMs: number        // 默认 1_000，指数退避 ×2，满抖动
  maxDelayMs: number         // 默认 30_000
  retryableCodes: string[]   // 默认 ['rate_limit', 'overloaded']
}
```

规则：

1. 仅当**本次请求尚未产出任何转录块**（流开始前或首事件即 error）时静默重试；一旦已有部分内容落入转录，错误照常入转录并终结回合（避免内容重复），交由既有 retry/resume 人工路径。
2. 退避等待期间尊重 abort signal（`abortable(delay(...), signal)`）；`phase` 保持 `provider_streaming`，并通过 `SessionEvent` 新增 `{ type: 'retry_scheduled', attempt, delayMs, code }` 让 UI 呈现"限流中，第 n 次重试"。
3. 配置挂在 `AgentSessionOptions.retry`，`AgentServerOptions` 透传；provider 不感知。
4. `ProviderEvent` error 增加可选 `retryAfterMs`（provider 从 `Retry-After` 头解析，可不填）；策略优先采用它。

压缩摘要请求（`CompactionController.generateSummary`）复用同一策略（它同样是"未产出内容即可安全重试"的形态）。

验收标准：StubProvider 脚本化"两次 rate_limit 后成功"，断言最终转录无 error 块、重试事件按序发出、退避受 abort 打断。

### 5.4 阶段 4：shell 输出记账重构与卫生清理

1. **`StreamAccounting` 类型**（新文件 `packages/shell/src/stream-accounting.ts`）：封装单命令的 stdout/stderr 记账——raw/decoded 缓冲、字节计数、快照游标、chunk 序列。`ForegroundProcess` 的 ~15 个计数器字段收敛为 `accounting: StreamAccounting`；`environment-output.ts` 的散装函数（`recordForegroundChunk` / `snapshotFrom*`）改为其方法。字节长度增量维护（追加时累加），消除整段重编码。
2. **字符串 → 字节游标**：`ShellCommandRecord.stdout/stderr` 保存 decoded string 不变（对外契约不动），但记账内部以字节数为一等公民，`utf8Bytes` 只在边界调用一次。
3. **去重**：删除 `environment.ts` 底部的 `utf8Bytes`/`utf8Slice`，改用 utils；`safeStringify`、`isNotFoundError` 上收 utils 并替换四处私有实现；`agent/tools.ts` 改用 utils 的 `asRecord`/`asString`。
4. **死代码/hack**：删除 `collectAborted` 的 `void snapshot`；用 `import type` + 实际使用替代末行 `void (undefined as ...)`。
5. **artifact 落盘节流**：`snapshotCommand` 仅在内容自上次持久化后有变化时写 artifact（记账里带 dirty 标记）。
6. **保留名推导**：`RESERVED_COMMAND_NAMES` = `DEMI_PORTABLE_COMMANDS ∪ fork 内建名（从 createLazyCommands 元数据取）∪ 显式追加清单`，测试断言现清单是新集合的子集，防止行为回退。

验收标准：`environment.test.ts`（2,649 行）全绿不改语义断言；新增大输出基准（100MB 级 stdout 分块）时间上界测试。

### 5.5 阶段 5：CommandSpec → AgentTool 双投影

新增 `packages/shell/src/command-tool-projection.ts`（shell 拥有 CommandSpec，但 AgentTool 类型在 agent —— 为避免反向依赖，投影函数放 **agent 包**：`packages/agent/src/command-tools.ts`，它已依赖 shell）：

```ts
export interface CommandToolProjectionOptions {
  include?: string[]   // 叶路径白名单，如 ['editor create', 'todo add']
  exclude?: string[]
}
export function commandTreeToAgentTools(
  environment: BashEnvironment,
  options?: CommandToolProjectionOptions,
): AgentTool[]
```

- 每个叶子命令投影为一个工具：name = 路径下划线连接（`editor_create`）；`inputSchema` 用 zod v4 原生 `z.toJSONSchema` 从 `CommandInputSpec` 生成（含 stdinField 字段，描述标注"large content field"）。
- `invoke` 合成 argv + stdin 后走 `BashEnvironment.exec`（而非绕过 shell 直接 `runRegisteredCommand`），使审计、artifact、`/@/commands` 寻址、metadata 管道与 shell 路径完全一致——投影只是输入形态不同。
- 开关：`AgentServerOptions.commandTools?: CommandToolProjectionOptions | false`（默认 false，保持现状）；开启后这些工具与标准 shell 工具并列进入 `tools()`。
- 系统提示：开启投影时 `commandsPrompt` 中对应命令标注"也可作为原生工具调用"。

验收标准：同一 `editor create` 分别经 shell heredoc 与原生工具调用，产生等价的文件结果、审计事件与 diff metadata；边界测试确认无新增反向依赖。

### 5.6 阶段 6：估算与默认值修正

1. **上下文估算换锚**（`transcript.ts`）：`estimateContextTokens` 优先使用**最近一个 `response` 块的真实 usage**（`inputTokens + cacheReadTokens + cacheWriteTokens`）作为该点之前历史的锚，仅对该点之后的块做 chars/4 估算；无 response 块时退回全量估算。image 估算改为 `max(1600, bytes/750)`（binary）或固定 1600（url），document 按 `bytes/4`。
2. **anthropic thinking**：`effort` 型配置显式映射（能力表：effort → budget_tokens 档位，如 low=4k / medium=16k / high=64k，表放 `models.ts` 可被 caller 覆盖）；无法映射时 yield 明确 error 而非静默丢弃。`max_tokens` 默认 `model.outputLimit ?? 32_000`，`options.request.maxTokens` 仍可覆盖。
3. **LocalHost store 根目录**：默认迁至 `${XDG_DATA_HOME ?? ~/.local/share}/demi/host-local/<hash>`（Windows 用 `%LOCALAPPDATA%`），`storeRoot` 选项不变。不做旧目录迁移（tmpdir 数据本就易失，符合"不留兼容路径"原则），CHANGELOG 说明。
4. **杂项**：删 core 首行陈旧注释；`boundText` 常数提升为 `TranscriptOptions` 可选项；`repl/index.ts` 拆分（`options.ts` / `render.ts` / `input-loop.ts` / `providers.ts`，无行为变化）。

### 5.7 阶段 7a：agent-eval 落地

按既有 `docs/internal/agent-evaluation-plan.md` 与 `package-boundaries.md` 中 `@demicodes/agent-eval` 的注册表条目执行，不在本文重复设计。强调两点与本方案的衔接：

- 基准需覆盖本项目的护城河语义：yield 唤醒续作、steer 双路径、auto-recover 压缩、重试策略（阶段 3 之后）。
- 双投影（阶段 5）落地后，同一任务集在"纯 shell 面"与"shell+原生工具面"两种配置下跑分，用数据回答裁定 A 的默认值应该是什么。

### 5.8 阶段 7b：just-bash fork 治理

1. 新增 `docs/just-bash-fork-policy.md`（`docs/internal/` 不入库，治理策略需要入库）：记录 fork 点、自有提交清单、上游 remote、同步节奏（建议按需 + 每季度评估）、冲突处理原则（fork 只做"暴露内部接口/裁剪产物"类改动，不改语义，降低合并成本）。
2. 裁剪评估：`src/commands` 91 个命令 5.1MB，`DEMI_PORTABLE_COMMANDS` 只启用 ~55 个；评估将未启用命令从构建产物剔除（保留源码，靠 `createLazyCommands` 的按需加载确认 tree-shaking 是否已足够——若已足够则只需文档说明，不动代码）。
3. 将当前未提交的 submodule 指针前移（496c3d7 → ea06eb1）按仓库规则在独立提交中落账。

---

## 6. 不做的事（明确排除）

- 不引入独立的 read/write/grep 原生工具集（与裁定 A 冲突；双投影已覆盖结构化输入需求）。
- 不给 provider 层加重试（裁定 C）。
- 不为 tmpdir → XDG 迁移写数据搬迁逻辑（不留兼容路径）。
- 不在本轮改 wire 协议的传输编码（JSON 帧维持现状；patch op 演进除外）。
- 不动 `platform-entrypoints` 边界测试的机制（各阶段只按需补充断言）。

---

## 7. 风险与回滚

| 风险 | 缓解 |
| --- | --- |
| 阶段 1 改动 patch 协议，web-ui/repl 渲染回归 | client 侧 `revision` 校验 + `sync_transcript` 全量兜底；web-ui 测试覆盖 append_text |
| 阶段 2 state 引用接管改变克隆语义，隐性共享 | 克隆职责集中在 AgentServer 单点；session 测试补"外部修改传入 state 不影响会话"反向断言删除说明 |
| 阶段 3 重试掩盖真实故障 | 重试事件全部可观测（SessionEvent + UI）；attempts 上限保守 |
| 阶段 4 记账重构引入输出边界 off-by-one | environment.test.ts 断言保持不变作为金标准；先加基准再动实现 |
| 阶段 5 工具名冲突（如 harness 自带同名工具） | 投影前检查与现有 tools 名字冲突，冲突即抛错 |

每阶段独立分支 + 独立验收，任一阶段可单独放弃回滚，不影响其余阶段。

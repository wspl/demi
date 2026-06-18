# 测试覆盖矩阵

| | |
|---|---|
| 日期 | 2026-06-19 |
| 状态 | 草案 |
| 范围 | demi agent、shell、provider、RPC、coding agent、TUI |

## 1. 状态定义

- 已覆盖：默认自动化测试直接验证该测试点。
- 部分覆盖：已有测试碰到该测试点，但断言不完整，或只覆盖简单路径。
- 未覆盖：没有有效自动化测试。
- 手动/Gated：不进入默认测试，依赖真实 CLI、真实模型、网络、本机登录状态或交互 UI。

测试点必须说明它能发现或规避的问题。没有明确风险的测试点不应进入矩阵。

## 2. 测试判定原则

- 当前产品目标是最小但长期稳定的 agent runtime，不是完整 IDE/GUI agent 产品面。
- 核心链路是 AgentSession 状态机、Transcript replay、Provider 可见上下文、Tool call/result、Shell 控制面、Compaction、Context cache/usage、RPC/TUI 事件呈现。
- Compaction 是长任务能力的核心路径，必须作为 P0 测试面；它要证明上下文接近上限时 agent 能继续工作，并且不切坏 tool pair、不重复执行工具、不污染失败状态。
- Context cache 是基线稳定性要求；即使不主动管理 provider cache，也必须保证模型可见上下文稳定、usage 指标不丢、compact 后上下文能重新稳定。
- 参考项目只用于校准核心稳定性标准，不作为功能照抄清单。
- 权限、审批、分享、revert、项目管理等能力是当前有意不做的产品面，不进入缺口矩阵；如果以后进入产品范围，再新增对应测试模块。

## 3. 参考实现校准

| 来源 | 对 demi 测试的影响 |
|---|---|
| agent-gui / Rust agent | 重点吸收 session marathon、compact long-conversation stories、tool pair invariant、abort/retry/resume/queue 组合压力测试。 |
| Codex | 重点吸收模型可见上下文稳定、无界内容截断、cache prefix 稳定、compaction 后历史重建正确性。 |
| pi agent | 重点吸收完整日志与模型上下文分离、compact 后仍保留可审计历史、provider conformance 的异常输入覆盖。 |
| opencode | 只吸收 session/compact/abort 这类通用 runtime 语义；permission/share/revert 等产品面不进入当前范围。 |

### 3.1 最小稳定 agent 验收门槛

| 门槛 | 必须证明 | 主要证据 |
|---|---|---|
| AgentSession 长生命周期稳定 | 单个 session 经过 send/queue/retry/resume/abort/tool/error/compact 后，phase、pending action、transcript、provider request 仍一致 | `base-agent` 单会话 marathon，关键步骤 exact provider request，统一 transcript invariant helper |
| 模型可见上下文稳定 | provider request 只来自 effective transcript 和当前 prompt context；内部块不泄漏；普通多轮 prefix 不无意义抖动 | `Transcript.collectInferenceItems` 复杂 fixture，连续 turn request prefix 比较，bounded injection 测试 |
| Compaction 可支撑长任务 | preflight/manual/auto compact 都可恢复；summary request 契约正确；不切 tool pair；失败/空 summary 不污染 transcript；多次 compact 只 replay 最新 boundary | `base-agent` compact P0 套件，覆盖 summary request、exact replay、failure atomicity、multi-compact、queue/persistence |
| Context cache baseline | cache usage 不丢；cache 只作为 provider 透明优化不改变 agent 行为；compact 后 request prefix 能重新稳定 | provider usage 映射、AgentSession/RPC usage 传播、compact 后 request 稳定性测试 |
| Shell 控制面支撑真实长命令 | 长进程走 foreground，wait/input/abort 语义稳定；不恢复空 input 轮询；不误报 needs_input | `shell` 单测、`agent-coding` 长命令 scenario、真实 provider/TUI gated smoke |
| Coding workflow 能发现真实问题 | 不是只验证单步命令，而是覆盖创建项目、测试失败、读取错误、修复、测试通过、tool error recovery | `agent-coding` scenario tests，断言真实文件、todo、shell result、provider request |
| 壳子路径能呈现真实模型行为 | TUI/RPC 能显示真实 text/thinking/tool output，真实 Claude Code provider 确实产出模型回复 | RPC transport tests，TUI 自动化或 gated smoke |

## 4. 默认测试入口

- `bun run typecheck`：类型检查。
- `bun run test`：主仓库默认自动化测试。
- `bun run test:just-bash-core`：demi 依赖的 just-bash 核心解析保护测试。
- `bun run check:registry`：public registry / package boundary 检查。

## 5. 模块测试点

### 5.1 平台与包边界

Owner：`packages/core`

| 测试点 | 状态 | 现有覆盖 / 待补 | 能发现或规避的问题 |
|---|---|---|---|
| 根入口保持 browser-safe / runtime-neutral，不静态依赖 Node-only 能力 | 已覆盖 | `packages/core/src/__tests__/platform-entrypoints.test.ts` | 防止浏览器或非 Node runtime 导入根包时因为 `node:*`、`process`、`Buffer` 直接崩溃。 |
| 只有 RPC host 在运行时代码中直接实例化 `AgentSession` | 已覆盖 | `platform-entrypoints.test.ts` | 防止 UI、provider 或业务包绕过协议层调用 agent runtime，导致状态和事件边界失控。 |
| package manifest 不引入越层依赖 | 已覆盖 | `platform-entrypoints.test.ts` | 防止平台无关包通过依赖声明暗中耦合 Node adapter、真实 provider 或 UI 包。 |
| 主仓库使用 forked just-bash package，不维护第二份上游源码快照 | 已覆盖 | `platform-entrypoints.test.ts` | 防止 bash engine 出现两个来源，造成修复只改一份、运行消费另一份的分叉问题。 |

### 5.2 Provider 抽象

Owner：`packages/provider`

| 测试点 | 状态 | 现有覆盖 / 待补 | 能发现或规避的问题 |
|---|---|---|---|
| `ProviderRegistry` 注册、查找、创建 provider | 已覆盖 | `packages/provider/src/__tests__/registry.test.ts` | 防止 provider id/type 解析错位，导致 RPC 打开的 provider 不是用户配置的 provider。 |
| `ProviderRegistry` registration snapshot 通知 | 已覆盖 | `registry.test.ts` | 防止 UI 或 host 看到过期 provider 列表，造成模型选择器和实际能力不一致。 |
| `StubProvider` 多轮 scripted events | 已覆盖 | `packages/provider/src/__tests__/stub.test.ts` | 防止 agent scenario test 依赖真实模型随机性，保证状态机测试可复现。 |
| `StubProvider` function script 能读取 request 并驱动 tool roundtrip | 已覆盖 | `stub.test.ts` | 能发现 provider request 内容错误、tool_result 没有回灌、下一轮上下文不正确等问题。 |
| `StubProvider` 脚本耗尽时报错 | 已覆盖 | `stub.test.ts` | 防止测试悄悄多跑一轮但仍然通过，暴露意外 retry/resume/loop。 |

### 5.3 Claude Code Provider

Owner：`packages/provider-claude-code`

| 测试点 | 状态 | 现有覆盖 / 待补 | 能发现或规避的问题 |
|---|---|---|---|
| Claude CLI path/version 检测 | 已覆盖 | `packages/provider-claude-code/src/__tests__/cli.test.ts` | 防止 provider 在没有可用 CLI 或版本信息异常时进入半可用状态。 |
| Claude auth/runtime state 读取，不依赖 SDK | 已覆盖 | `cli.test.ts` | 防止认证状态判断依赖不可控 SDK 行为，导致 UI 误报可用或不可用。 |
| Claude CLI args/env 构造契约 | 已覆盖 | `cli.test.ts` | 防止模型、thinking、MCP/config 等关键启动参数在 CLI 调用时丢失或拼错。 |
| demi inference items 转 Claude JSONL input messages | 已覆盖 | `packages/provider-claude-code/src/__tests__/jsonl-output.test.ts` | 防止 user/assistant/tool 历史转换错位，真实模型看到的上下文与 transcript 不一致。 |
| assistant turn 与 tool result grouping | 已覆盖 | `jsonl-output.test.ts` | 防止 Claude 所需的 assistant/tool_result 顺序被拆乱，引发 tool_use 无匹配结果。 |
| binary media content 转 Claude base64 source | 已覆盖 | `jsonl-output.test.ts` | 防止图片或二进制输入在 provider 边界损坏。 |
| Claude stdout 映射 text/thinking/tool call/usage | 已覆盖 | `jsonl-output.test.ts` | 防止真实模型输出被吞、thinking 不显示、tool_call 不触发或 usage 统计失真。 |
| cache read/write usage 字段解析 | 已覆盖 | `jsonl-output.test.ts` 只覆盖 usage 字段映射 | 防止 provider 已返回 cache 指标但 demi 丢字段，影响后续 cache contract 判断。 |
| malformed assistant tool_use / control_request 报错并不污染 pending state | 已覆盖 | `jsonl-output.test.ts`、`provider.test.ts` | 防止异常 provider 消息让 session 永久等待不存在的 tool_result。 |
| control_request、SDK MCP control_request、assistant tool_use 跨 run 状态机 | 已覆盖 | `packages/provider-claude-code/src/__tests__/provider.test.ts` | 防止 Claude Code 多轮工具协议在 run 边界丢失 pending call 或重复提交结果。 |
| 缺失 tool_result / pending control_request 收敛为 provider error | 已覆盖 | `provider.test.ts` | 防止协议断裂时 agent 卡死或继续用不完整上下文调用模型。 |
| CLI 非零退出、stdout 迭代失败、abort 的 transport 清理 | 已覆盖 | `provider.test.ts` | 防止 CLI 进程泄漏、后续 run 复用坏 transport，或错误无法传回 session。 |
| 与 `AgentSession` 和 shell tools 的 provider 集成 | 已覆盖 | `provider.test.ts` | 能发现 provider event、AgentSession tool loop、shell tool result 三者之间的接口不匹配。 |
| 真实 Claude CLI e2e | 手动/Gated | `real-cli.e2e.test.ts`，不属于默认稳定测试 | 用来发现 fake transport 无法覆盖的本机 CLI、账号、网络、真实输出格式变化。 |
| 真实模型 thinking/tool use/text 输出验收 | 手动/Gated | 需要 TUI/CLI smoke 流程 | 用来确认最终用户路径确实看到真实模型回复、thinking 和 tooluse，而不是只验证 mock。 |

### 5.4 Agent Session Runtime

Owner：`packages/base-agent`

| 测试点 | 状态 | 现有覆盖 / 待补 | 能发现或规避的问题 |
|---|---|---|---|
| `send` 写入 user turn，再构造 provider request | 已覆盖 | `packages/base-agent/src/__tests__/session.test.ts` | 防止 provider request 漏掉用户输入、preamble 顺序错误，或 transcript 与模型上下文分叉。 |
| provider response 写入 transcript | 已覆盖 | `session.test.ts` | 防止 UI/RPC 无法看到模型输出，或 retry/compact 基于缺失历史运行。 |
| provider error 写入 transcript、发 event、reject action | 已覆盖 | `session.test.ts` | 防止错误只在 Promise 中出现而 transcript/UI 不可见，或者 action 静默成功。 |
| provider error 后关闭 provider iterator | 已覆盖 | `session.test.ts` | 防止 provider stream 泄漏，后续 run 收到旧 stream 的残留事件。 |
| reference resolution 在写入 user turn 和 provider request 前执行 | 已覆盖 | `session.test.ts` | 防止 transcript 保存未展开引用，导致恢复或重放时上下文不可复现。 |
| reference resolution 卡住时 abort 可收敛 | 已覆盖 | `session.test.ts` | 防止文件引用或外部解析卡住后 session 无法取消。 |
| provider 不 yield 时 abort 可收敛 | 已覆盖 | `session.test.ts` | 防止模型 stream 卡死时 UI 的停止按钮无效。 |
| long-running tool invocation 卡住时 abort 可收敛 | 已覆盖 | `session.test.ts` | 防止工具实现挂起后占住 session，后续用户输入无法执行。 |
| tool call 执行后继续 provider roundtrip | 已覆盖 | `session.test.ts` | 防止 tool_result 没有回传给模型，模型无法基于工具结果继续回答。 |
| tool invocation throw 转成 error tool result 并继续 | 已覆盖 | `session.test.ts` | 防止工具异常升级成 session 崩溃，模型失去恢复机会。 |
| tool progress event 发出 | 已覆盖 | `session.test.ts` | 防止 shell output/audit 等进度事件只留在内部，UI 无法实时显示。 |
| queued send 排队并按顺序 drain | 已覆盖 | `session.test.ts` | 防止用户连续发送消息时顺序错乱，或后一条消息覆盖前一条运行状态。 |
| retry 截断最后 assistant response 并 rerun latest user turn | 已覆盖 | `session.test.ts` | 防止 retry 把旧错误输出和新输出混在一起，或重跑了错误的用户 turn。 |
| resume 标记 abort 为 resumed，并追加 resume turn | 已覆盖 | `session.test.ts` | 防止恢复后 transcript 无法区分已中止内容和继续执行内容。 |
| abort 后 resume 前清理 pending tool calls | 已覆盖 | `session.test.ts` | 防止恢复时模型看到仍在执行的旧 tool_call，重复等待或重复执行工具。 |
| mutation guard 拒绝 busy/reserved 期间 mutation | 已覆盖 | `session.test.ts` | 防止 compact/retry/resume 与 active run 交错修改 transcript，造成不可恢复状态。 |
| store snapshot 写入 | 已覆盖 | `session.test.ts` | 防止进程退出或 host 重启后没有可恢复的会话状态。 |
| extension state snapshot 通过 lifecycle 写入并持久化 | 已覆盖 | `session.test.ts` | 防止 todo 等 agent 扩展状态只在内存里存在，恢复后状态丢失。 |
| 从 store snapshot 重建 session 后继续运行 | 未覆盖 | 需要 persistence/recovery 测试 | 需要发现 snapshot schema、idFactory、phase 或 transcript replay 在重启后不兼容的问题。 |
| provider error 后恢复不重复发送已完成 tool result | 未覆盖 | 需要恢复场景测试 | 需要规避重启后重复执行破坏性工具或给模型重复上下文。 |
| abort/retry/resume/compact 组合交错 | 部分覆盖 | 单点已测，组合路径不足 | 组合路径容易暴露单点测试发现不了的 phase、queue、transcript 原子性问题。 |
| 单会话 marathon 覆盖 send/queue/retry/resume/abort/tool/error/compact 累计状态 | 未覆盖 | 需要一个长生命周期 StubProvider 场景 | 用来发现状态只在单点测试里正确，长期累计后 id、phase、pending action、tool 状态或 transcript 顺序漂移。 |
| 每个关键步骤精确断言 provider request | 部分覆盖 | 简单 send/tool 有断言，缺少 marathon 全程断言 | 防止模型实际看到的上下文与 transcript 看起来正确但不同，尤其是 compact、retry、resume 后的重放内容。 |
| transcript 结构不变量集中校验 | 部分覆盖 | 分散在单测断言里，缺少通用 invariant helper | 防止出现重复 block id、缺 createdAt、turn 未以 user/resume 开始、terminal block 后仍追加内容、completed turn 里 tool_call 缺 output。 |
| provider mock 行为贴近真实 provider 事件顺序 | 部分覆盖 | tool roundtrip 已覆盖，缺少 pending tool + response + auto compact 的组合 | 防止 deterministic 测试用一个过于理想的 provider，真实 Claude Code 路径才暴露 tool/response 时序问题。 |

### 5.5 Transcript 与 Replay

Owner：`packages/base-agent`

| 测试点 | 状态 | 现有覆盖 / 待补 | 能发现或规避的问题 |
|---|---|---|---|
| user/text/response blocks 转 inference items | 已覆盖 | `packages/base-agent/src/__tests__/transcript.test.ts` | 防止 transcript 能显示但不能正确喂回 provider。 |
| pending tool call 发现与完成 | 已覆盖 | `transcript.test.ts` | 防止 tool_call 状态卡在 executing，后续 run 误判还有未完成工具。 |
| completed tool call 转 `tool_use` + `tool_result` | 已覆盖 | `transcript.test.ts` | 防止模型下一轮看不到工具输入或工具输出，导致重复调用工具。 |
| non-JSON tool input 安全存储和 replay | 已覆盖 | `transcript.test.ts` | 防止异常输入对象让 transcript 序列化崩溃或 replay 失败。 |
| dangling executing tool call 清理 | 已覆盖 | `transcript.test.ts` | 防止 abort/restart 后遗留 executing tool 阻塞 resume。 |
| extension state snapshot latest 查询 | 已覆盖 | `transcript.test.ts` | 防止恢复 agent 扩展状态时拿到过期 snapshot。 |
| non-JSON extension state token estimate | 已覆盖 | `transcript.test.ts` | 防止 token estimate 被 BigInt/cycle 等扩展状态打断，影响 compact 判断。 |
| transcript snapshot 序列化/反序列化等价 | 部分覆盖 | 有 store snapshot 写入测试，缺少重建后 replay 等价测试 | 需要发现保存后再加载丢 block、丢 metadata 或改变 replay 内容的问题。 |
| provider request exact replay 内容 | 部分覆盖 | 简单 send 覆盖，复杂 transcript 未覆盖 | 需要发现复杂历史、tool、extension、compact 混合时喂给模型的上下文漂移。 |
| effective replay 只包含模型应看到的 block | 部分覆盖 | `collectInferenceItems` 对 marker/extension 没有输出，缺少完整 fixture | 防止 compaction marker、extension snapshot、internal error 状态等内部块进入 provider request。 |
| replay 保持 tool_use/tool_result 成对且顺序正确 | 部分覆盖 | 简单 tool pair 已测，缺少 compact/retry/resume 后的复杂历史 | 防止 provider 收到孤立 tool_result、孤立 tool_use 或乱序工具历史。 |
| replay 中 thinking/redacted thinking 能跨 provider 边界保留 | 部分覆盖 | provider JSONL 有 thinking 映射，transcript 复杂 replay 不足 | 防止开启 thinking 的真实模型路径在重放或 compact 后丢失签名、redacted thinking 或顺序。 |

### 5.6 Compaction

Owner：`packages/base-agent`

| 测试点 | 状态 | 现有覆盖 / 待补 | 能发现或规避的问题 |
|---|---|---|---|
| 手动 compact 插入 boundary 和 marker | 已覆盖 | `session.test.ts` | 防止 compact 完成后 transcript 无法标识历史摘要边界。 |
| compact 不删除旧 blocks | 已覆盖 | `session.test.ts` | 防止 compact 破坏审计历史，影响 UI 查看完整 transcript。 |
| replay 从 latest compaction boundary 开始 | 已覆盖 | `transcript.test.ts` | 防止 compact 后仍把旧长历史发给 provider，失去压缩意义。 |
| boundary summary 转成下一次 inference user message | 已覆盖 | `transcript.test.ts` | 防止模型看不到摘要，只收到被截断的 recent context。 |
| usage 接近 context limit 时自动 compact + resume | 已覆盖 | `session.test.ts` | 防止长会话到上下文边界后直接失败，而不是自动恢复。 |
| compaction summary provider 卡住时 abort 可收敛 | 已覆盖 | `session.test.ts` | 防止 compact 阶段卡死导致用户无法停止 session。 |
| preflight compact 在新 provider request 前发生 | 未覆盖 | 需要 send 前 token 阈值场景 | 防止新用户消息已经把上下文推过上限后才请求模型，导致真实 provider 直接 context overflow。 |
| compaction summary request 的模型、thinking、cwd、tools 与契约一致 | 未覆盖 | 需要捕获 summary provider request | 防止 summary 用错模型/思考等级、错误 cwd，或把普通工具暴露给总结请求。 |
| cut point 不能切断 `tool_use -> tool_result` | 未覆盖 | 需要 transcript cut-point invariant 测试 | 需要防止模型看到孤立 tool_use 或孤立 tool_result，引发 provider 协议错误。 |
| compact 后下一次 provider request 精确等于 summary + recent context | 未覆盖 | 需要 exact request 测试 | 需要发现 summary、recent blocks、preamble 或 tool history 被漏放、重复放、乱序放。 |
| 多次 compact 只 replay latest boundary，不重复旧 summary | 未覆盖 | 需要 multi-compact 测试 | 需要规避摘要套摘要无限膨胀，或旧历史重新进入上下文。 |
| summary provider error/abort 不留下半截 boundary/marker | 部分覆盖 | abort 收敛已测，transcript 原子性未完整断言 | 需要防止 compact 失败后 transcript 处于既不像旧状态也不像新状态的中间态。 |
| empty summary 不插入 boundary/marker，且 session 可继续 | 未覆盖 | 需要 summary provider 返回空文本场景 | 防止空摘要把旧历史替换成无信息 boundary，后续模型失去任务上下文。 |
| auto compact + resume 不重复执行已完成 tool call | 未覆盖 | 需要 tool + compact 场景测试 | 需要防止上下文压缩后把已完成工具当成待执行工具再次运行。 |
| compact 期间 queued send 能按序 drain | 未覆盖 | 需要 compacting phase 中排队 user message 场景 | 防止长 summary 期间用户继续输入后消息丢失、乱序，或 compact 完成后没有继续处理。 |
| compact 后持久化再恢复，replayable blocks 保持一致 | 未覆盖 | 需要 persistence roundtrip 测试 | 需要发现 boundary/marker 在 snapshot 中丢失或恢复后 replay 起点错误。 |
| thinking/redacted thinking/extension state/tool metadata 混合 transcript 下 cut point 正确 | 未覆盖 | 需要复杂 transcript fixture | 需要防止非文本 block 或扩展状态让 compact 切点算法误判。 |

### 5.7 模型可见上下文与 Context Cache

Owner：`packages/provider-claude-code`、`packages/base-agent`

| 测试点 | 状态 | 现有覆盖 / 待补 | 能发现或规避的问题 |
|---|---|---|---|
| provider request prefix 在普通多轮对话中稳定 | 未覆盖 | 需要连续 turn 捕获 request 并做 prefix 比较 | 防止 system prompt、tools schema、preamble 或历史 items 无意义重排，破坏 context cache 基线。 |
| provider request 只由 effective transcript 和当前 prompt context 构成 | 部分覆盖 | 简单 request 已测，缺少复杂历史 | 防止 store snapshot、extension state、compaction marker、UI-only 状态进入模型可见上下文。 |
| 注入内容有明确上限和截断策略 | 部分覆盖 | shell output limit 已覆盖，ref/preamble/tool result 通用上限未覆盖 | 防止大文件引用、大工具输出或过长 preamble 直接撑爆上下文，compact 也来不及恢复。 |
| provider usage 中 cache read/write token 字段被解析 | 已覆盖 | `jsonl-output.test.ts` | 防止 provider cache 指标被丢弃，后续无法判断真实 cache 行为。 |
| cache usage 被 AgentSession 记录并对外暴露 | 部分覆盖 | response usage 记录路径存在，缺少专门 cache usage 断言 | 需要发现 usage 在 provider 到 session 到 UI/RPC 链路中丢字段。 |
| cache 只是 provider 透明优化时，不影响 agent 行为 | 未覆盖 | 需要定义 contract 后测试 | 需要保证 cache 指标变化不会改变 transcript、tool loop 或错误处理。 |
| demi 主动保障 stable prompt prefix 时，跨 turn prefix 字节级稳定 | 未覆盖 | 需要定义并断言 stable prefix contract | 如果要主动利用 cache，该测试能发现无意义重排 tools/system prompt 破坏命中率。 |
| tools/schema/system prompt/model/preamble 改变时 cache 失效规则 | 未覆盖 | 需要先定义 contract | 需要防止 cache 命中建立在错误前缀上，或该失效时没有失效。 |
| compact 后 cache prefix 变化与重新稳定 | 未覆盖 | 需要 compact + cache contract 测试 | 需要发现 compact 后上下文前缀持续抖动，导致 cache 永远无法稳定命中。 |
| 真实 provider cache 命中 | 手动/Gated | 只能做真实 provider smoke，不进默认 deterministic 测试 | 用来确认 deterministic contract 之外的真实 CLI/服务端 cache 行为没有退化。 |

### 5.8 Command Registry

Owner：`packages/shell`

| 测试点 | 状态 | 现有覆盖 / 待补 | 能发现或规避的问题 |
|---|---|---|---|
| positionals、flags、stdin fields 解析 | 已覆盖 | `packages/shell/src/__tests__/command.test.ts` | 防止模型调用注册命令时参数被错配到错误字段。 |
| long options 校验与数字 coercion | 已覆盖 | `command.test.ts` | 防止字符串数字、非法 option 进入命令实现造成隐式行为。 |
| `--json`、boolean、repeated array options | 已覆盖 | `command.test.ts` | 防止 agent 依赖结构化输出时拿到 raw text 或数组/布尔解析错。 |
| unknown options / invalid values 拒绝 | 已覆盖 | `command.test.ts` | 防止模型拼错参数却被静默忽略，产生看似成功的错误操作。 |
| `CommandSpec` 作为 prompt/help 单一来源 | 已覆盖 | `command.test.ts` | 防止 system prompt、`prompt` 子命令和实际 parser 三套说明漂移。 |
| registry 注册命令并渲染 prompt | 已覆盖 | `command.test.ts` | 防止新增命令未进入 agent 可见能力列表。 |
| 注册命令名不能复用 shell/system reserved names | 已覆盖 | `command.test.ts` | 防止 agent 命令遮蔽常见系统命令或 shell builtin，破坏用户预期。 |
| `<command> prompt` 使用同一个 renderer | 已覆盖 | `command.test.ts` | 防止模型通过 help 学到的调用方式与 system prompt 不一致。 |
| JSON mode output schema 校验 | 已覆盖 | `command.test.ts` | 防止注册命令声称 JSON 输出但返回不可解析或结构错误的数据。 |
| 无 JSON schema 的 subcommand 拒绝 JSON mode | 已覆盖 | `command.test.ts` | 防止调用方误以为某命令有结构化输出而继续自动处理。 |

### 5.9 Bash Environment 语义

Owner：`packages/shell`

| 测试点 | 状态 | 现有覆盖 / 待补 | 能发现或规避的问题 |
|---|---|---|---|
| cwd/env/last status 跨 `shell_exec` 保持 | 已覆盖 | `packages/shell/src/__tests__/environment.test.ts` | 防止 shell session 退化成一次性命令执行，模型后续命令跑在错误目录或环境。 |
| stateful builtins 在当前 session 生效且非法参数不污染状态 | 已覆盖 | `environment.test.ts` | 防止 `cd/export/unset` 等失败后留下半修改状态。 |
| `read`、stdin、heredoc、here-string 进入当前 shell 语义 | 已覆盖 | `environment.test.ts` | 防止交互式或 heredoc 命令的输入被错误丢弃或写到错误命令。 |
| list operators、prefix assignments、assignment-only commands | 已覆盖 | `environment.test.ts` | 防止常见 shell 语法被错误 fallback 到系统 shell 或执行顺序错误。 |
| parameter/arithmetic/command substitution expansion | 已覆盖 | `environment.test.ts` | 防止变量展开和命令替换与 bash 语义偏离，导致 agent 命令读写错误路径或值。 |
| functions、local、return、function redirection | 已覆盖 | `environment.test.ts` | 防止 sourced script 或函数调用污染外层 scope，或 redirection 应用时机错误。 |
| background jobs、jobs wait、spawn failure | 已覆盖 | `environment.test.ts` | 防止后台任务丢失、无法等待，或 spawn 失败导致 shell 挂死。 |
| `source`、`set`、`shift`、`eval`、`type`、`command` builtin | 已覆盖 | `environment.test.ts` | 防止这些会改变 session 或 introspection 的 builtin 被错误交给系统命令。 |
| registered command / builtin / function / system command 调度顺序 | 已覆盖 | `environment.test.ts` | 防止 agent 注册能力被 shell function 遮蔽，或 `command` 绕过状态 builtin。 |
| glob、redirection、pipeline、compound command、subshell | 已覆盖 | `environment.test.ts` | 防止常见脚本组合在 agent shell 中与真实 shell 行为严重不一致。 |
| `if`、`[[ ]]`、`case`、for/while/until、group、break/continue | 已覆盖 | `environment.test.ts` | 防止控制流判断错误，尤其是脚本自动化里跳错分支或循环无法退出。 |
| explicit `exit` 不被 negation/control flow 改写 | 已覆盖 | `environment.test.ts` | 防止用户脚本已经退出但 runner 继续执行后续命令。 |
| unsupported parser constructs 明确拒绝，不整段交给系统 shell | 已覆盖 | `environment.test.ts` | 防止逃过 audit/registered command/Host 抽象，直接用系统 shell 执行不可控脚本。 |
| system command audit events 和 spawn failure | 已覆盖 | `environment.test.ts` | 防止 UI/RPC 审计缺失，或命令不存在时工具调用挂死。 |
| 更完整的 bash 兼容性 spec | 部分覆盖 | 主仓库覆盖关键 agent 语义；更完整 spec 在 just-bash 子模块 | 用来发现主仓库关键路径之外的 bash 兼容性回归。 |

### 5.10 Shell 控制面

Owner：`packages/shell`

| 测试点 | 状态 | 现有覆盖 / 待补 | 能发现或规避的问题 |
|---|---|---|---|
| `shell_exec` running/yield 后可 `shell_wait` | 已覆盖 | `environment.test.ts`、`packages/shell/src/__tests__/tools.test.ts` | 防止长命令超出单次等待后失联，模型无法继续观察输出。 |
| output limit 触发 running output_limit | 已覆盖 | `environment.test.ts` | 防止大输出撑爆 tool result 或 transcript。 |
| `shell_input` 可向 foreground process 写 stdin | 已覆盖 | `environment.test.ts`、`tools.test.ts` | 防止交互式命令无法继续，或输入写入错误 shell。 |
| idle foreground process 默认保持 running，不误报 needs_input | 已覆盖 | `environment.test.ts` | 防止安静安装、构建、dev server 被误判为需要用户输入。 |
| `shell_wait` 的 `yieldAfterMs` 从每次调用重新计时 | 已覆盖 | `environment.test.ts` | 防止 wait 因累计时间错误立刻返回，模型反复空轮询。 |
| `shell_abort` 终止 foreground process，且是 control action | 已覆盖 | `environment.test.ts`、`tools.test.ts` | 防止停止 dev server 被当作任务失败，或进程继续占端口。 |
| abort/timeout flush redirected foreground output | 已覆盖 | `environment.test.ts` | 防止本应写入文件的输出泄漏到 tool output，或文件内容丢失。 |
| dispose shell kill foreground/background 并移除 session | 已覆盖 | `environment.test.ts` | 防止关闭 session 后仍有子进程残留。 |
| timeout kill foreground process，之后 shell 可复用 | 已覆盖 | `environment.test.ts` | 防止 timeout 后 abort state 泄漏，导致下一条命令立即失败。 |
| shell tool result 格式保留 metadata 且模型可读 | 已覆盖 | `tools.test.ts` | 防止模型必须解析不稳定 JSON 或看不到 shellId/status/next action。 |
| `shell_input` 拒绝空 stdin，不承担 polling | 已覆盖 | `tools.test.ts` | 防止空 input 被滥用成 wait，重新引入无意义控制动作。 |
| AgentSession abort signal 传播到 `shell_exec` / `shell_wait` / `shell_input` | 已覆盖 | `tools.test.ts` | 防止 UI abort 只停止 session 状态，不停止实际前台进程。 |
| 模型在真实长进程场景中稳定选择 wait/input/abort | 手动/Gated | 需要真实 provider/TUI smoke 多次验证 | 用来发现 prompt/tool result 虽然单测正确，但真实模型仍然误用控制面的问题。 |

### 5.11 Host、FS 与 Store

Owner：`packages/shell`

| 测试点 | 状态 | 现有覆盖 / 待补 | 能发现或规避的问题 |
|---|---|---|---|
| `HostBackedFileSystem` 通过 `Host.spawn` 完成 read/exists/stat/write/append/readdir | 已覆盖 | `packages/shell/src/__tests__/host-fs.test.ts` | 防止 shell/coding 绕过 Host 直接读写本机 fs，破坏远程或容器后端边界。 |
| readFileBuffer 返回 raw bytes | 已覆盖 | `host-fs.test.ts` | 防止二进制文件被文本编码损坏。 |
| `LocalHost` spawn capture stdout 和 stdin | 已覆盖 | `packages/shell/src/__tests__/local-host.test.ts` | 防止本地 adapter 不能正确连接进程输入输出。 |
| `LocalHost` terminate foreground process | 已覆盖 | `local-host.test.ts` | 防止 shell abort/timeout 在本地进程层失效。 |
| `AgentSessionCommandStorage` 按 agent session id prefix 隔离 keys | 已覆盖 | `packages/shell/src/__tests__/store.test.ts` | 防止多个 agent session 的 todo 或命令状态互相污染。 |
| storage 拒绝逃逸 session prefix 的 key/session id | 已覆盖 | `store.test.ts` | 防止注册命令通过恶意 key 读写其他 session 或 store 根目录。 |
| `LocalDemiStore` 拒绝非相对 store path | 已覆盖 | `store.test.ts` | 防止本地 store 被路径穿越写到任意文件。 |
| 远程 Host / 容器 Host | 未覆盖 | 尚未实现 | 未来实现时用于发现 LocalHost 假设泄漏到通用 Host 接口。 |

### 5.12 Coding Agent Definition

Owner：`packages/agent-coding`

| 测试点 | 状态 | 现有覆盖 / 待补 | 能发现或规避的问题 |
|---|---|---|---|
| coding definition 暴露 shell session tools | 已覆盖 | `packages/agent-coding/src/__tests__/coding-definition.test.ts` | 防止 coding agent 打开后模型没有可用的 shell 控制面。 |
| registered command prompt 注入 system prompt | 已覆盖 | `coding-definition.test.ts` | 防止模型不知道 `editor`、`todo` 等专属命令的正确调用方式。 |
| file reference 通过 workspace host 读取 | 已覆盖 | `coding-definition.test.ts` | 防止引用展开绕过 Host，或模型拿不到用户指定文件内容。 |
| file reference 拒绝 workspace root 外路径 | 已覆盖 | `coding-definition.test.ts` | 防止通过 reference 读取工作区外文件。 |
| definition dispose 清理 environment shell sessions | 已覆盖 | `coding-definition.test.ts` | 防止关闭 coding agent 后 shell 进程继续运行。 |
| reference resolution 与 AgentSession send 顺序集成 | 部分覆盖 | base-agent 有通用覆盖，coding 只覆盖 definition 层 | 需要发现 coding reference 在真实 session 中是否会先写入未展开内容。 |

### 5.13 Editor Command

Owner：`packages/agent-coding`

| 测试点 | 状态 | 现有覆盖 / 待补 | 能发现或规避的问题 |
|---|---|---|---|
| `editor create` 用 heredoc 创建文件 | 已覆盖 | `packages/agent-coding/src/__tests__/editor-command.test.ts` | 防止模型写文件时多行内容、引号或换行被 shell 参数破坏。 |
| editor 拒绝 workspace root 外路径 | 已覆盖 | `editor-command.test.ts` | 防止编辑命令越权修改工作区外文件。 |
| patch escaped path 时写入前拒绝 | 已覆盖 | `editor-command.test.ts` | 防止 unified diff 中一个恶意路径导致部分文件已修改后才失败。 |
| `editor edit` exact replace 和 ambiguous matches 失败 | 已覆盖 | `editor-command.test.ts` | 防止错误替换多个位置或在歧义情况下误改代码。 |
| context disambiguation 只在唯一最近匹配时生效 | 已覆盖 | `editor-command.test.ts` | 防止模型提供上下文后仍改到错误位置。 |
| empty old text 拒绝且不修改文件 | 已覆盖 | `editor-command.test.ts` | 防止空匹配导致在文件所有位置插入内容。 |
| unified diff patch、timestamp headers、多文件创建/删除 | 已覆盖 | `editor-command.test.ts` | 防止常见 patch 格式无法应用，或删除/新增文件语义错。 |
| patch 全量校验后再写入，保证跨文件事务 | 已覆盖 | `editor-command.test.ts` | 防止 patch 中后续文件失败时前面文件已经被部分修改。 |

### 5.14 Todo Command

Owner：`packages/agent-coding`

| 测试点 | 状态 | 现有覆盖 / 待补 | 能发现或规避的问题 |
|---|---|---|---|
| `todo add/list/update/done` raw output | 已覆盖 | `packages/agent-coding/src/__tests__/todo-command.test.ts` | 防止模型拿不到可读的任务状态反馈。 |
| `todo add/list/update/done` JSON output | 已覆盖 | `todo-command.test.ts` | 防止 agent 或 UI 需要结构化 todo 状态时解析失败。 |
| todo 状态按 agent session id 隔离 | 已覆盖 | `todo-command.test.ts` | 防止不同会话共享 todo，造成用户任务串线。 |
| todo 与 shell id 不混淆 | 部分覆盖 | 已通过 session scoped storage 间接覆盖，缺少端到端多 shell 场景 | 需要发现多个 shell 共用同一 agent session 时 todo 是否仍归属正确会话。 |

### 5.15 Coding Agent 工作流

Owner：`packages/agent-coding`

| 测试点 | 状态 | 现有覆盖 / 待补 | 能发现或规避的问题 |
|---|---|---|---|
| StubProvider 通过 shell tools 驱动 editor/todo，真实写文件 | 已覆盖 | `packages/agent-coding/src/__tests__/coding-marathon.test.ts` | 能发现 AgentSession、shell tools、registered command、Host 写文件之间的集成断裂。 |
| workflow 中复用同一个 shell session | 已覆盖 | `coding-marathon.test.ts` | 防止模型后续命令丢 cwd/env 或拿不到之前的 shellId。 |
| workflow 后文件内容正确 | 已覆盖 | `coding-marathon.test.ts` | 防止工具调用看似成功但真实 artifact 没写对。 |
| workflow 后 todo 状态在 agent session 下可读 | 已覆盖 | `coding-marathon.test.ts` | 防止 workflow 中 todo 写到了 shell-local 或全局错误位置。 |
| 创建文件 -> 测试失败 -> 读取错误 -> 修复 -> 测试通过 | 未覆盖 | 需要更真实的 agent scenario test | 用来发现 agent 是否能围绕失败反馈迭代，而不是只验证单步命令。 |
| 长命令 running/yield -> wait -> input/abort 的 agent 级流程 | 未覆盖 | shell 层已测，AgentSession + coding workflow 未测 | 用来发现 shell 控制面在模型多轮工具调用中是否仍保持正确上下文。 |
| tool error 后模型恢复继续执行任务 | 部分覆盖 | base-agent tool error 已测，coding workflow 场景未测 | 用来发现 coding 命令失败后模型是否有足够上下文修正，而不是 session 停死。 |
| 多轮 user message 对 coding workflow 的影响 | 未覆盖 | 需要 multi-turn scenario | 用来发现 queued send、transcript 和 coding state 在连续用户输入下是否错序。 |
| 多 shell + 同 agent session 的 todo/storage 一致性 | 未覆盖 | 需要 agent scenario | 用来发现 shellId 和 agentSessionId 再次混淆。 |

### 5.16 RPC 协议

Owner：`packages/rpc`

| 测试点 | 状态 | 现有覆盖 / 待补 | 能发现或规避的问题 |
|---|---|---|---|
| JSON codec 保留 BigInt metadata 和 Uint8Array | 已覆盖 | `packages/rpc/src/__tests__/json-codec.test.ts` | 防止跨进程/网络传输后 metadata 或二进制数据损坏。 |
| transcript patch 更新 in-place tool_call metadata/status | 已覆盖 | `packages/rpc/src/__tests__/patch.test.ts` | 防止 UI 收不到 tool_call 状态变化，只能靠全量刷新。 |
| transcript diff 处理非 JSON / cyclic metadata | 已覆盖 | `patch.test.ts` | 防止一个不可序列化 metadata 打断整个 RPC 同步。 |
| root entry 不导出 node-only stdio transports | 已覆盖 | `packages/rpc/src/__tests__/root-entry.test.ts` | 防止 browser/client bundle 意外包含 Node-only transport。 |
| RpcClient open/send 经 InProcessTransport 发 transcript/phase | 已覆盖 | `packages/rpc/src/__tests__/rpc.test.ts` | 防止基本协议动作不能驱动 session 或本地 view 不更新。 |
| client close 清空本地 transcript view | 已覆盖 | `rpc.test.ts` | 防止关闭会话后 UI 还显示旧 transcript。 |
| provider error code 只 forward 一次，并保留 transcript error block | 已覆盖 | `rpc.test.ts` | 防止 UI 重复报错，或错误只在 frame 中出现而 transcript 丢失。 |
| shell output、bash audit、generic tool progress frame 映射 | 已覆盖 | `rpc.test.ts` | 防止 TUI/GUI 看不到实时 shell 输出、审计事件或工具进度。 |
| shell_input frames 桥接到 active shell session tool | 已覆盖 | `rpc.test.ts` | 防止用户在壳子里输入内容但没有写入正在等待的进程。 |
| client `shellInput` 等待 result，未 open 时 reject | 已覆盖 | `rpc.test.ts` | 防止 UI 认为输入成功但实际上没有 active session。 |
| retry 产生 transcript patch removals | 已覆盖 | `rpc.test.ts` | 防止 retry 后 UI 留着已经被 runtime 截断的旧 assistant blocks。 |
| host queued send while busy 并按序 drain | 已覆盖 | `rpc.test.ts` | 防止外部壳子连续发送时 action 顺序与 session 执行顺序不一致。 |
| busy 时 host 拒绝 retry/resume/compact | 已覆盖 | `rpc.test.ts` | 防止外部壳子在 active run 中触发破坏性 mutation。 |
| client queued send promise 按各自 phase cycle resolve | 已覆盖 | `rpc.test.ts` | 防止多个 queued send 因同一个 idle 事件一起 resolve。 |
| error 后只 reject active action，queued send 继续 | 已覆盖 | `rpc.test.ts` | 防止一次 provider error 把后续排队用户消息全部错误取消。 |
| abort idle 返回 false，active 返回 true | 已覆盖 | `rpc.test.ts` | 防止 UI stop 按钮在 idle/active 状态下显示错误结果。 |
| close frame abort active session 并 dispose definition resources | 已覆盖 | `rpc.test.ts` | 防止关闭壳子后后台 run 或 shell environment 泄漏。 |
| stdio/WebSocket 基础传输和 RpcClient/RpcHost 端到端 | 已覆盖 | `stdio-transport.test.ts`、`websocket-transport.test.ts` | 防止协议只在 in-process 测试里成立，真实 transport 序列化后失败。 |
| stdio/WebSocket 下 queued send、abort、retry、resume、compact 与 in-process 一致 | 部分覆盖 | transport e2e 基础已测，复杂 action 组合未覆盖 | 需要发现真实 transport latency/frame ordering 下的 action 收敛差异。 |
| close 时 active foreground process 通过真实 transport 被终止 | 未覆盖 | 需要 RPC + shell long process 场景 | 需要防止壳子关闭后远端 session 进程继续占资源。 |

### 5.17 TUI

Owner：`packages/tui`

| 测试点 | 状态 | 现有覆盖 / 待补 | 能发现或规避的问题 |
|---|---|---|---|
| 基本渲染、输入、scroll | 未覆盖 | 需要 TUI 自动化或 snapshot/integration 测试 | 防止核心能力可用但用户无法操作或看不到完整输出。 |
| thinking/text/tool output 显示 | 手动/Gated | 目前靠真实 TUI smoke 验收 | 防止真实模型输出被 TUI 分流、折叠或渲染错。 |
| 通过 RPC client open/send/receive phase/transcript/shell frames | 部分覆盖 | RPC 层已覆盖；TUI 壳子自身未覆盖 | 需要发现 TUI 自己订阅、状态合并、刷新节奏的问题。 |
| 真实 Claude Code provider 输出真实模型回复 | 手动/Gated | 需要指定真实模型和 thinking 等级 smoke | 防止验收只跑 stub provider，没有确认真实 provider 路径。 |
| 交互式 shell 操作在 TUI 中顺畅 | 手动/Gated | 需要多次 smoke，因为模型行为有随机性 | 用来发现真实模型在模糊指令下是否会持续碰壁或误用 shell 控制面。 |

### 5.18 just-bash 子模块

Owner：`packages/just-bash`

| 测试点 | 状态 | 现有覆盖 / 待补 | 能发现或规避的问题 |
|---|---|---|---|
| demi 依赖的 parser protection | 已覆盖 | `bun run test:just-bash-core` | 防止 parser 在安全边界或保护逻辑上退化。 |
| parser edge cases | 已覆盖 | `bun run test:just-bash-core` | 防止 agent 常见脚本语法在升级 just-bash 后突然解析错误。 |
| parse errors | 已覆盖 | `bun run test:just-bash-core` | 防止非法语法被误解析并执行。 |
| upstream bash/awk/sed/grep/jq 等 spec/comparison 测试 | 部分覆盖 | 存在于子模块，不属于主仓库默认入口 | 用来发现主仓库关键路径以外的命令兼容性回归。 |

## 6. 当前优先补测顺序

1. `base-agent` compaction P0：tool boundary、exact replay、多次 compact、failure atomicity、auto compact after tool、preflight compact、persistence roundtrip。
2. 模型可见上下文与 cache baseline：stable request prefix、usage/cache 字段端到端保留、compact 后上下文重新稳定、无界 tool output/ref/preamble 截断策略。
3. AgentSession marathon invariants：单会话内组合 send/queue/retry/resume/abort/tool/error/compact，并在关键步骤断言 transcript 和 provider request。
4. coding agent scenario：创建项目、测试失败、读取错误、修复、测试通过、长命令 wait/input/abort、tool error recovery。
5. persistence/recovery：从 store snapshot 重建后继续 send/retry/resume/compact，且不重复发送或执行已完成 tool result。
6. RPC/TUI：真实 transport 下复杂 action 收敛；gated smoke 覆盖真实 provider、thinking、tool use、交互式 shell 输出。

## 7. 新增测试放置规则

- 单模块行为放在 owner package 的 `src/__tests__/`。
- 跨模块行为放在最接近不变量 owner 的 package：session/runtime 放 `base-agent`，coding workflow 放 `agent-coding`，协议收敛放 `rpc`。
- 涉及真实 CLI、真实模型、网络、本机登录状态或交互 UI 的测试必须 gated，不进入默认 `bun run test`。
- 测试应断言真实 artifact、provider request、transcript blocks、events、session state、RPC frames 或文件内容。
- 新增测试点必须说明能发现或规避的具体问题；说不清风险的问题不进入矩阵。
- 新增 architecture/workflow 约束时，先更新 `docs/agent-rewrite-plan.md` 或本文件，再补测试。

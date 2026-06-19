# 测试覆盖矩阵

| | |
|---|---|
| 日期 | 2026-06-19 |
| 阶段 | 有效性审查中；旧覆盖状态已清空 |
| 范围 | demi agent、shell、provider、RPC、coding agent、TUI |

## 1. 审查结论定义

- 有效：测试断言直接命中文档测试点描述的失败模式，且对应命令通过。
- 部分有效：测试覆盖了部分风险，但关键断言、场景深度或产品边界 oracle 不足。
- 无效：测试存在但不能证明该测试点，或只验证 mock/实现细节自洽。
- 缺口：没有找到能证明该测试点的有效自动化测试。
- Gated：该测试点依赖真实 CLI、真实模型、网络、本机登录状态或交互 UI，不进入默认测试。

测试点必须说明它能发现或规避的问题。没有明确风险的测试点不应进入矩阵。

## 2. 测试判定原则

- 当前产品目标是最小但长期稳定的 agent runtime，不是完整 IDE/GUI agent 产品面。
- 核心链路是 AgentSession 状态机、Transcript replay、Provider 可见上下文、Tool call/result、Shell 控制面、Compaction、Context cache/usage、RPC/TUI 事件呈现。
- Compaction 是长任务能力的核心路径，必须作为 P0 测试面；它要证明上下文接近上限时 agent 能继续工作，并且不切坏 tool pair、不重复执行工具、不污染失败状态。
- Context cache 是基线稳定性要求；即使不主动管理 provider cache，也必须保证模型可见上下文稳定、usage 指标不丢、compact 后上下文能重新稳定。
- 参考项目只用于校准核心稳定性标准，不作为功能照抄清单。
- 权限、审批、分享、revert、项目管理等能力是当前有意不做的产品面，不进入缺口矩阵；如果以后进入产品范围，再新增对应测试模块。
- 每个测试点默认没有结论；只有读过测试实现并确认断言能发现对应风险，才写入审查结论。
- 测试命令通过只能证明测试可运行；有效性必须来自断言是否对准产品边界、失败模式和文档描述。
- 候选覆盖文件只是审查入口，不等于结论。

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

### 3.2 当前门槛审查结论

审查要求：

- 旧的覆盖状态不再作为结论使用。
- 每个门槛必须在第 5 章对应测试点完成有效性审查后，才能回填结论。
- 默认测试命令通过只能证明测试可运行，不能单独证明测试有效或与文档对齐。

| 门槛 | 审查结论 | 审查记录 |
|---|---|---|
| AgentSession 长生命周期稳定 |  | 待逐项审查 `5.4`、`5.5`、`5.6`、`5.7` 后回填。 |
| 模型可见上下文稳定 |  | 待逐项审查 `5.5`、`5.7` 后回填。 |
| Compaction 可支撑长任务 |  | 待逐项审查 `5.6` 后回填。 |
| Context cache baseline |  | 待逐项审查 `5.3`、`5.7`、RPC usage 相关测试后回填。 |
| Shell 控制面支撑真实长命令 |  | 待逐项审查 `5.10` 和 coding 长命令场景后回填。 |
| Coding workflow 能发现真实问题 |  | 待逐项审查 `5.12`、`5.13`、`5.14`、`5.15` 后回填。 |
| 壳子路径能呈现真实模型行为 |  | 待逐项审查 `5.16`、`5.17` 后回填。 |

### 3.3 Compact 参考故事映射

agent-gui/Rust 的 compact 测试是当前最重要的校准对象；demi 不照搬 UI 产品面，但长任务能力相关故事都必须落到 `base-agent` 或协议层测试里。

| 参考故事 | demi 范围判定 | 候选落点 / 待审查 | 能发现或规避的问题 |
|---|---|---|---|
| 新消息发给模型前先做 preflight compact | P0 | `5.6 Compaction`，待审查 | 防止新用户输入已经把上下文推过上限后才请求真实 provider，直接触发 context overflow。 |
| usage 接近上限后 auto compact，再 resume 原动作 | P0 | `5.6 Compaction`，待审查 | 防止长任务在接近 context limit 时直接失败，或 compact 后丢失当前 action。 |
| 用户手动 compact 成功后，summary 成为下一轮模型可见上下文 | P0 runtime；UI 呈现走壳子测试 | `5.6 Compaction`、`5.7 模型可见上下文与 Context Cache`，待审查 | 防止 transcript 看似有 summary，但下一轮 provider request 没有使用它。 |
| summary provider stop/error/empty 都能恢复到一致状态 | P0 | `5.6 Compaction`，待审查 | 防止失败 compact 留下半截 boundary/marker，或空 summary 抹掉旧上下文。 |
| tool 运行期间不 compact；cut point 不切断 `tool_use -> tool_result` | P0 | `5.6 Compaction`，待审查 | 防止模型收到孤立 tool 历史，或 compact 后重复执行已完成工具。 |
| 多次 compact 只从 latest boundary replay，不把旧 summary 反复套入上下文 | P0 | `5.6 Compaction`，待审查 | 防止摘要无限膨胀，或被压缩历史重新进入模型上下文。 |
| compact/preflight 期间 queued send、abort、retry、resume 收敛 | P0 | `5.4 Agent Session Runtime`、`5.6 Compaction`，待审查 | 防止用户在 summary 阶段继续操作时消息丢失、乱序或 session 卡在 compacting。 |
| 长上下文里 stop/continue/retry 后触发 compact | P0 | `5.4 Agent Session Runtime`、`5.6 Compaction`，待审查 | 防止 aborted text、completed tool result 或 retry 后的最新 user turn 在 summary 输入里丢失。 |
| 单个 turn 本身超过 recent budget，需要 split-turn summary | P0 | `5.6 Compaction`，待审查 | 防止一个超长工具/assistant turn 找不到 cut point 后无法 compact，或硬切到 tool result 中间。 |
| compact 后 close/reopen，summary 和 replay 起点保持一致 | P0 persistence | `5.4 Agent Session Runtime`、`5.6 Compaction`，待审查 | 防止重启后 boundary/marker 丢失，模型重新看到旧长历史或看不到摘要。 |
| edit/replay 历史分支影响 summary | 当前无 `replay_from` 产品面；只保留 replay invariant | `5.5 Transcript 与 Replay` | 防止被删掉的历史 mutation 面重新进入设计，同时保证现有 replay 仍只取模型应见内容。 |
| 切换到更小 context 的模型后触发 compact | 当前无运行中切换模型产品面；保留 context limit contract | `5.7 模型可见上下文与 Context Cache`，待审查 | 防止未来 provider/model 配置变化时仍按旧 context limit 请求模型。 |

### 3.4 模型上下文与 provider conformance 映射

Codex 与 pi agent 的参考价值主要在模型实际看到什么、异常 provider 输入如何收敛、完整历史与模型上下文如何分离。

| 参考能力 | demi 范围判定 | 候选落点 / 待审查 | 能发现或规避的问题 |
|---|---|---|---|
| 模型可见上下文由一个中心 replay/effective context 产生 | P0 | `5.5 Transcript 与 Replay`、`5.7 模型可见上下文与 Context Cache`，待审查 | 防止 provider、compact、retry 各自拼上下文，最终真实模型看到的内容不一致。 |
| 大 tool output、引用内容、preamble 有边界和截断策略 | P0 | `5.7 模型可见上下文与 Context Cache`，待审查 | 防止任意一次工具输出或文件引用直接撑爆 context，compact 也无法补救。 |
| cache read/write usage 从 provider 传到 session/RPC，但不改变 agent 行为 | 基线 | `5.3 Claude Code Provider`、`5.7 模型可见上下文与 Context Cache`，待审查 | 防止 cache 指标丢失，或把 provider cache 当成会影响 transcript/tool loop 的状态。 |
| compact 后历史重建与 live compaction 结果一致 | P0 | `5.6 Compaction`、`5.11 Host、FS 与 Store`，待审查 | 防止 session 恢复后 replay 起点、summary 或 recent context 与内存态不一致。 |
| 完整审计日志与模型上下文分离 | P0 | `5.5 Transcript 与 Replay`、`5.6 Compaction`、`5.7 模型可见上下文与 Context Cache`，待审查 | 防止为了压缩模型上下文而删除完整历史，或把审计块误发给模型。 |
| provider 异常输入：empty、malformed、unicode/media、context overflow、tool result 缺失 | P0 provider 边界 | `5.3 Claude Code Provider`，待审查默认边界；真实 CLI 仍走 gated | 防止真实 CLI/服务端返回边界事件时 agent 卡死、污染 pending state 或错误分类不可恢复。 |
| pre-turn/mid-turn/manual compact 的 request shape 有 snapshot 级断言 | P0 | `5.6 Compaction`、`5.7 模型可见上下文与 Context Cache`，待审查 | 防止 incoming user 被重复放入请求、control-only item 被摘要、或 mid-turn continuation 丢失当前上下文。 |

### 3.5 范围裁剪

这些参考项目能力不进入当前测试缺口矩阵；如果产品范围变化，再作为新模块写入本文件。

| 参考能力 | 当前处理 | 原因 |
|---|---|---|
| GUI 历史编辑、revert、branch/tree replay | 不作为缺口；只保留 append-only replay invariant | 当前架构已删除 `replay_from`，只允许 retry 截断最后一轮。 |
| pi 的 branch summarization / tree navigation | 不作为缺口 | demi 当前没有分支会话树，compact 只服务单条 append-only session。 |
| opencode 的 permission/share/project/worktree 管理 | 不作为缺口 | 当前产品目标是最小稳定 agent runtime，不是完整 IDE/协作产品。 |
| plugin/custom compaction hooks | 不作为缺口 | 当前没有插件扩展 API；先保证默认 compact 的确定性和原子性。 |
| opencode 式旧 tool output pruning | 不作为缺口；只接受未来作为模型上下文裁剪策略 | demi 的 transcript 必须保留完整审计历史，不能把 pruning 误写成默认行为。 |
| 远程 Host / 容器 Host adapter | 不作为当前缺口 | 当前实现目标只要求 Host 抽象边界和 LocalHost 稳定，远程 adapter 属于后续产品范围。 |

## 4. 默认测试入口

- `bun run typecheck`：类型检查。
- `bun run test`：主仓库默认自动化测试。
- `bun run test:just-bash-core`：demi 依赖的 just-bash 核心解析保护测试。
- `bun run check:registry`：public registry / package boundary 检查。

## 5. 模块测试点

### 5.1 平台与包边界

Owner：`packages/core`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| 根入口保持 browser-safe / runtime-neutral，不静态依赖 Node-only 能力 | 有效 | `platform-entrypoints.test.ts` 从 root entry 递归解析静态 import 闭包，并对 `node:` import/require、`Buffer`、`process.env/cwd`、显式 Node-only subpath 做负向断言；能直接发现 root 包把 Node-only 代码带入浏览器入口。验证：`bun test packages/core/src/__tests__/platform-entrypoints.test.ts packages/provider/src/__tests__/registry.test.ts packages/provider/src/__tests__/stub.test.ts`，15 pass。 | `packages/core/src/__tests__/platform-entrypoints.test.ts` | 防止浏览器或非 Node runtime 导入根包时因为 `node:*`、`process`、`Buffer` 直接崩溃。 |
| 只有 RPC host 在运行时代码中直接实例化 `AgentSession` | 有效 | 测试扫描 `packages` runtime source，查找从 `@demi/base-agent` runtime import `AgentSession` 的文件，并断言唯一允许文件是 `packages/rpc/src/host.ts`；能发现 UI/provider/业务包绕过 RPC host 的 runtime 耦合。验证同上。 | `platform-entrypoints.test.ts` | 防止 UI、provider 或业务包绕过协议层调用 agent runtime，导致状态和事件边界失控。 |
| package manifest 不引入越层依赖 | 有效 | 测试读取 package manifests，断言 platform-neutral 包不依赖 `@demi/provider-claude-code`，`@demi/shell` 不声明越层依赖，Claude provider 不引入 Claude SDK；能发现依赖声明层面的隐藏耦合。验证同上。 | `platform-entrypoints.test.ts` | 防止平台无关包通过依赖声明暗中耦合 Node adapter、真实 provider 或 UI 包。 |
| 主仓库使用 forked just-bash package，不维护第二份上游源码快照 | 有效 | 测试检查禁止目录是否存在，并扫描 runtime imports 是否指向 vendor/upstream/旧 package 名；能发现仓库里出现第二份 just-bash 源或错误 import 来源。验证同上。 | `platform-entrypoints.test.ts` | 防止 bash engine 出现两个来源，造成修复只改一份、运行消费另一份的分叉问题。 |

### 5.2 Provider 抽象

Owner：`packages/provider`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| `ProviderRegistry` 注册、查找、创建 provider | 有效 | `registry.test.ts` 注册带 `state` 和 `createProvider` 的 definition，断言 `get/state/createProvider` 返回同一 definition 并实际运行 provider 输出配置值；能发现 type lookup、config 传递或 provider 创建错位。验证：`bun test packages/core/src/__tests__/platform-entrypoints.test.ts packages/provider/src/__tests__/registry.test.ts packages/provider/src/__tests__/stub.test.ts`，15 pass。 | `packages/provider/src/__tests__/registry.test.ts` | 防止 provider id/type 解析错位，导致 RPC 打开的 provider 不是用户配置的 provider。 |
| `ProviderRegistry` registration snapshot 通知 | 有效 | 测试通过 `observe` 收集初始、register、unregister 三次 snapshot，并断言 provider type 序列为 `[] -> ['stub'] -> []`；能发现注册列表通知漏发或顺序错误。验证同上。 | `registry.test.ts` | 防止 UI 或 host 看到过期 provider 列表，造成模型选择器和实际能力不一致。 |
| `StubProvider` 多轮 scripted events | 有效 | 测试连续调用 `run()` 两次并断言每轮输出不同 scripted events 和 usage；能发现 StubProvider 没有按 turn 推进，导致 scenario test 不可复现。验证同上。 | `packages/provider/src/__tests__/stub.test.ts` | 防止 agent scenario test 依赖真实模型随机性，保证状态机测试可复现。 |
| `StubProvider` function script 能读取 request 并驱动 tool roundtrip | 有效 | 测试第二轮 function script 检查 request.items 中存在 `tool_result`，没有则主动失败；能发现 agent/provider scenario 没有把 tool result 回灌到下一轮 request。验证同上。 | `stub.test.ts` | 能发现 provider request 内容错误、tool_result 没有回灌、下一轮上下文不正确等问题。 |
| `StubProvider` 脚本耗尽时报错 | 有效 | 测试消费唯一 turn 后再次 `run()` 并断言抛出 `ran out of turns`；能发现意外额外 provider turn、retry/resume/loop 不会静默通过。验证同上。 | `stub.test.ts` | 防止测试悄悄多跑一轮但仍然通过，暴露意外 retry/resume/loop。 |

### 5.3 Claude Code Provider

Owner：`packages/provider-claude-code`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| Claude CLI path/version 检测 |  |  | `packages/provider-claude-code/src/__tests__/cli.test.ts` | 防止 provider 在没有可用 CLI 或版本信息异常时进入半可用状态。 |
| Claude auth/runtime state 读取，不依赖 SDK |  |  | `cli.test.ts` | 防止认证状态判断依赖不可控 SDK 行为，导致 UI 误报可用或不可用。 |
| Claude CLI args/env 构造契约 |  |  | `cli.test.ts` | 防止模型、thinking、MCP/config 等关键启动参数在 CLI 调用时丢失或拼错。 |
| demi inference items 转 Claude JSONL input messages |  |  | `packages/provider-claude-code/src/__tests__/jsonl-output.test.ts` | 防止 user/assistant/tool 历史转换错位，真实模型看到的上下文与 transcript 不一致。 |
| assistant turn 与 tool result grouping |  |  | `jsonl-output.test.ts` | 防止 Claude 所需的 assistant/tool_result 顺序被拆乱，引发 tool_use 无匹配结果。 |
| binary media content 转 Claude base64 source |  |  | `jsonl-output.test.ts` | 防止图片或二进制输入在 provider 边界损坏。 |
| Claude stdout 映射 text/thinking/tool call/usage |  |  | `jsonl-output.test.ts` | 防止真实模型输出被吞、thinking 不显示、tool_call 不触发或 usage 统计失真。 |
| cache read/write usage 字段解析 |  |  | `jsonl-output.test.ts` 只覆盖 usage 字段映射 | 防止 provider 已返回 cache 指标但 demi 丢字段，影响后续 cache contract 判断。 |
| malformed assistant tool_use / control_request 报错并不污染 pending state |  |  | `jsonl-output.test.ts`、`provider.test.ts` | 防止异常 provider 消息让 session 永久等待不存在的 tool_result。 |
| control_request、SDK MCP control_request、assistant tool_use 跨 run 状态机 |  |  | `packages/provider-claude-code/src/__tests__/provider.test.ts` | 防止 Claude Code 多轮工具协议在 run 边界丢失 pending call 或重复提交结果。 |
| 缺失 tool_result / pending control_request 收敛为 provider error |  |  | `provider.test.ts` | 防止协议断裂时 agent 卡死或继续用不完整上下文调用模型。 |
| CLI 非零退出、stdout 迭代失败、abort 的 transport 清理 |  |  | `provider.test.ts` | 防止 CLI 进程泄漏、后续 run 复用坏 transport，或错误无法传回 session。 |
| context overflow / rate limit / auth expired 等 provider 错误分类 |  |  | `jsonl-output.test.ts`、`provider.test.ts` 覆盖 context overflow / result error / rate-limit code / auth expired code | 防止真实 provider 给出可恢复或需用户处理的错误时，session 只得到不可行动的 generic error。 |
| empty assistant content、empty stream、空 thinking 事件 |  |  | `jsonl-output.test.ts`、`provider.test.ts` | 防止 provider 返回空内容时误判成功、插入无意义 response，或 compact empty summary 污染 transcript。 |
| unicode surrogate、超长 JSONL 字段、media + tool_result 混合输入 |  |  | `jsonl-output.test.ts` | 防止 JSONL 编码、base64、tool result grouping 在真实边界输入下损坏。 |
| 与 `AgentSession` 和 shell tools 的 provider 集成 |  |  | `provider.test.ts` | 能发现 provider event、AgentSession tool loop、shell tool result 三者之间的接口不匹配。 |
| 真实 Claude CLI e2e |  |  | `real-cli.e2e.test.ts`，不属于默认稳定测试 | 用来发现 fake transport 无法覆盖的本机 CLI、账号、网络、真实输出格式变化。 |
| 真实模型 thinking/tool use/text 输出验收 |  |  | 需要 TUI/CLI smoke 流程 | 用来确认最终用户路径确实看到真实模型回复、thinking 和 tooluse，而不是只验证 mock。 |

### 5.4 Agent Session Runtime

Owner：`packages/base-agent`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| `send` 写入 user turn，再构造 provider request |  |  | `packages/base-agent/src/__tests__/session.test.ts` | 防止 provider request 漏掉用户输入、preamble 顺序错误，或 transcript 与模型上下文分叉。 |
| provider response 写入 transcript |  |  | `session.test.ts` | 防止 UI/RPC 无法看到模型输出，或 retry/compact 基于缺失历史运行。 |
| provider error 写入 transcript、发 event、reject action |  |  | `session.test.ts` | 防止错误只在 Promise 中出现而 transcript/UI 不可见，或者 action 静默成功。 |
| provider error 后关闭 provider iterator |  |  | `session.test.ts` | 防止 provider stream 泄漏，后续 run 收到旧 stream 的残留事件。 |
| reference resolution 在写入 user turn 和 provider request 前执行 |  |  | `session.test.ts` | 防止 transcript 保存未展开引用，导致恢复或重放时上下文不可复现。 |
| reference resolution 卡住时 abort 可收敛 |  |  | `session.test.ts` | 防止文件引用或外部解析卡住后 session 无法取消。 |
| provider 不 yield 时 abort 可收敛 |  |  | `session.test.ts` | 防止模型 stream 卡死时 UI 的停止按钮无效。 |
| long-running tool invocation 卡住时 abort 可收敛 |  |  | `session.test.ts` | 防止工具实现挂起后占住 session，后续用户输入无法执行。 |
| tool call 执行后继续 provider roundtrip |  |  | `session.test.ts` | 防止 tool_result 没有回传给模型，模型无法基于工具结果继续回答。 |
| tool invocation throw 转成 error tool result 并继续 |  |  | `session.test.ts` | 防止工具异常升级成 session 崩溃，模型失去恢复机会。 |
| tool progress event 发出 |  |  | `session.test.ts` | 防止 shell output/audit 等进度事件只留在内部，UI 无法实时显示。 |
| queued send 排队并按顺序 drain |  |  | `session.test.ts` | 防止用户连续发送消息时顺序错乱，或后一条消息覆盖前一条运行状态。 |
| retry 截断最后 assistant response 并 rerun latest user turn |  |  | `session.test.ts` | 防止 retry 把旧错误输出和新输出混在一起，或重跑了错误的用户 turn。 |
| resume 标记 abort 为 resumed，并追加 resume turn |  |  | `session.test.ts` | 防止恢复后 transcript 无法区分已中止内容和继续执行内容。 |
| abort 后 resume 前清理 pending tool calls |  |  | `session.test.ts` | 防止恢复时模型看到仍在执行的旧 tool_call，重复等待或重复执行工具。 |
| mutation guard 拒绝 busy/reserved 期间 mutation |  |  | `session.test.ts` | 防止 compact/retry/resume 与 active run 交错修改 transcript，造成不可恢复状态。 |
| store snapshot 写入 |  |  | `session.test.ts` | 防止进程退出或 host 重启后没有可恢复的会话状态。 |
| extension state snapshot 通过 lifecycle 写入并持久化 |  |  | `session.test.ts` | 防止 todo 等 agent 扩展状态只在内存里存在，恢复后状态丢失。 |
| 从 store snapshot 重建 session 后继续运行 |  |  | `session-marathon.test.ts` | 需要发现 snapshot schema、idFactory、phase 或 transcript replay 在重启后不兼容的问题。 |
| provider error 后恢复不重复发送已完成 tool result |  |  | `session-marathon.test.ts` | 需要规避重启后重复执行破坏性工具或给模型重复上下文。 |
| abort/retry/resume/compact 组合交错 |  |  | `session-marathon.test.ts` | 组合路径容易暴露单点测试发现不了的 phase、queue、transcript 原子性问题。 |
| 单会话 marathon 覆盖 send/queue/retry/resume/abort/tool/error/compact 累计状态 |  |  | `session-marathon.test.ts` | 用来发现状态只在单点测试里正确，长期累计后 id、phase、pending action、tool 状态或 transcript 顺序漂移。 |
| 每个关键步骤精确断言 provider request |  |  | `session-marathon.test.ts`、`context-cache.test.ts`、`compaction.test.ts` | 防止模型实际看到的上下文与 transcript 看起来正确但不同，尤其是 compact、retry、resume 后的重放内容。 |
| transcript 结构不变量集中校验 |  |  | `helpers.ts` 的 invariant helper 被 `session-marathon.test.ts`、`compaction.test.ts` 使用，覆盖 block id、createdAt、toolUseId、completed tool output、compaction marker 引用 | 防止出现重复 block id、缺 createdAt、重复 toolUseId、completed tool_call 缺 output，或 compaction marker 指向不存在的 boundary。 |
| provider mock 行为贴近真实 provider 事件顺序 |  |  | `session-marathon.test.ts`、`compaction.test.ts` 覆盖 pending tool、response、auto compact、abort event；真实 Claude 随机路径仍靠 gated smoke | 防止 deterministic 测试用一个过于理想的 provider，真实 Claude Code 路径才暴露 tool/response 时序问题。 |

### 5.5 Transcript 与 Replay

Owner：`packages/base-agent`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| user/text/response blocks 转 inference items |  |  | `packages/base-agent/src/__tests__/transcript.test.ts` | 防止 transcript 能显示但不能正确喂回 provider。 |
| pending tool call 发现与完成 |  |  | `transcript.test.ts` | 防止 tool_call 状态卡在 executing，后续 run 误判还有未完成工具。 |
| completed tool call 转 `tool_use` + `tool_result` |  |  | `transcript.test.ts` | 防止模型下一轮看不到工具输入或工具输出，导致重复调用工具。 |
| non-JSON tool input 安全存储和 replay |  |  | `transcript.test.ts` | 防止异常输入对象让 transcript 序列化崩溃或 replay 失败。 |
| dangling executing tool call 清理 |  |  | `transcript.test.ts` | 防止 abort/restart 后遗留 executing tool 阻塞 resume。 |
| extension state snapshot latest 查询 |  |  | `transcript.test.ts` | 防止恢复 agent 扩展状态时拿到过期 snapshot。 |
| non-JSON extension state token estimate |  |  | `transcript.test.ts` | 防止 token estimate 被 BigInt/cycle 等扩展状态打断，影响 compact 判断。 |
| transcript snapshot 序列化/反序列化等价 |  |  | `context-cache.test.ts`、`session-marathon.test.ts` | 需要发现保存后再加载丢 block、丢 metadata 或改变 replay 内容的问题。 |
| provider request exact replay 内容 |  |  | `context-cache.test.ts`、`session-marathon.test.ts`、`compaction.test.ts` | 需要发现复杂历史、tool、extension、compact 混合时喂给模型的上下文漂移。 |
| effective replay 只包含模型应看到的 block |  |  | `context-cache.test.ts` | 防止 compaction marker、extension snapshot、internal error 状态等内部块进入 provider request。 |
| replay 保持 tool_use/tool_result 成对且顺序正确 |  |  | `compaction.test.ts`、`context-cache.test.ts`、`session-marathon.test.ts` | 防止 provider 收到孤立 tool_result、孤立 tool_use 或乱序工具历史。 |
| replay 中 thinking/redacted thinking 能跨 provider 边界保留 |  |  | `transcript.test.ts`、`jsonl-output.test.ts`、`compaction.test.ts` | 防止开启 thinking 的真实模型路径在重放或 compact 后丢失签名、redacted thinking 或顺序。 |

### 5.6 Compaction

Owner：`packages/base-agent`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| 手动 compact 插入 boundary 和 marker |  |  | `session.test.ts` | 防止 compact 完成后 transcript 无法标识历史摘要边界。 |
| compact 不删除旧 blocks |  |  | `session.test.ts` | 防止 compact 破坏审计历史，影响 UI 查看完整 transcript。 |
| replay 从 latest compaction boundary 开始 |  |  | `transcript.test.ts` | 防止 compact 后仍把旧长历史发给 provider，失去压缩意义。 |
| boundary summary 转成下一次 inference user message |  |  | `transcript.test.ts` | 防止模型看不到摘要，只收到被截断的 recent context。 |
| usage 接近 context limit 时自动 compact + resume |  |  | `session.test.ts` | 防止长会话到上下文边界后直接失败，而不是自动恢复。 |
| compaction summary provider 卡住时 abort 可收敛 |  |  | `session.test.ts` | 防止 compact 阶段卡死导致用户无法停止 session。 |
| preflight compact 在新 provider request 前发生 |  |  | `compaction.test.ts` | 防止新用户消息已经把上下文推过上限后才请求模型，导致真实 provider 直接 context overflow。 |
| compaction summary request 的模型、thinking、cwd、tools 与契约一致 |  |  | `compaction.test.ts` | 防止 summary 用错模型/思考等级、错误 cwd，或把普通工具暴露给总结请求。 |
| cut point 不能切断 `tool_use -> tool_result` |  |  | `compaction.test.ts` | 需要防止模型看到孤立 tool_use 或孤立 tool_result，引发 provider 协议错误。 |
| 单个超长 turn 的 split-turn cut point |  |  | `compaction.test.ts`、`transcript.test.ts` | 防止整轮超过 recent budget 时无法 compact，或把同一 turn 的早期上下文直接丢掉。 |
| compact 后下一次 provider request 精确等于 summary + recent context |  |  | `compaction.test.ts` | 需要发现 summary、recent blocks、preamble 或 tool history 被漏放、重复放、乱序放。 |
| pre-turn compact request shape：incoming user 恰好出现一次，control-only item 不进 summary |  |  | `compaction.test.ts` | 防止当前用户消息被重复注入，或模型切换、cwd/config diff、resume control block 等运行时控制信息污染 summary。 |
| mid-turn compact / auto-recover continuation request shape |  |  | `compaction.test.ts` | 防止工具执行后自动 compact 时丢失 continuation 上下文，或把已完成工具重新放成 pending。 |
| 多次 compact 只 replay latest boundary，不重复旧 summary |  |  | `compaction.test.ts` | 需要规避摘要套摘要无限膨胀，或旧历史重新进入上下文。 |
| summary provider error/abort 不留下半截 boundary/marker |  |  | `compaction.test.ts`、`session.test.ts` | 需要防止 compact 失败后 transcript 处于既不像旧状态也不像新状态的中间态。 |
| empty summary 不插入 boundary/marker，且 session 可继续 |  |  | `compaction.test.ts` | 防止空摘要把旧历史替换成无信息 boundary，后续模型失去任务上下文。 |
| 没有可压缩历史时 manual compact 明确 no-op 或可解释失败 |  |  | `compaction.test.ts` | 防止用户触发 compact 后 session 状态变化但没有任何有效 summary，或 UI/RPC 收不到可解释结果。 |
| auto compact + resume 不重复执行已完成 tool call |  |  | `compaction.test.ts` | 需要防止上下文压缩后把已完成工具当成待执行工具再次运行。 |
| aborted text、completed tool result 在 compact summary 输入中保留 |  |  | `compaction.test.ts` | 防止长任务中被用户停止过的有用进展或已完成工具证据在压缩后消失。 |
| compact 期间 queued send 能按序 drain |  |  | `compaction.test.ts` | 防止长 summary 期间用户继续输入后消息丢失、乱序，或 compact 完成后没有继续处理。 |
| compact/preflight 期间 abort、retry、resume 的 action 收敛 |  |  | `session.test.ts`、`compaction.test.ts` | 防止停止、重试或继续操作与 summary 写入交错，留下 busy phase、重复 action 或半截 transcript。 |
| compact 过程遇到 context-window 错误与普通 provider 错误分流 |  |  | `compaction.test.ts`、`context-cache.test.ts` | 防止可通过裁剪/重试恢复的超限错误被当成普通失败，或普通 provider 错误无限重试。 |
| compact 后持久化再恢复，replayable blocks 保持一致 |  |  | `compaction.test.ts`、`context-cache.test.ts` | 需要发现 boundary/marker 在 snapshot 中丢失或恢复后 replay 起点错误。 |
| thinking/redacted thinking/extension state/tool metadata 混合 transcript 下 cut point 正确 |  |  | `compaction.test.ts`、`context-cache.test.ts` | 需要防止非文本 block 或扩展状态让 compact 切点算法误判。 |
| thinking 模型下 summary request 的 token/budget 设置有效 |  |  | `compaction.test.ts` | 防止 summary 请求因为 thinking budget 与 max output token 冲突而失败，导致长任务无法 compact。 |

### 5.7 模型可见上下文与 Context Cache

Owner：`packages/provider-claude-code`、`packages/base-agent`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| provider request prefix 在普通多轮对话中稳定 |  |  | `context-cache.test.ts` | 防止 system prompt、tools schema、preamble 或历史 items 无意义重排，破坏 context cache 基线。 |
| provider request 只由 effective transcript 和当前 prompt context 构成 |  |  | `context-cache.test.ts` | 防止 store snapshot、extension state、compaction marker、UI-only 状态进入模型可见上下文。 |
| provider、retry、compact、resume 共用同一个 effective transcript 入口 |  |  | `context-cache.test.ts`、`session-marathon.test.ts`、`compaction.test.ts` | 防止不同路径各自拼 request，导致正常 send 通过但 compact 或 retry 后真实模型上下文漂移。 |
| 注入内容有明确上限和截断策略 |  |  | `context-cache.test.ts` 覆盖 preamble、reference 展开文本、assistant text、tool result 的 provider-visible 截断，且 transcript audit log 保留原文 | 防止大文件引用、大工具输出或过长 preamble 直接撑爆上下文，compact 也来不及恢复。 |
| 记录给模型的 tool output 使用 head/tail + truncation marker |  |  | `context-cache.test.ts` 覆盖 head/tail 保留和 truncation marker | 防止截断后模型不知道内容被省略，或只保留头部导致错误诊断。 |
| provider context-overflow 错误触发可恢复路径或明确失败 |  |  | `context-cache.test.ts` | 防止模型上下文超限后 session 只记录 generic error，无法 compact/retry 或向壳子给出明确状态。 |
| provider usage 中 cache read/write token 字段被解析 |  |  | `jsonl-output.test.ts` | 防止 provider cache 指标被丢弃，后续无法判断真实 cache 行为。 |
| cache usage 被 AgentSession 记录并对外暴露 |  |  | `context-cache.test.ts` | 需要发现 usage 在 provider 到 session 到 UI/RPC 链路中丢字段。 |
| cache 只是 provider 透明优化时，不影响 agent 行为 |  |  | `context-cache.test.ts` | 需要保证 cache 指标变化不会改变 transcript、tool loop 或错误处理。 |
| demi 主动保障 stable prompt prefix 时，跨 turn prefix 字节级稳定 |  |  | `context-cache.test.ts` 断言等价历史下 stable prefix 字节级一致 | 如果要主动利用 cache，该测试能发现无意义重排 tools/system prompt 破坏命中率。 |
| tools/schema/system prompt/model/preamble 改变时 cache 失效规则 |  |  | `context-cache.test.ts` 断言 tools、system prompt、model、preamble 变化会改变 cache 输入前缀 | 需要防止 cache 命中建立在错误前缀上，或该失效时没有失效。 |
| compact 后 cache prefix 变化与重新稳定 |  |  | `context-cache.test.ts`、`compaction.test.ts` 覆盖 compact 后 summary prefix 替换旧历史，后续 request 重新形成稳定前缀 | 需要发现 compact 后上下文前缀持续抖动，导致 cache 永远无法稳定命中。 |
| 真实 provider cache 命中 |  |  | 只能做真实 provider smoke，不进默认 deterministic 测试 | 用来确认 deterministic contract 之外的真实 CLI/服务端 cache 行为没有退化。 |

### 5.8 Command Registry

Owner：`packages/shell`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| positionals、flags、stdin fields 解析 |  |  | `packages/shell/src/__tests__/command.test.ts` | 防止模型调用注册命令时参数被错配到错误字段。 |
| long options 校验与数字 coercion |  |  | `command.test.ts` | 防止字符串数字、非法 option 进入命令实现造成隐式行为。 |
| `--json`、boolean、repeated array options |  |  | `command.test.ts` | 防止 agent 依赖结构化输出时拿到 raw text 或数组/布尔解析错。 |
| unknown options / invalid values 拒绝 |  |  | `command.test.ts` | 防止模型拼错参数却被静默忽略，产生看似成功的错误操作。 |
| `CommandSpec` 作为 prompt/help 单一来源 |  |  | `command.test.ts` | 防止 system prompt、`prompt` 子命令和实际 parser 三套说明漂移。 |
| registry 注册命令并渲染 prompt |  |  | `command.test.ts` | 防止新增命令未进入 agent 可见能力列表。 |
| 注册命令名不能复用 shell/system reserved names |  |  | `command.test.ts` | 防止 agent 命令遮蔽常见系统命令或 shell builtin，破坏用户预期。 |
| `<command> prompt` 使用同一个 renderer |  |  | `command.test.ts` | 防止模型通过 help 学到的调用方式与 system prompt 不一致。 |
| JSON mode output schema 校验 |  |  | `command.test.ts` | 防止注册命令声称 JSON 输出但返回不可解析或结构错误的数据。 |
| 无 JSON schema 的 subcommand 拒绝 JSON mode |  |  | `command.test.ts` | 防止调用方误以为某命令有结构化输出而继续自动处理。 |

### 5.9 Bash Environment 语义

Owner：`packages/shell`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| cwd/env/last status 跨 `shell_exec` 保持 |  |  | `packages/shell/src/__tests__/environment.test.ts` | 防止 shell session 退化成一次性命令执行，模型后续命令跑在错误目录或环境。 |
| stateful builtins 在当前 session 生效且非法参数不污染状态 |  |  | `environment.test.ts` | 防止 `cd/export/unset` 等失败后留下半修改状态。 |
| `read`、stdin、heredoc、here-string 进入当前 shell 语义 |  |  | `environment.test.ts` | 防止交互式或 heredoc 命令的输入被错误丢弃或写到错误命令。 |
| list operators、prefix assignments、assignment-only commands |  |  | `environment.test.ts` | 防止常见 shell 语法被错误 fallback 到系统 shell 或执行顺序错误。 |
| parameter/arithmetic/command substitution expansion |  |  | `environment.test.ts` | 防止变量展开和命令替换与 bash 语义偏离，导致 agent 命令读写错误路径或值。 |
| functions、local、return、function redirection |  |  | `environment.test.ts` | 防止 sourced script 或函数调用污染外层 scope，或 redirection 应用时机错误。 |
| background jobs、jobs wait、spawn failure |  |  | `environment.test.ts` | 防止后台任务丢失、无法等待，或 spawn 失败导致 shell 挂死。 |
| `source`、`set`、`shift`、`eval`、`type`、`command` builtin |  |  | `environment.test.ts` | 防止这些会改变 session 或 introspection 的 builtin 被错误交给系统命令。 |
| registered command / builtin / function / system command 调度顺序 |  |  | `environment.test.ts` | 防止 agent 注册能力被 shell function 遮蔽，或 `command` 绕过状态 builtin。 |
| glob、redirection、pipeline、compound command、subshell |  |  | `environment.test.ts` | 防止常见脚本组合在 agent shell 中与真实 shell 行为严重不一致。 |
| `if`、`[[ ]]`、`case`、for/while/until、group、break/continue |  |  | `environment.test.ts` | 防止控制流判断错误，尤其是脚本自动化里跳错分支或循环无法退出。 |
| explicit `exit` 不被 negation/control flow 改写 |  |  | `environment.test.ts` | 防止用户脚本已经退出但 runner 继续执行后续命令。 |
| unsupported parser constructs 明确拒绝，不整段交给系统 shell |  |  | `environment.test.ts` | 防止逃过 audit/registered command/Host 抽象，直接用系统 shell 执行不可控脚本。 |
| system command audit events 和 spawn failure |  |  | `environment.test.ts` | 防止 UI/RPC 审计缺失，或命令不存在时工具调用挂死。 |
| 更完整的 bash 兼容性 spec |  |  | 主仓库覆盖关键 agent 语义；更完整 spec 在 just-bash 子模块 | 用来发现主仓库关键路径之外的 bash 兼容性回归。 |

### 5.10 Shell 控制面

Owner：`packages/shell`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| `shell_exec` running/yield 后可 `shell_wait` |  |  | `environment.test.ts`、`packages/shell/src/__tests__/tools.test.ts` | 防止长命令超出单次等待后失联，模型无法继续观察输出。 |
| output limit 触发 running output_limit |  |  | `environment.test.ts` | 防止大输出撑爆 tool result 或 transcript。 |
| `shell_input` 可向 foreground process 写 stdin |  |  | `environment.test.ts`、`tools.test.ts` | 防止交互式命令无法继续，或输入写入错误 shell。 |
| idle foreground process 默认保持 running，不误报 needs_input |  |  | `environment.test.ts` | 防止安静安装、构建、dev server 被误判为需要用户输入。 |
| `shell_wait` 的 `yieldAfterMs` 从每次调用重新计时 |  |  | `environment.test.ts` | 防止 wait 因累计时间错误立刻返回，模型反复空轮询。 |
| `shell_abort` 终止 foreground process，且是 control action |  |  | `environment.test.ts`、`tools.test.ts` | 防止停止 dev server 被当作任务失败，或进程继续占端口。 |
| abort/timeout flush redirected foreground output |  |  | `environment.test.ts` | 防止本应写入文件的输出泄漏到 tool output，或文件内容丢失。 |
| dispose shell kill foreground/background 并移除 session |  |  | `environment.test.ts` | 防止关闭 session 后仍有子进程残留。 |
| timeout kill foreground process，之后 shell 可复用 |  |  | `environment.test.ts` | 防止 timeout 后 abort state 泄漏，导致下一条命令立即失败。 |
| shell tool result 格式保留 metadata 且模型可读 |  |  | `tools.test.ts` | 防止模型必须解析不稳定 JSON 或看不到 shellId/status/next action。 |
| `shell_input` 拒绝空 stdin，不承担 polling |  |  | `tools.test.ts` | 防止空 input 被滥用成 wait，重新引入无意义控制动作。 |
| AgentSession abort signal 传播到 `shell_exec` / `shell_wait` / `shell_input` |  |  | `tools.test.ts` | 防止 UI abort 只停止 session 状态，不停止实际前台进程。 |
| 模型在真实长进程场景中稳定选择 wait/input/abort |  |  | 需要真实 provider/TUI smoke 多次验证 | 用来发现 prompt/tool result 虽然单测正确，但真实模型仍然误用控制面的问题。 |

### 5.11 Host、FS 与 Store

Owner：`packages/shell`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| `HostBackedFileSystem` 通过 `Host.spawn` 完成 read/exists/stat/write/append/readdir |  |  | `packages/shell/src/__tests__/host-fs.test.ts` | 防止 shell/coding 绕过 Host 直接读写本机 fs，破坏远程或容器后端边界。 |
| shell root entry 只暴露 browser-safe Host contract / FS class |  |  | `packages/shell/src/__tests__/root-entry.test.ts` | 防止 `@demi/shell` 根入口静态带入 Node-only adapter，破坏 browser/runtime-neutral 包边界。 |
| readFileBuffer 返回 raw bytes |  |  | `host-fs.test.ts` | 防止二进制文件被文本编码损坏。 |
| `LocalHost` spawn capture stdout 和 stdin |  |  | `packages/shell/src/__tests__/local-host.test.ts` | 防止本地 adapter 不能正确连接进程输入输出。 |
| `LocalHost` terminate foreground process |  |  | `local-host.test.ts` | 防止 shell abort/timeout 在本地进程层失效。 |
| `AgentSessionCommandStorage` 按 agent session id prefix 隔离 keys |  |  | `packages/shell/src/__tests__/store.test.ts` | 防止多个 agent session 的 todo 或命令状态互相污染。 |
| storage 拒绝逃逸 session prefix 的 key/session id |  |  | `store.test.ts` | 防止注册命令通过恶意 key 读写其他 session 或 store 根目录。 |
| `LocalDemiStore` 拒绝非相对 store path |  |  | `store.test.ts` | 防止本地 store 被路径穿越写到任意文件。 |

### 5.12 Coding Agent Definition

Owner：`packages/agent-coding`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| coding definition 暴露 shell session tools |  |  | `packages/agent-coding/src/__tests__/coding-definition.test.ts` | 防止 coding agent 打开后模型没有可用的 shell 控制面。 |
| registered command prompt 注入 system prompt |  |  | `coding-definition.test.ts` | 防止模型不知道 `editor`、`todo` 等专属命令的正确调用方式。 |
| file reference 通过 workspace host 读取 |  |  | `coding-definition.test.ts` | 防止引用展开绕过 Host，或模型拿不到用户指定文件内容。 |
| file reference 拒绝 workspace root 外路径 |  |  | `coding-definition.test.ts` | 防止通过 reference 读取工作区外文件。 |
| definition dispose 清理 environment shell sessions |  |  | `coding-definition.test.ts` | 防止关闭 coding agent 后 shell 进程继续运行。 |
| reference resolution 与 AgentSession send 顺序集成 |  |  | `coding-definition.test.ts` 通过 `AgentSession` 断言 provider request 和 transcript 都保存展开后的 file content | 需要发现 coding reference 在真实 session 中是否会先写入未展开内容。 |

### 5.13 Editor Command

Owner：`packages/agent-coding`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| `editor create` 用 heredoc 创建文件 |  |  | `packages/agent-coding/src/__tests__/editor-command.test.ts` | 防止模型写文件时多行内容、引号或换行被 shell 参数破坏。 |
| editor 拒绝 workspace root 外路径 |  |  | `editor-command.test.ts` | 防止编辑命令越权修改工作区外文件。 |
| patch escaped path 时写入前拒绝 |  |  | `editor-command.test.ts` | 防止 unified diff 中一个恶意路径导致部分文件已修改后才失败。 |
| `editor edit` exact replace 和 ambiguous matches 失败 |  |  | `editor-command.test.ts` | 防止错误替换多个位置或在歧义情况下误改代码。 |
| context disambiguation 只在唯一最近匹配时生效 |  |  | `editor-command.test.ts` | 防止模型提供上下文后仍改到错误位置。 |
| empty old text 拒绝且不修改文件 |  |  | `editor-command.test.ts` | 防止空匹配导致在文件所有位置插入内容。 |
| unified diff patch、timestamp headers、多文件创建/删除 |  |  | `editor-command.test.ts` | 防止常见 patch 格式无法应用，或删除/新增文件语义错。 |
| patch 全量校验后再写入，保证跨文件事务 |  |  | `editor-command.test.ts` | 防止 patch 中后续文件失败时前面文件已经被部分修改。 |

### 5.14 Todo Command

Owner：`packages/agent-coding`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| `todo add/list/update/done` raw output |  |  | `packages/agent-coding/src/__tests__/todo-command.test.ts` | 防止模型拿不到可读的任务状态反馈。 |
| `todo add/list/update/done` JSON output |  |  | `todo-command.test.ts` | 防止 agent 或 UI 需要结构化 todo 状态时解析失败。 |
| todo 状态按 agent session id 隔离 |  |  | `todo-command.test.ts` | 防止不同会话共享 todo，造成用户任务串线。 |
| todo 与 shell id 不混淆 |  |  | `todo-command.test.ts` 覆盖 agent session 隔离和同一 agent session 下 shell 重建后的 storage 延续 | 需要发现 shellId 和 agentSessionId 再次混淆。 |

### 5.15 Coding Agent 工作流

Owner：`packages/agent-coding`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| StubProvider 通过 shell tools 驱动 editor/todo，真实写文件 |  |  | `packages/agent-coding/src/__tests__/coding-marathon.test.ts` | 能发现 AgentSession、shell tools、registered command、Host 写文件之间的集成断裂。 |
| workflow 中复用同一个 shell session |  |  | `coding-marathon.test.ts` | 防止模型后续命令丢 cwd/env 或拿不到之前的 shellId。 |
| workflow 后文件内容正确 |  |  | `coding-marathon.test.ts` | 防止工具调用看似成功但真实 artifact 没写对。 |
| workflow 后 todo 状态在 agent session 下可读 |  |  | `coding-marathon.test.ts` | 防止 workflow 中 todo 写到了 shell-local 或全局错误位置。 |
| 创建文件 -> 测试失败 -> 读取错误 -> 修复 -> 测试通过 |  |  | `coding-marathon.test.ts` | 用来发现 agent 是否能围绕失败反馈迭代，而不是只验证单步命令。 |
| 长命令 running/yield -> wait -> input/abort 的 agent 级流程 |  |  | `coding-marathon.test.ts` | 用来发现 shell 控制面在模型多轮工具调用中是否仍保持正确上下文。 |
| tool error 后模型恢复继续执行任务 |  |  | `coding-marathon.test.ts` 覆盖命令非零退出后读取、修复、继续；base-agent 覆盖 invoke throw | 用来发现 coding 命令失败后模型是否有足够上下文修正，而不是 session 停死。 |
| 多轮 user message 对 coding workflow 的影响 |  |  | `coding-marathon.test.ts` 覆盖两次 `session.send` 后继续读取上一轮文件、tool result 和 todo 状态 | 用来发现 queued send、transcript 和 coding state 在连续用户输入下是否错序。 |
| 多 shell + 同 agent session 的 todo/storage 一致性 |  |  | `todo-command.test.ts` 覆盖同一 agent session 重建 shell 后仍读取同一 todo storage；`coding-marathon.test.ts` 覆盖 agent workflow 内 todo 状态延续 | 用来发现 shellId 和 agentSessionId 再次混淆。 |

### 5.16 RPC 协议

Owner：`packages/rpc`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| JSON codec 保留 BigInt metadata 和 Uint8Array |  |  | `packages/rpc/src/__tests__/json-codec.test.ts` | 防止跨进程/网络传输后 metadata 或二进制数据损坏。 |
| transcript patch 更新 in-place tool_call metadata/status |  |  | `packages/rpc/src/__tests__/patch.test.ts` | 防止 UI 收不到 tool_call 状态变化，只能靠全量刷新。 |
| transcript diff 处理非 JSON / cyclic metadata |  |  | `patch.test.ts` | 防止一个不可序列化 metadata 打断整个 RPC 同步。 |
| root entry 不导出 node-only stdio transports |  |  | `packages/rpc/src/__tests__/root-entry.test.ts` | 防止 browser/client bundle 意外包含 Node-only transport。 |
| RpcClient open/send 经 InProcessTransport 发 transcript/phase |  |  | `packages/rpc/src/__tests__/rpc.test.ts` | 防止基本协议动作不能驱动 session 或本地 view 不更新。 |
| client close 清空本地 transcript view |  |  | `rpc.test.ts` | 防止关闭会话后 UI 还显示旧 transcript。 |
| provider error code 只 forward 一次，并保留 transcript error block |  |  | `rpc.test.ts` | 防止 UI 重复报错，或错误只在 frame 中出现而 transcript 丢失。 |
| shell output、bash audit、generic tool progress frame 映射 |  |  | `rpc.test.ts` | 防止 TUI/GUI 看不到实时 shell 输出、审计事件或工具进度。 |
| shell_input frames 桥接到 active shell session tool |  |  | `rpc.test.ts` | 防止用户在壳子里输入内容但没有写入正在等待的进程。 |
| client `shellInput` 等待 result，未 open 时 reject |  |  | `rpc.test.ts` | 防止 UI 认为输入成功但实际上没有 active session。 |
| retry 产生 transcript patch removals |  |  | `rpc.test.ts` | 防止 retry 后 UI 留着已经被 runtime 截断的旧 assistant blocks。 |
| host queued send while busy 并按序 drain |  |  | `rpc.test.ts` | 防止外部壳子连续发送时 action 顺序与 session 执行顺序不一致。 |
| busy 时 host 拒绝 retry/resume/compact |  |  | `rpc.test.ts` | 防止外部壳子在 active run 中触发破坏性 mutation。 |
| client queued send promise 按各自 phase cycle resolve |  |  | `rpc.test.ts` | 防止多个 queued send 因同一个 idle 事件一起 resolve。 |
| error 后只 reject active action，queued send 继续 |  |  | `rpc.test.ts` | 防止一次 provider error 把后续排队用户消息全部错误取消。 |
| abort idle 返回 false，active 返回 true |  |  | `rpc.test.ts` | 防止 UI stop 按钮在 idle/active 状态下显示错误结果。 |
| close frame abort active session 并 dispose definition resources |  |  | `rpc.test.ts` | 防止关闭壳子后后台 run 或 shell environment 泄漏。 |
| stdio/WebSocket 基础传输和 RpcClient/RpcHost 端到端 |  |  | `stdio-transport.test.ts`、`websocket-transport.test.ts` | 防止协议只在 in-process 测试里成立，真实 transport 序列化后失败。 |
| stdio/WebSocket 下 queued send、abort、retry、resume、compact 与 in-process 一致 |  |  | `stdio-transport.test.ts`、`websocket-transport.test.ts` | 需要发现真实 transport latency/frame ordering 下的 action 收敛差异。 |
| close 时 active foreground process 通过真实 transport 被终止 |  |  | `stdio-transport.test.ts` | 需要防止壳子关闭后远端 session 进程继续占资源。 |

### 5.17 TUI

Owner：`packages/tui`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| 基本渲染、输入、scroll |  |  | 需要 TUI 自动化或 snapshot/integration 测试 | 防止核心能力可用但用户无法操作或看不到完整输出。 |
| thinking/text/tool output 显示 |  |  | 目前靠真实 TUI smoke 验收 | 防止真实模型输出被 TUI 分流、折叠或渲染错。 |
| 通过 RPC client open/send/receive phase/transcript/shell frames |  |  | RPC 层有候选覆盖；TUI 壳子自身待审查 | 需要发现 TUI 自己订阅、状态合并、刷新节奏的问题。 |
| 真实 Claude Code provider 输出真实模型回复 |  |  | 需要指定真实模型和 thinking 等级 smoke | 防止验收只跑 stub provider，没有确认真实 provider 路径。 |
| 交互式 shell 操作在 TUI 中顺畅 |  |  | 需要多次 smoke，因为模型行为有随机性 | 用来发现真实模型在模糊指令下是否会持续碰壁或误用 shell 控制面。 |

### 5.18 just-bash 子模块

Owner：`packages/just-bash`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| demi 依赖的 parser protection |  |  | `bun run test:just-bash-core` | 防止 parser 在安全边界或保护逻辑上退化。 |
| parser edge cases |  |  | `bun run test:just-bash-core` | 防止 agent 常见脚本语法在升级 just-bash 后突然解析错误。 |
| parse errors |  |  | `bun run test:just-bash-core` | 防止非法语法被误解析并执行。 |
| upstream bash/awk/sed/grep/jq 等 spec/comparison 测试 |  |  | 存在于子模块，不属于主仓库默认入口 | 用来发现主仓库关键路径以外的命令兼容性回归。 |

## 6. 当前剩余优先补测顺序

1. 先按第 5 章从上到下逐项审查测试有效性和文档对齐；未审查条目不得视为已覆盖。
2. TUI / 真实 provider smoke、just-bash 完整上游 spec 等候选 gated/部分范围项，也必须在逐项审查时明确记录原因。

## 7. 新增测试放置规则

- 单模块行为放在 owner package 的 `src/__tests__/`。
- 跨模块行为放在最接近不变量 owner 的 package：session/runtime 放 `base-agent`，coding workflow 放 `agent-coding`，协议收敛放 `rpc`。
- 涉及真实 CLI、真实模型、网络、本机登录状态或交互 UI 的测试必须 gated，不进入默认 `bun run test`。
- 测试应断言真实 artifact、provider request、transcript blocks、events、session state、RPC frames 或文件内容。
- 新增测试点必须说明能发现或规避的具体问题；说不清风险的问题不进入矩阵。
- 新增 architecture/workflow 约束时，先更新 `docs/agent-rewrite-plan.md` 或本文件，再补测试。

## 8. 测试有效性门禁

新增或重写测试时，必须先证明测试本身有意义。覆盖率、快照数量、mock 调用次数都不能单独作为有效性证据。

| 门禁 | 要求 | 适用位置 | 防止的问题 |
|---|---|---|---|
| 风险先行 | 每个测试先写清楚要捕获的失败模式，再写 fixture 和断言 | 所有测试矩阵项 | 防止为了覆盖率或已有实现形状凭空写测试。 |
| 产品边界 oracle | 优先断言 provider request、transcript、session state、RPC frame、真实文件、真实 shell output | agent/session/provider/RPC/coding workflow | 防止只验证 mock 自洽，真实用户路径仍然坏。 |
| 反证检查 | P0 测试落地后，用临时破坏实现或 fixture 的方式确认测试会失败 | compact、context cache、tool pair、marathon | 防止测试永远绿但不能发现实际回归。 |
| Mutation / fault injection | 人为注入 tool pair 断裂、summary 缺失、usage 错误、provider stream 异常、storage 丢 block | base-agent、provider、store/RPC | 防止只覆盖 happy path，错误路径没有有效 oracle。 |
| Property-based / stateful | 对 append-only transcript、queue、phase、tool_call 状态生成多步 action 序列并校验不变量 | AgentSession、Transcript、Shell control | 防止少量手写场景漏掉 action 交错和状态漂移。 |
| Metamorphic testing | 同一任务经过 retry/resume/compact/persist/reopen 后，模型可见上下文满足等价或明确变化规则 | replay、compact、context cache | 防止只测单一路径，无法发现等价操作后的上下文偏移。 |
| Differential / reference testing | shell 语义对比 just-bash/upstream 或真实 shell；provider message shape 对比参考 fixture | shell、provider conversion | 防止本地实现悄悄偏离已有可靠语义。 |
| Scenario / marathon | 单会话累计执行 send、queue、tool、error、abort、resume、retry、compact，并逐步断言不变量 | base-agent、coding workflow | 防止单点测试都过，但长任务累计后状态损坏。 |
| Golden snapshot 有结构化入口 | snapshot 只用于 request/frame 这类结构化 artifact，并配合字段级断言 | compact request shape、RPC transport | 防止快照变成不可审查的大文本批准。 |
| Gated smoke 只补充不替代 | 真实 Claude/TUI smoke 验证真实模型随机路径；默认 deterministic 测试仍负责契约 | TUI、真实 provider | 防止把不可复现的人工验收当成核心正确性证明。 |

P0 测试合入前至少要满足：有明确失败模式、有产品边界 oracle、有一次反证检查；涉及状态机或上下文压缩时，还要覆盖多步 action 序列或等价路径。

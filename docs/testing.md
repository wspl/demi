# 测试覆盖矩阵

| | |
|---|---|
| 日期 | 2026-06-19 |
| 阶段 | 第 5 章测试点已逐项审查；剩余工作见第 6 章 |
| 范围 | demi agent、shell、provider、AgentServer/transport、coding agent、REPL |

## 1. 审查结论定义

- 有效：测试断言直接命中文档测试点描述的失败模式，且对应命令通过。
- 部分有效：测试覆盖了部分风险，但关键断言、场景深度或产品边界 oracle 不足。
- 无效：测试存在但不能证明该测试点，或只验证 mock/实现细节自洽。
- 缺口：没有找到能证明该测试点的有效自动化测试。
- Gated：该测试点依赖真实 CLI、真实模型、网络、本机登录状态或交互 UI，不进入默认测试。

测试点必须说明它能发现或规避的问题。没有明确风险的测试点不应进入矩阵。

## 2. 测试判定原则

- 当前产品目标是最小但长期稳定的 agent runtime，不是完整 IDE/GUI agent 产品面。
- 核心链路是 AgentSession 状态机、Transcript replay、Provider 可见上下文、Tool call/result、Shell 控制面、Compaction、Context cache/usage、AgentClient/REPL 事件呈现。
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
| AgentSession 长生命周期稳定 | 单个 session 经过 send/queue/retry/resume/abort/tool/error/compact 后，phase、pending action、transcript、provider request 仍一致 | `agent` 单会话 marathon，关键步骤 exact provider request，统一 transcript invariant helper |
| 模型可见上下文稳定 | provider request 只来自 effective transcript 和当前 prompt context；内部块不泄漏；普通多轮 prefix 不无意义抖动 | `Transcript.collectInferenceItems` 复杂 fixture，连续 turn request prefix 比较，bounded injection 测试 |
| Compaction 可支撑长任务 | preflight/manual/auto compact 都可恢复；summary request 契约正确；不切 tool pair；失败/空 summary 不污染 transcript；多次 compact 只 replay 最新 boundary | `agent` compact P0 套件，覆盖 summary request、exact replay、failure atomicity、multi-compact、queue/persistence |
| Context cache baseline | cache usage 不丢；cache read/write 作为 provider 真实上下文 token 参与 auto compact；cache 字段本身不进入模型可见上下文；compact 后 request prefix 能重新稳定 | provider usage 映射、AgentSession/AgentClient usage 传播、compact 后 request 稳定性测试 |
| Shell 控制面支撑真实长命令 | 长进程走 foreground，wait/input/abort 语义稳定；不恢复空 input 轮询；不误报 needs_input | `shell` 单测、`coding-agent` 长命令 scenario、真实 provider/REPL gated smoke |
| Coding workflow 能发现真实问题 | 不是只验证单步命令，而是覆盖创建项目、测试失败、读取错误、修复、测试通过、tool error recovery | `coding-agent` scenario tests，断言真实文件、todo、shell result、provider request |
| 壳子路径能呈现真实模型行为 | REPL/AgentClient 能显示真实 text/thinking/tool output，真实 Claude Code provider 确实产出模型回复 | transport tests，REPL 自动化或 gated smoke |

### 3.2 当前门槛审查结论

审查要求：

- 旧的覆盖状态不再作为结论使用。
- 每个门槛必须在第 5 章对应测试点完成有效性审查后，才能回填结论。
- 默认测试命令通过只能证明测试可运行，不能单独证明测试有效或与文档对齐。

| 门槛 | 审查结论 | 审查记录 |
|---|---|---|
| AgentSession 长生命周期稳定 | 有效 | `5.4`、`5.5`、`5.6`、`5.7` 已覆盖 send/queue/retry/resume/abort/tool/error/compact 的主要路径，并覆盖官方 `AgentSession.fromSnapshot`、transcript/store snapshot 不被 live mutation 污染；provider realism 补测了 tool_call 后 provider 暂停、无同轮 response 的真实 Claude Code 时序。 |
| 模型可见上下文稳定 | 有效 | `5.5`、`5.7` 覆盖 exact transcript payload、effective transcript、当前 transcript schema 的 internal block 过滤、bounded injection、stable prefix 和 compact 后重稳。 |
| Compaction 可支撑长任务 | 有效 / Gated | `5.6` 是当前最强覆盖面，preflight/manual/auto、tool pair、multi-compact、snapshot restore、context overflow 裁剪重试、action 交错有效；真实 provider 下 medium thinking + budget 组合已有 gated 验证。2026-06-19 的 Haiku 4.5 REPL 长上下文验收最初发现 post-compact shell tool 不可用、重复 tool loop、MCP request id 复用污染 tool result 配对；对应修复已由 provider/agent/shell 测试锁住。真实 REPL 已分别复测 preflight/tool-heavy compact continuation 和 provider-usage auto compact：前者观察到 4 次 compact、至少 3 次 post-compact shell tool continuation 和 1 次 no-tool continuation；后者在同一会话中观察到 3 次 usage-based auto compact。Compact + Abort + Resume 真实验收又覆盖 abort 后 resume、usage-based auto compact、manual compact、post-compact shell follow-up 的同会话交错，并暴露 unsigned thinking replay 与 abort/error 重复渲染问题；记录见 `docs/repl-compact-haiku-acceptance.md` 和 `docs/repl-acceptance/compact-abort-resume.md`。 |
| Context cache baseline | 有效 / Gated | `5.3`、`5.7`、`5.16`、`5.17` 覆盖 usage 字段、stable prefix、cache usage 作为 context pressure 触发 auto compact、cache 字段不进入模型上下文、AgentClient transcript usage 传播和 REPL usage 呈现；真实 provider cache hit 已有 gated 验证，真实 REPL auto compact 已验证 cache-backed usage 超过阈值后触发 compact 并恢复。`docs/repl-acceptance/context-cache-stability-marathon.md` 又验证真实 REPL 中短 one-tool turns 可能报告 zero cache，但 multi-tool provider continuation 在 manual compact 前后都能报告非零 cache usage。 |
| Shell 控制面支撑真实长命令 | 有效 / Gated | `5.10` 和 `5.15` 覆盖 wait/input/abort、busy-default auxiliary shell、raw stdin/newline、hard-stop timeout 和 agent prompt guard；真实 REPL/model fuzzy shell-control 记录见 `docs/repl-acceptance/interactive-shell-control.md`，默认测试负责稳定契约，gated smoke 负责真实模型策略漂移。`docs/repl-acceptance/runaway-tool-loop-guard.md` 验证 repeated command guard 在真实 provider 下可见、可收敛，且后续用户 turn 不会被旧 control_request 吞掉。 |
| Coding workflow 能发现真实问题 | 有效 | `5.12` 到 `5.15` 覆盖真实文件、todo、测试失败到修复、长命令控制、shell cwd/env 复用、file reference Host 边界和 editor 写入失败事务。 |
| 壳子路径能呈现真实模型行为 | Gated | `5.16` AgentServer/transport 层有效，`5.17` REPL renderer/command/process 层已有 deterministic 测试；真实 Claude Code provider 回复、thinking、tool output 显示已有 gated smoke 入口。Compact + Abort + Resume 验收确认真实 REPL 在 abort/resume/compact 后仍能呈现模型输出和 shell 工具结果，并补上 abort/error patch 去重测试；Runaway Tool Loop Guard 又补上 errored tool output 的真实 REPL 可见性和去重测试。 |

### 3.2.1 长程 REPL Gated 验收设计

长程真实 REPL 验收的设计清单和每个测试项的过程记录模板见 `docs/repl-long-run-acceptance.md`。这些记录用于补充默认 deterministic 测试，重点发现真实模型、真实 provider、长上下文、shell 控制和 REPL/AgentClient action 收敛中的问题。

### 3.3 Compact 参考故事映射

agent-gui/Rust 的 compact 测试是当前最重要的校准对象；demi 不照搬 UI 产品面，但长任务能力相关故事都必须落到 `agent` 或协议层测试里。

| 参考故事 | demi 范围判定 | 候选落点 / 当前结论 | 能发现或规避的问题 |
|---|---|---|---|
| 新消息发给模型前先做 preflight compact | P0 | `5.6 Compaction`：有效 | 防止新用户输入已经把上下文推过上限后才请求真实 provider，直接触发 context overflow。 |
| usage 接近上限后 auto compact，再 resume 原动作 | P0 | `5.6 Compaction`：有效 | 防止长任务在接近 context limit 时直接失败，或 compact 后丢失当前 action。 |
| 用户手动 compact 成功后，summary 成为下一轮模型可见上下文 | P0 runtime；UI 呈现走壳子测试 | `5.6`、`5.7`：runtime 有效；REPL 呈现 gated | 防止 transcript 看似有 summary，但下一轮 provider request 没有使用它。 |
| summary provider stop/error/empty 都能恢复到一致状态 | P0 | `5.6 Compaction`：有效 | 防止失败 compact 留下半截 boundary/marker，或空 summary 抹掉旧上下文。 |
| tool 运行期间不 compact；cut point 不切断 `tool_use -> tool_result` | P0 | `5.6 Compaction`：有效 | 防止模型收到孤立 tool 历史，或 compact 后重复执行已完成工具。 |
| 多次 compact 只从 latest boundary replay，不把旧 summary 反复套入上下文 | P0 | `5.6 Compaction`：有效 | 防止摘要无限膨胀，或被压缩历史重新进入模型上下文。 |
| compact/preflight 期间 queued send、abort、retry、resume 收敛 | P0 | `5.4`、`5.6`：有效 | 防止用户在 summary 阶段继续操作时消息丢失、乱序或 session 卡在 compacting。 |
| 长上下文里 stop/continue/retry 后触发 compact | P0 | `5.4`、`5.6`：有效 | 防止 aborted text、completed tool result 或 retry 后的最新 user turn 在 summary 输入里丢失。 |
| 单个 turn 本身超过 recent budget，需要 split-turn summary | P0 | `5.6 Compaction`：有效 | 防止一个超长工具/assistant turn 找不到 cut point 后无法 compact，或硬切到 tool result 中间。 |
| compact 后 close/reopen，summary 和 replay 起点保持一致 | P0 persistence | `5.6`、`5.4` snapshot restore：有效 | 防止重启后 boundary/marker 丢失，模型重新看到旧长历史或看不到摘要。 |
| edit/replay 历史分支影响 summary | 当前无 `replay_from` 产品面；只保留 replay invariant | `5.5 Transcript 与 Replay` | 防止被删掉的历史 mutation 面重新进入设计，同时保证现有 replay 仍只取模型应见内容。 |
| 切换到更小 context 的模型后触发 compact | 当前无运行中切换模型产品面；保留 context limit contract | `5.7`：contract 有效；运行中切换不在范围 | 防止未来 provider/model 配置变化时仍按旧 context limit 请求模型。 |

### 3.4 模型上下文与 provider conformance 映射

Codex 与 pi agent 的参考价值主要在模型实际看到什么、异常 provider 输入如何收敛、完整历史与模型上下文如何分离。

| 参考能力 | demi 范围判定 | 候选落点 / 当前结论 | 能发现或规避的问题 |
|---|---|---|---|
| 模型可见上下文由一个中心 replay/effective context 产生 | P0 | `5.5`、`5.7`：有效 | 防止 provider、compact、retry 各自拼上下文，最终真实模型看到的内容不一致。 |
| 大 tool output、引用内容、preamble 有边界和截断策略 | P0 | `5.7`：有效 | 防止任意一次工具输出或文件引用直接撑爆 context，compact 也无法补救。 |
| cache read/write usage 从 provider 传到 session/AgentClient，但不改变 agent 行为 | 基线 | `5.3`、`5.7`：有效；真实 provider hit gated | 防止 cache 指标丢失，或把 provider cache 当成会影响 transcript/tool loop 的状态。 |
| compact 后历史重建与 live compaction 结果一致 | P0 | `5.4`、`5.6`：有效 | 防止 session 恢复后 replay 起点、summary 或 recent context 与内存态不一致。 |
| 完整审计日志与模型上下文分离 | P0 | `5.5`、`5.6`、`5.7`：有效 | 防止为了压缩模型上下文而删除完整历史，或把审计块误发给模型。 |
| provider 异常输入：empty、malformed、unicode/media、context overflow、tool result 缺失 | P0 provider 边界 | `5.3`：有效；真实 CLI gated | 防止真实 CLI/服务端返回边界事件时 agent 卡死、污染 pending state 或错误分类不可恢复。 |
| pre-turn/mid-turn/manual compact 的 request shape 有 snapshot 级断言 | P0 | `5.6`、`5.7`：有效，部分边界仍需 fault injection | 防止 incoming user 被重复放入请求、control-only item 被摘要、或 mid-turn continuation 丢失当前上下文。 |

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
- `packages/web-ui/src/agent/__tests__/pending-steers.test.ts`：覆盖 web 壳子的 pending steer
  呈现/回收规则，确保 `steer_result accepted` 与真实 transcript materialization 之间不会丢失
  半透明本地消息，也不会被历史同内容 steer block 误清理；render block 保留原始 pending
  steer id，供 hover X 删除精确取消对应 pending steer；queued message 来自 Vue reactive
  state 时也能安全 clone 成 pending steer 内容。
- `packages/web-ui/src/agent/__tests__/queue-submit.test.ts`：覆盖空 composer 提交时的 queued
  message 选择规则，确保 Enter 快捷选择队列最后一条，而不是队首或本地伪 queue。
- `packages/web-ui/src/agent/__tests__/input-editor.test.ts`：覆盖 composer editor 自身的
  keydown 判定和内容状态投影，确保 bare Enter 才触发提交，Shift+Enter / IME composition
  不会提交，且 steer/queue 按钮由 editor update 后的内容状态驱动。
- `packages/web/src/server/__tests__/transport.e2e.test.ts`：覆盖 web 后端只暴露 `/control`
  和 `/agent` WebSocket 能力，普通 HTTP 请求必须拒绝并指向 Vite dev server，防止静态
  `dist` / preview / production bundle 路径重新进入验收流程；同时覆盖启动时显式
  `--model` 会作为选中 provider 的首选模型进入 control catalog，避免 compatible
  endpoint 被官方默认 catalog 的第一项误选。

## 5. 模块测试点

### 5.1 平台与包边界

Owner：`packages/core`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| 根入口保持 browser-safe / runtime-neutral，不静态依赖 Node-only 能力 | 有效 | `platform-entrypoints.test.ts` 从 root entry 递归解析静态 import 闭包，并对 `node:` import/require、`Buffer`、`process.env/cwd`、显式 Node-only subpath 做负向断言；能直接发现 root 包把 Node-only 代码带入浏览器入口。验证：`bun test packages/core/src/__tests__/platform-entrypoints.test.ts`，13 pass。 | `packages/core/src/__tests__/platform-entrypoints.test.ts` | 防止浏览器或非 Node runtime 导入根包时因为 `node:*`、`process`、`Buffer` 直接崩溃。 |
| 只有 AgentServer 在运行时代码中直接实例化 `AgentSession` | 有效 | 测试扫描 `packages` runtime source，查找从 `@demi/agent` runtime import `AgentSession` 的文件，并断言唯一允许文件是 `packages/agent/src/server.ts`；能发现 UI/provider/业务包绕过 AgentServer 的 runtime 耦合。验证同上。 | `platform-entrypoints.test.ts` | 防止 UI、provider 或业务包绕过协议层调用 agent runtime，导致状态和事件边界失控。 |
| package manifest 不引入越层依赖 | 有效 | 测试读取 package manifests，断言 platform-neutral 包不依赖 concrete provider packages，`@demi/shell` 不声明越层依赖，Claude provider 不引入 Claude SDK；能发现依赖声明层面的隐藏耦合。验证同上。 | `platform-entrypoints.test.ts` | 防止平台无关包通过依赖声明暗中耦合 Node adapter、真实 provider 或 UI 包。 |
| 主仓库使用 forked just-bash package，不维护第二份上游源码快照 | 有效 | 测试检查禁止目录是否存在，并扫描 runtime imports 是否指向 vendor/upstream/旧 package 名；能发现仓库里出现第二份 just-bash 源或错误 import 来源。验证同上。 | `platform-entrypoints.test.ts` | 防止 bash engine 出现两个来源，造成修复只改一份、运行消费另一份的分叉问题。 |
| `@demi/core` 和 `@demi/provider` 不包含具体 provider 细节 | 有效 | 测试扫描两个平台基础包的 production source，禁止出现 concrete provider 包名、实现类名、catalog source label、backend 标识和产品名；能发现 `codex-backend`、`models.dev` 等细节重新进入公共 provider/core 契约。验证同上。 | `platform-entrypoints.test.ts` | 防止抽象层因字段、注释或 helper 泄漏具体 provider/source，导致低层包开始承载产品适配知识。 |
| production source 依赖图与 `docs/package-boundaries.md` 一致 | 有效 | 测试解析所有非测试源码的 workspace imports，构造包级有向图，断言不存在越过文档允许边的依赖和循环；能发现 provider 反向依赖 agent/shell、coding-agent 组装 runtime、或中立包依赖 REPL。验证同上。 | `platform-entrypoints.test.ts` | 防止包职责重新混乱，尤其是 provider、shell、agent、REPL 之间出现双向或越层依赖。 |
| production workspace imports 必须声明在 `dependencies` | 有效 | 测试把 production source 的 workspace import 与 package manifest 对齐，只接受 `dependencies`，不接受藏在 `devDependencies` 或依赖传递里；能发现源码已经使用某包但 manifest 没表达真实运行依赖。验证同上。 | `platform-entrypoints.test.ts` | 防止发布或隔离安装时因为隐式依赖缺失而失败，也防止依赖图被 manifest 掩盖。 |
| provider 公共根导出不泄漏内部实现 | 有效 | 测试检查 `@demi/provider`、`@demi/provider-claude-code`、`@demi/provider-codex`、`@demi/provider-openai-api`、`@demi/provider-anthropic-api` 的 root index，禁止 testing helper、wildcard export、transport/parser/protocol/auth-store/cache reset helper/runtime class 等内部 API 从公共根入口泄漏。验证同上。 | `platform-entrypoints.test.ts` | 防止包外代码把实现细节当成稳定 API，后续重构 provider transport、auth store 或 API stream mapper 时被公共契约绑死。 |

### 5.2 Provider 抽象

Owner：`packages/provider`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| `defineProvider` 暴露 public provider 并隐藏 runtime factory | 有效 | `provider.test.ts` 通过 `defineProvider` 创建 provider，断言公开字段可枚举、runtime factory 不进入 JSON 序列化，并通过 `providerRuntime` 实际运行 StubProvider 输出；能发现 runtime factory 泄漏到 browser-visible DTO 或无法由 AgentServer 创建 live runtime。验证：`bun test packages/provider/src/__tests__/provider.test.ts packages/provider/src/__tests__/stub.test.ts`。 | `packages/provider/src/__tests__/provider.test.ts` | 防止 `ProviderDefinition`/registry 概念重新成为用户装配 API，也防止 secret-bearing provider closure 被序列化。 |
| `applyModelPolicy` 过滤模型并重写 provider id | 有效 | `provider.test.ts` 构造 source catalog，断言 include/exclude/default 生效，且返回的 list/model providerId 被 remap 到 public provider id；能发现自定义 provider id 下模型选择错配。验证同上。 | `provider.test.ts` | 防止 UI 看到的 provider id 与 model catalog 内部 providerId 不一致，导致 open/set_provider 被 AgentServer 拒绝。 |
| `StubProvider` 多轮 scripted events | 有效 | 测试连续调用 `run()` 两次并断言每轮输出不同 scripted events 和 usage；能发现 StubProvider 没有按 turn 推进，导致 scenario test 不可复现。验证同上。 | `packages/provider/src/__tests__/stub.test.ts` | 防止 agent scenario test 依赖真实模型随机性，保证状态机测试可复现。 |
| `StubProvider` function script 能读取 request 并驱动 tool roundtrip | 有效 | 测试第二轮 function script 检查 request.items 中存在 `tool_result`，没有则主动失败；能发现 agent/provider scenario 没有把 tool result 回灌到下一轮 request。验证同上。 | `stub.test.ts` | 能发现 provider request 内容错误、tool_result 没有回灌、下一轮上下文不正确等问题。 |
| `StubProvider` 脚本耗尽时报错 | 有效 | 测试消费唯一 turn 后再次 `run()` 并断言抛出 `ran out of turns`；能发现意外额外 provider turn、retry/resume/loop 不会静默通过。验证同上。 | `stub.test.ts` | 防止测试悄悄多跑一轮但仍然通过，暴露意外 retry/resume/loop。 |

### 5.3 Claude Code Provider

Owner：`packages/provider-claude-code`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| Claude Code provider definition 不做外部 CLI preflight | 有效 | `provider.test.ts` 断言 provider definition 的 `auth.status()` 和 `state()` 只返回非预检状态，不调用 `which claude`、`claude --version` 或 `claude auth status`；真实可用性由 provider transport 在请求运行时暴露。 | `packages/provider-claude-code/src/__tests__/provider.test.ts` | 防止 UI、registry 或模型目录查询为了显示状态而额外 spawn Claude CLI，导致启动慢、误报或依赖本机 CLI 输出格式。 |
| Claude Code 模型目录来自 `models.dev` | 有效 / Gated | `models.test.ts` 用 `models.dev` fixture 覆盖 Claude full id 解析、`minimumModelVersion = "4.6"` 过滤、unknown 5.x family 保留、`reasoning_options[type="effort"].values[]` 顺序保留、future effort id 保留、缺失 metadata 映射为 `null`、ETag/Last-Modified 缓存、stale fallback 和无硬编码 fallback；真实 smoke 已用 `listClaudeCodeModels()` 请求 `https://models.dev/api.json`，本机返回 `claude-opus-4-7`、`claude-opus-4-8`、`claude-fable-5`、`claude-opus-4-6`、`claude-sonnet-4-6`。验证：同模型目录 targeted command，26 pass / 1 skip；真实 smoke 为手动命令。 | `packages/provider-claude-code/src/__tests__/models.test.ts`、`docs/provider-model-catalog-design.md` | 防止仓库维护过期 Claude 模型表、别名映射、丢弃未来 reasoning effort，或按名字臆测能力；也防止网络失败时伪造可选模型。 |
| Claude CLI args/env 构造契约 | 有效 | 测试精确断言 `--print`、stream-json input/output、partial messages、no session persistence、safe mode、disabled slash commands、empty tools、permission bypass、model、system prompt、thinking effort 和 env 清理；`buildClaudeArgsForRequest` 还断言 summary request 的 medium thinking effort 会映射到 `--effort medium`。验证同上。 | `cli.test.ts` | 防止模型、thinking、MCP/config 等关键启动参数在 CLI 调用时丢失或拼错。 |
| demi inference items 转 Claude JSONL input messages | 有效 | `jsonl-output.test.ts` 构造 user/assistant/tool_result items，断言生成三条 stream-json message、tool_result 以 Claude user message 结构写入，并能 JSONL roundtrip；新增 unsigned assistant thinking 覆盖，断言 `signature: null` 或空签名不会进入 Claude replay，单项转换返回 `null`。验证同上。 | `packages/provider-claude-code/src/__tests__/jsonl-output.test.ts` | 防止 user/assistant/tool 历史转换错位，真实模型看到的上下文与 transcript 不一致；同时防止 abort/resume 后把 unsigned thinking 发给 Claude Code，触发 `thinking.signature` API 400。 |
| assistant turn 与 tool result grouping | 有效 | 测试同一 assistant turn 内 thinking/text/两个 tool_use 被合并成一条 assistant message，随后两个 tool_result 被合并成一条 user message，之后的新 assistant text 开新 message；能发现 Claude 所需顺序被拆乱。验证同上。 | `jsonl-output.test.ts` | 防止 Claude 所需的 assistant/tool_result 顺序被拆乱，引发 tool_use 无匹配结果。 |
| binary media content 转 Claude base64 source | 有效 | 测试 image binary、image URL、document binary 同时输入，断言二进制 image/document 转 Claude base64 source、URL 保留、document title 保留，并 JSONL roundtrip；能发现 media 编码损坏。验证同上。 | `jsonl-output.test.ts` | 防止图片或二进制输入在 provider 边界损坏。 |
| Claude stdout 映射 text/thinking/tool call/usage | 有效 | 测试 assistant text/tool_use、stream text delta、result usage，以及空 thinking 边界事件；断言输出 `text_delta`、`thinking_*`、`tool_call_requested`、`response.usage` 的 demi event 结构。验证同上。 | `jsonl-output.test.ts` | 防止真实模型输出被吞、thinking 不显示、tool_call 不触发或 usage 统计失真。 |
| cache read/write usage 字段解析 | 有效 | `jsonl-output.test.ts` 和 `provider.test.ts` 都断言 `cache_read_input_tokens`、`cache_creation_input_tokens` 映射到 `cacheReadTokens`、`cacheWriteTokens`，能发现 provider 已返回 cache 指标但 demi 丢字段。验证同上。 | `jsonl-output.test.ts`、`provider.test.ts` | 防止 provider 已返回 cache 指标但 demi 丢字段，影响后续 cache contract 判断。 |
| malformed assistant tool_use / control_request 报错并不污染 pending state | 有效 | `jsonl-output.test.ts` 断言缺 id/name 的 assistant tool_use 变成 error event；`provider.test.ts` 断言 malformed assistant tool_use 不写 control_response，legacy 和 SDK MCP malformed tools/call 都写 error response 后继续读后续 assistant/result，未进入 pending。验证同上。 | `jsonl-output.test.ts`、`provider.test.ts` | 防止异常 provider 消息让 session 永久等待不存在的 tool_result。 |
| control_request、SDK MCP control_request、assistant tool_use 跨 run 状态机 | 有效 | `provider.test.ts` 分别覆盖 legacy control_request、SDK MCP control_request、assistant tool_use 三条路径；第一轮只产出 tool call，第二轮必须带 matching `tool_result` 才继续读 text/response，并断言写回格式。验证同上。 | `packages/provider-claude-code/src/__tests__/provider.test.ts` | 防止 Claude Code 多轮工具协议在 run 边界丢失 pending call 或重复提交结果。 |
| 缺失 tool_result / pending control_request 收敛为 provider error | 有效 | 测试 assistant tool_use 和 control_request 两种 pending 在第二轮缺少 matching `tool_result` 时都 reject，并断言 transport 被 kill/wait、control_request 不写成功 response；能发现协议断裂后继续消费 Claude 输出或永久等待。验证同上。 | `provider.test.ts` | 防止协议断裂时 agent 卡死或继续用不完整上下文调用模型。 |
| CLI 非零退出、stdout 迭代失败、abort 的 transport 清理 | 有效 | `provider.test.ts` 分别断言无 result 的非零退出产出 error，stdout iterator 抛错后 kill/wait 并下一轮可用新 transport，abort hanging run 后 kill/wait；能发现 transport 泄漏或坏 active 状态复用。验证同上。 | `provider.test.ts` | 防止 CLI 进程泄漏、后续 run 复用坏 transport，或错误无法传回 session。 |
| context overflow / rate limit / auth expired 等 provider 错误分类 | 有效 | 测试证明 `type:error` 的 explicit code 会透传、缺 code 的 rate-limit 文本会归类为 `rate_limit`，`result.is_error` 会合并 result/errors 文本、保留 usage，并把 context/rate/auth 文本分别归类为 `context_length_exceeded`、`rate_limit`、`auth_expired`；provider 集成测试断言 context overflow result error 带 code 转发。验证同上。 | `jsonl-output.test.ts`、`provider.test.ts` 覆盖 error code 透传、文本分类、result error 文本、context overflow result error | 防止真实 provider 给出可恢复或需用户处理的错误时，session 只得到不可行动的 generic error。 |
| empty assistant content、empty stream、空 thinking 事件 | 有效 | `jsonl-output.test.ts` 断言 empty assistant content 不产出事件、缺字段 text/thinking/redacted_thinking 映射为空字符串事件；`provider.test.ts` 断言空 stream 不泄漏 transport 状态且下一轮正常。验证同上。 | `jsonl-output.test.ts`、`provider.test.ts` | 防止 provider 返回空内容时误判成功、插入无意义 response，或 compact empty summary 污染 transcript。 |
| unicode surrogate、超长 JSONL 字段、media + tool_result 混合输入 | 有效 | 测试包含 Unicode separator、snowman、surrogate emoji、10k 长文本和 text+image tool_result，断言文本保留、image tool output 转标记、JSONL 包含长字段；能发现编码或 grouping 破坏。验证同上。 | `jsonl-output.test.ts` | 防止 JSONL 编码、base64、tool result grouping 在真实边界输入下损坏。 |
| 与 `AgentSession` 和 shell tools 的 provider 集成 | 有效 | `provider.test.ts` 用真实 `AgentSession`、`BashEnvironment`、shell tools 和 fake Claude control_request 跑完整 tool loop，断言 transcript 为 user/tool_call/text/response，shell 结果写回 control_response；能发现 provider event、session tool loop、shell 输出格式接口不匹配。验证同上。 | `provider.test.ts` | 能发现 provider event、AgentSession tool loop、shell tool result 三者之间的接口不匹配。 |
| 真实 Claude CLI e2e | Gated | `real-cli.e2e.test.ts` 的最小真实 CLI streaming response 只有 `DEMI_CLAUDE_CODE_E2E=1` 时启用；默认 targeted command 中 skip。它能验最小真实 CLI、账号、网络和真实输出格式，但不进入默认稳定测试。 | `real-cli.e2e.test.ts`，不属于默认稳定测试 | 用来发现 fake transport 无法覆盖的本机 CLI、账号、网络、真实输出格式变化。 |
| 真实 Claude medium thinking e2e | Gated | `real-cli.e2e.test.ts` 新增 `DEMI_CLAUDE_CODE_THINKING_E2E=1` gated smoke：用 `claude-opus-4-8`、medium thinking 发起 summary-shaped request；每次 run 断言无 provider error、无 tool call、真实 text delta 拼接后包含 `DEMI_THINKING_BUDGET_OK`、response usage 有 input/output tokens；默认两次 run 中至少一次必须出现非空 thinking delta、thinking signature 或 redacted thinking。验证通过：`DEMI_CLAUDE_CODE_THINKING_E2E=1 bun test packages/provider-claude-code/src/__tests__/real-cli.e2e.test.ts --timeout 240000`，1 pass / 2 skip。 | `real-cli.e2e.test.ts` gated smoke，不进默认测试 | 用来发现真实 Claude Code CLI 在 medium thinking 下不再返回可映射的 thinking 信号、response usage 或 exact model text。 |
| 真实模型 thinking/tool use/text 输出验收 | Gated | `real-repl.e2e.test.ts` 在 `DEMI_REPL_REAL_E2E=1` 时用真实 REPL + Claude Code 跑 `claude-opus-4-8` / medium；每次 run 必须看到 tool、tool output、assistant text、usage，默认两次 run 中至少一次必须看到 `thinking>`。本机验证通过：`DEMI_REPL_REAL_E2E=1 bun test packages/repl/src/__tests__/real-repl.e2e.test.ts --timeout 360000`，1 pass。默认测试仍 skip。 | `real-repl.e2e.test.ts` gated smoke | 用来确认最终用户路径确实看到真实模型回复、thinking 和 tooluse，而不是只验证 mock。 |

### 5.3.1 Codex Provider

Owner：`packages/provider-codex`

验证：`bun test packages/provider-codex/src --timeout 120000`，27 pass / 5 gated skip。

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| 官方 Codex `auth.json` 解析与状态脱敏 | 有效 | `auth.test.ts` 构造官方形状的 ChatGPT token auth，断言 account/email/fedramp claim、auth status 和 access token 解析；另测 API key、缺失 auth 和 redaction，不把 secret 带入 status/error。 | `packages/provider-codex/src/__tests__/auth.test.ts` | 防止 provider 自建登录或读错官方 auth storage，也防止 token 进入 UI、日志或测试快照。 |
| ChatGPT token refresh 写回契约 | 有效 | `auth.test.ts` 用 near-expiry access token 触发 refresh，断言使用 refresh token、写回新 token、保留未知字段、更新 `last_refresh`、文件权限保持 `0600`；`provider.test.ts` 覆盖真实请求 401 后强制 refresh 并重试一次。 | `auth.test.ts`、`provider.test.ts` | 防止 access token 过期后只能让用户重新登录，或 refresh 破坏官方 Codex 的 auth 文件。 |
| Codex 模型目录来自官方 backend | 有效 / Gated | `models.test.ts` 覆盖 `auth.json` ChatGPT/PAT/agentIdentity header、默认静态 `client_version=0.130.0`、显式 `clientVersion` override、401 force refresh 后重试、API key auth 不请求 ChatGPT catalog、backend `slug` 映射为 full model id、capability metadata、future reasoning effort、`service_tiers` 映射和 stale cache；真实 smoke 已用本机 `~/.codex/auth.json` 请求 `https://chatgpt.com/backend-api/codex/models`，本机返回 `gpt-5.5`、`gpt-5.4`、`gpt-5.4-mini`、`gpt-5.3-codex-spark`、`codex-auto-review`。 | `packages/provider-codex/src/__tests__/models.test.ts`、`docs/provider-model-catalog-design.md` | 防止 Codex provider 自建鉴权、用错模型 id 字段、把 ChatGPT catalog 套到 API key auth、运行时依赖 `codex --version`、丢弃未来 reasoning effort、把 legacy `fast` alias 当 service tier，或在 backend 不可用时返回仓库硬编码模型。 |
| Responses request conversion 与 prompt cache key | 有效 | `responses.test.ts` 精确断言 system prompt、user text/image、assistant text、signed thinking、tool_use、tool_result、tools、medium reasoning、`include: ["reasoning.encrypted_content"]`、`store:false`、`stream:true` 和 `prompt_cache_key=sessionId`；unsigned thinking 会被跳过；另测未选 service tier 时省略 `service_tier`，选中 `priority` 时写入 `service_tier: "priority"`。 | `responses.test.ts` | 防止模型可见上下文、thinking replay、tool pairing、service tier 或 cache 亲和在 provider 边界偏离官方 Responses schema。 |
| Responses stream 映射 text/thinking/tool/usage/error | 有效 | `responses.test.ts` 覆盖 reasoning start/delta/signature、final raw reasoning fallback、text delta、function call args delta/done、completed usage、cache read token 扣减、failed/incomplete/error；`provider.test.ts` 又通过 fake transport 跑同一事件链。 | `responses.test.ts`、`provider.test.ts` | 防止真实模型输出被吞、thinking 不可见、tool call 不触发、cache usage 丢失或 context/rate/auth 错误无法被 agent 分类恢复。 |
| SSE parser 与 WebSocket fallback/headers | 有效 | `responses.test.ts` 覆盖 SSE data chunk；`provider.test.ts` 覆盖 WebSocket connect 前失败 fallback SSE、已开始输出后失败不 fallback，避免重复输出；WebSocket transport 测试断言删除 SSE-only headers 并使用 `responses_websockets=2026-02-06` beta header；新增 completed-without-close 测试锁住真实 `auto` smoke 暴露的问题，收到 `response.completed` 后必须主动结束 stream。 | `responses.test.ts`、`provider.test.ts` | 防止传输层把 SSE/WebSocket 协议混用，或 WebSocket 中途失败后重复同一轮模型输出；同时防止真实 WebSocket 已完成但连接不 close 时 REPL 卡到 timeout。 |
| ChatGPT/OpenAI route 与 provider config 白名单 | 有效 | `provider.test.ts` 断言 ChatGPT/PAT 路由到 `chatgpt.com/backend-api/codex/responses`、API key 路由到 `api.openai.com/v1/responses`，并断言 public provider definition 只接受可序列化 config，测试注入不能经协议 config 泄漏。 | `provider.test.ts` | 防止 AgentServer/transport 接收不可序列化 provider 实例，也防止 auth mode 路由错误导致真实请求打到错误 backend。 |
| 与 `AgentSession` 和 shell tools 的 provider 集成 | 有效 | `provider.test.ts` 用真实 `AgentSession`、`BashEnvironment`、shell tools 和 fake Codex Responses function call 跑完整 tool roundtrip，断言 `tool_result` 被写回下一次 Responses input，shell 输出进入 transcript。 | `provider.test.ts` | 能发现 provider event、Responses tool id、AgentSession tool loop、shell tool result 四者之间的接口错位。 |
| provider-stream steer 的同 turn continuation | 有效 | `provider.test.ts` 用 gated Codex transport 阻塞第一轮 provider stream，在 active run 期间同时提交 queued send 与 steer，断言第二个 Responses request 中当前 assistant output 位于 `steer current` 之前且不包含 queued turn，queued send 只在 active turn 两次 provider request 完成后进入第三个 request。验证：`bun test packages/provider-codex/src/__tests__/provider.test.ts`，9 pass。 | `provider.test.ts` | 防止把 steer 错误排到下一 turn、抢到当前 sampling output 前面，或为了支持 steer 发明未验证的 Responses WebSocket 控制事件，导致当前 turn continuation、queue 顺序和 provider request 历史错位。 |
| 平台包边界不引入 Codex provider | 有效 | `platform-entrypoints.test.ts` 把 `@demi/provider-codex` 加入 platform-neutral 禁止依赖清单；根包静态 import 闭包不会把 Node-only auth/fetch/WebSocket provider 带入浏览器入口。 | `packages/core/src/__tests__/platform-entrypoints.test.ts` | 防止新增真实 provider 后污染 core/provider/agent 这些平台无关入口。 |
| 真实 Codex e2e | Gated；已手动验证 | `real-codex.e2e.test.ts` 默认 skip；`DEMI_CODEX_E2E=1` 验证本机官方 Codex auth、真实 Responses route 和真实文本输出；`DEMI_CODEX_THINKING_E2E=1` 验证 medium thinking、thinking signal 和 usage；`DEMI_CODEX_CACHE_E2E=1` 验证真实 cache read usage；`DEMI_CODEX_TOOL_E2E=1` 通过真实 AgentSession + shell tool 验证真实 function call roundtrip；`DEMI_CODEX_STEER_E2E=1` 通过真实 AgentSession + gated tool 验证 active turn 被 steer 后先于 queued turn 完成，验收记录见 `docs/repl-acceptance/codex-steer-queue-interleaving.md`。真实验收发现并修复两点：cache e2e 不能要求第一轮 cache write，真实 Codex 在第二轮给 `cacheReadTokens`；`auto` transport 的 WebSocket 路径必须在 `response.completed` 后主动结束。验证：`DEMI_CODEX_E2E=1 DEMI_CODEX_THINKING_E2E=1 DEMI_CODEX_TOOL_E2E=1 DEMI_CODEX_CACHE_E2E=1 DEMI_CODEX_TRANSPORT=auto bun test packages/provider-codex/src/__tests__/real-codex.e2e.test.ts --timeout 600000`，4 pass。SSE 同命令设置 `DEMI_CODEX_TRANSPORT=sse` 也为 4 pass。Steer/queue 验证：`DEMI_CODEX_STEER_E2E=1 DEMI_CODEX_TRANSPORT=sse bun test packages/provider-codex/src/__tests__/real-codex.e2e.test.ts --timeout 420000`，1 pass / 4 skip。 | `real-codex.e2e.test.ts` gated smoke，不进默认测试 | 用来发现 fake transport 无法覆盖的账号、网络、真实模型、真实 Responses event、thinking/cache/tool 输出变化，以及真实 provider 路径下 queue 与 steer 的 active-session 输入策略错序。 |

### 5.3.2 OpenAI API Provider

Owner：`packages/provider-openai-api`

验证：`bun test packages/provider-openai-api/src/__tests__/provider.test.ts`，7 pass。

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| endpoint/env/api key 解析 | 有效 | `provider.test.ts` 通过真实 provider runtime + fake fetch 断言默认 `OPENAI_BASE_URL`、`OPENAI_API_KEY` 会被读取，默认 URL 拼为 `{baseUrl}/responses`；另测 `wireApi: "chat-completions"` 下显式 `baseUrl`/`apiKey` 优先于自定义 `envPrefix` env vars，并保留 custom header。 | `packages/provider-openai-api/src/__tests__/provider.test.ts` | 防止 API provider 只能硬编码官方 endpoint，或把 envPrefix/baseUrl/apiKey/wireApi 优先级做反。 |
| Responses request/stream conversion | 有效 | 测试直接断言 system/user/assistant/tool_use/tool_result、function tool schema、service tier、reasoning、`store:false`、`include:["reasoning.encrypted_content"]` 和 `prompt_cache_key` 的 Responses body；stream 测试覆盖 thinking、split text、function call args 和 usage。 | `provider.test.ts` | 防止官方 OpenAI 默认路径没有走 Responses schema，或模型可见上下文、thinking replay、tool pairing、usage 映射错位。 |
| Chat Completions request conversion | 有效 | 测试直接断言 `wireApi: "chat-completions"` 使用 system/user/assistant/tool_use/tool_result、tool schema、service tier、reasoning effort 和 `stream_options.include_usage` 的 request body。 | `provider.test.ts` | 防止兼容 endpoint 路径的模型可见上下文、tool pairing、service tier 或 reasoning effort 在 OpenAI Chat Completions schema 上错位。 |
| Chat Completions stream 映射 text/tool/usage | 有效 | 测试 split `choices[].delta.content`、split `tool_calls[].function.arguments`、`finish_reason=tool_calls`、final usage 和 `[DONE]`，断言输出 Demi `text_delta`、`tool_call_requested` 和 `response.usage`。 | `provider.test.ts` | 防止真实 compatible streaming chunk 被吞、tool args 分片丢失、cache read usage 扣减错误或 response 终止事件缺失。 |
| malformed tool args 可预测降级 | 有效 | 测试 malformed JSON tool args 不抛异常，`input` 以原始 string 进入 `tool_call_requested`。 | `provider.test.ts` | 防止 provider 返回不完整工具参数时 agent stream 崩溃，或静默丢掉 tool call。 |
| Web secret boundary | 有效 | Web control path 只返回 `ProviderSelection`，provider runtime 通过闭包持有 `baseUrl`/`apiKey`/headers；transport e2e 断言 browser-visible prepare/open frame 不包含 `apiKey`、secret headers、`baseUrl`、`envPrefix` 或 raw provider options。 | `packages/web/src/server/__tests__/transport.e2e.test.ts` | 防止 API key 和 endpoint 配置经 Web UI 往返，或被 AgentClient frame 序列化。 |
| 真实 OpenAI API e2e | Gated；已手动验证 DeepSeek compatible | 预留 `DEMI_OPENAI_API_E2E=1`：最小文本、standard shell tool roundtrip、active steer fallback 三类真实验收。默认 deterministic 测试不依赖网络和账号。2026-06-25 手动用 DeepSeek `deepseek-v4-pro`、`baseUrl=https://api.deepseek.com`、`wireApi="chat-completions"` 验证 provider runtime 最小文本返回 `demi-provider-openai-ok`、工具流返回 `record_value({ value: "openai-tool-ok" })`，并通过 REPL 真实路径返回 `repl-openai-deepseek-ok`。 | 待新增 `real-openai-api.e2e.test.ts` | 用来发现 fake SSE 无法覆盖的真实账号、模型、API event shape、tool_call 分片和限流/错误分类变化。 |

### 5.3.3 Anthropic API Provider

Owner：`packages/provider-anthropic-api`

验证：`bun test packages/provider-anthropic-api/src/__tests__/provider.test.ts`，4 pass。

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| endpoint/env/api key 解析 | 有效 | `provider.test.ts` 通过真实 provider runtime + fake fetch 断言默认 `ANTHROPIC_BASE_URL`、`ANTHROPIC_API_KEY` 会被读取，URL 拼为 `{baseUrl}/messages`；另测显式 `baseUrl`/`apiKey`/`anthropicVersion` 优先于自定义 `envPrefix` env vars，并保留 custom header。 | `packages/provider-anthropic-api/src/__tests__/provider.test.ts` | 防止 API provider 只能硬编码官方 endpoint，或把 envPrefix/baseUrl/apiKey/version 优先级做反。 |
| Messages request conversion | 有效 | 测试断言 user/tool_result 与 assistant/tool_use 按 Anthropic role 分组，system、tools、service tier、max_tokens 和 budget thinking 进入 body。 | `provider.test.ts` | 防止 Anthropic Messages 对 assistant/tool_result 顺序要求被拆乱，引发 tool_use 无匹配结果或模型上下文漂移。 |
| Event stream 映射 thinking/text/tool/usage | 有效 | 测试覆盖 `message_start` usage、thinking start/delta/signature、text delta、tool_use input_json_delta、content_block_stop 和 `message_stop`，断言输出 Demi `thinking_*`、`text_delta`、`tool_call_requested` 和 `response.usage`。 | `provider.test.ts` | 防止真实 stream thinking 不显示、tool call 不触发、cache read/write usage 丢失或 response 终止事件缺失。 |
| Web secret boundary | 有效 | Web control path 只返回 `ProviderSelection`，provider runtime 通过闭包持有 `baseUrl`/`apiKey`/headers/version；transport e2e 断言 browser-visible prepare/open frame 不包含 `apiKey`、secret headers、`baseUrl`、`envPrefix` 或 raw provider options。 | `packages/web/src/server/__tests__/transport.e2e.test.ts` | 防止 API key 和 endpoint 配置经 Web UI 往返，或被 AgentClient frame 序列化。 |
| 真实 Anthropic API e2e | Gated；已手动验证 DeepSeek compatible | 预留 `DEMI_ANTHROPIC_API_E2E=1`：最小文本、standard shell tool roundtrip、active steer fallback 三类真实验收。默认 deterministic 测试不依赖网络和账号。2026-06-25 手动用 DeepSeek `deepseek-v4-pro`、`baseUrl=https://api.deepseek.com/anthropic` 验证 provider runtime 最小文本返回 `demi-provider-anthropic-ok`、工具流返回 `record_value({ value: "anthropic-tool-ok" })`，并通过 REPL 真实路径返回 `repl-anthropic-deepseek-ok`。真实 DeepSeek Anthropic-compatible 即使未请求 thinking 也会返回 thinking stream，当前 mapper 可正常呈现。 | 待新增 `real-anthropic-api.e2e.test.ts` | 用来发现 fake event stream 无法覆盖的真实账号、模型、API event shape、tool_use 分片和限流/错误分类变化。 |

### 5.4 Agent Session Runtime

Owner：`packages/agent`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| `send` 写入 user turn，再构造 provider request | 有效 | `session.test.ts` 在 provider function 内断言 `systemPrompt`、`cwd` 和 `request.items`，确认 user turn 先写入且 preamble 合并到同一 user message；随后断言 transcript 为 user/text/response。验证：5.4 targeted command，75 pass。 | `packages/agent/src/__tests__/session.test.ts` | 防止 provider request 漏掉用户输入、preamble 顺序错误，或 transcript 与模型上下文分叉。 |
| provider response 写入 transcript | 有效 | 同一 send 测试断言 provider 的 text/response events 写入 transcript，并检查 phase 回到 idle、transcript_changed 事件发出；能发现模型输出没有进入可见历史。验证同上。 | `session.test.ts` | 防止 UI/AgentClient 无法看到模型输出，或 retry/compact 基于缺失历史运行。 |
| provider error 写入 transcript、发 event、reject action | 有效 | 测试 StubProvider 返回 error event 后 `send` reject，transcript 写入 error block，session event 包含 `ProviderStreamError`，phase 回 idle；能发现错误只在 Promise 或只在 transcript 单边存在。验证同上。 | `session.test.ts` | 防止错误只在 Promise 中出现而 transcript/UI 不可见，或者 action 静默成功。 |
| provider error 后关闭 provider iterator | 有效 | `ClosingProvider` 在第一条 error 后若 iterator 被 `return()` 会设置标志；测试断言 `send` reject 后 `returned=true`，能发现 provider stream 未关闭导致后续残留事件。验证同上。 | `session.test.ts` | 防止 provider stream 泄漏，后续 run 收到旧 stream 的残留事件。 |
| provider iterator exception 归一为 transcript error，并继续处理队列 | 有效 | 新增 `ThrowingProvider` 在第一轮已输出部分 text 后模拟 transport throw，先反证旧实现只 reject Promise 而不写 error block；修复后断言 transcript 为 user/text/error/user/text/response、error code 保留、queued send 清空并完成。验证：`bun test packages/agent/src/__tests__/session.test.ts`，23 pass。 | `session.test.ts` | 防止真实 CLI/transport 中途断开时 UI 和恢复路径只看到半截 assistant text，看不到失败原因，或错误误杀后续 queued send。 |
| reference resolution 在写入 user turn 和 provider request 前执行 | 有效 | 测试用 `resolveReferences` 把 reference 转成 text，并同时断言 provider request 和 transcript user block 都只包含展开后的 content；能发现 transcript/request 保存未展开引用。验证同上。 | `session.test.ts` | 防止 transcript 保存未展开引用，导致恢复或重放时上下文不可复现。 |
| reference resolution 卡住时 abort 可收敛 | 有效 | 测试让 `resolveReferences` 返回永不完成的 Promise，abort 后 `send` 收敛、provider 未运行、transcript 只留下 abort，phase 回 idle；能发现引用解析卡死阻塞停止。验证同上。 | `session.test.ts` | 防止文件引用或外部解析卡住后 session 无法取消。 |
| provider 不 yield 时 abort 可收敛 | 有效 | `HangingProvider` 监听 cancel signal 后永不 yield；测试 abort 后 `send` 在 timeout 内收敛、cancelled=true、phase idle、transcript user/abort；能发现模型 stream 卡死时停止无效。验证同上。 | `session.test.ts` | 防止模型 stream 卡死时 UI 的停止按钮无效。 |
| long-running tool invocation 卡住时 abort 可收敛 | 有效 | 测试 tool invoke 永不 resolve，等 pending tool_call 出现后 abort，断言运行收敛、phase idle、最后写入 abort；能发现工具执行卡住占住 session。验证同上。 | `session.test.ts` | 防止工具实现挂起后占住 session，后续用户输入无法执行。 |
| tool call 执行后继续 provider roundtrip | 有效 | 测试第一轮 provider 发 tool_call，tool 执行后第二轮 provider function 断言 `request.items` 为 user/tool_use/tool_result，再返回最终 text/response；能发现 tool_result 未回灌。验证同上。 | `session.test.ts` | 防止 tool_result 没有回传给模型，模型无法基于工具结果继续回答。 |
| tool invocation throw 转成 error tool result 并继续 | 有效 | 测试 tool 抛错后第二轮 provider 断言 tool_result `isError=true` 且输出 `Tool failed: broken tool`，最终 transcript tool_call 为 error 且 pending 清空；能发现工具异常导致 session 崩溃。验证同上。 | `session.test.ts` | 防止工具异常升级成 session 崩溃，模型失去恢复机会。 |
| tool progress event 发出 | 有效 | 测试 tool invoke 调用 `ctx.emitProgress`，订阅 session events 并断言收到 tool_progress 的 id/name/payload；能发现工具进度只停留在内部。验证同上。 | `session.test.ts` | 防止 shell output/audit 等进度事件只留在内部，UI 无法实时显示。 |
| queued send 排队并按顺序 drain | 有效 | `GateProvider` 阻塞第一轮时发送第二条消息，断言 queuedMessages 先出现再清空，两轮 release 后 transcript user 文本顺序为 first/second；能发现 active run 期间输入乱序或丢失。验证同上。 | `session.test.ts` | 防止用户连续发送消息时顺序错乱，或后一条消息覆盖前一条运行状态。 |
| provider-stream steer 不需要 native provider hook | 有效 | `session.test.ts` 用无 `ProviderRun.steer` 的 gated provider，在 active provider stream 期间提交 queued send 与 steer，断言 accepted ack 后 steer 先保持 pending，当前 provider output 先进入 transcript，随后 steer block 在最近 provider boundary materialize 并触发同 turn provider continuation，queued send 保持等待；另用 slow snapshot store 证明 materialization 发生在当前 output 之后、下一次 request 之前；新增 provider-stream 产出 tool_call 的用例，断言 steer 在工具执行前 materialize，而下一次 request 仍按 `tool_use`、`tool_result`、`user_steer` 顺序重放；`server.test.ts` 断言同一路径经 AgentClient 先收到 accepted `steer_result`，再在当前 stream 结束后收到 steer transcript patch。验证：`bun test packages/agent/src/__tests__/session.test.ts packages/agent/src/__tests__/server.test.ts`。 | `session.test.ts`、`server.test.ts` | 防止 active steer 被误拒、被自动转成 queue、抢到当前 sampling output 前面、拖到整个任务完成后才插入、accepted ack 后没有 same-turn provider continuation，或慢持久化导致 continuation race。 |
| pending steer 删除/取消 | 有效 | `session.test.ts` 覆盖 accepted pending steer 在 materialization 前取消后不触发同 turn continuation、reference resolution 未完成时先收到 cancel 也不会随后 materialize；`server.test.ts` 覆盖 `AgentClient.cancelPendingSteer` 帧阻止 transcript steer patch 和额外 provider request。验证同上。 | `session.test.ts`、`server.test.ts` | 防止 web hover X 只删本地气泡但 runtime 仍把 steer 写进 transcript，或取消竞态导致已经删除的 steer 之后又出现。 |
| retry 截断最后 assistant response 并 rerun latest user turn | 有效 | 测试先产生 old response，再 retry；第二轮 provider 断言 request 只包含 latest user message，最终 transcript 为 user/new text/response 且旧 assistant 输出不存在。验证同上。 | `session.test.ts` | 防止 retry 把旧错误输出和新输出混在一起，或重跑了错误的用户 turn。 |
| resume 标记 abort 为 resumed，并追加 resume turn | 有效 | 测试手工构造 user+abort transcript，resume 后断言 abort `isResumed=true`，并追加 resume/text/response；能发现恢复历史无法区分已处理 abort。验证同上。 | `session.test.ts` | 防止恢复后 transcript 无法区分已中止内容和继续执行内容。 |
| abort 后 resume 前清理 pending tool calls | 有效 | 测试慢 tool pending 后 abort，断言 pending tool_call 被转成 error output 且 pending 清零；resume 的 provider request 必须包含 user/tool_use/error tool_result/resume user message，避免重复等待旧 tool。验证同上。 | `session.test.ts` | 防止恢复时模型看到仍在执行的旧 tool_call，重复等待或重复执行工具。 |
| mutation guard 拒绝 busy/reserved 期间 mutation | 有效 | 测试 `reserveMutation` 期间再次 reserve 抛错、run busy 期间 reserve 抛错；另一个测试在 external mutation reserved 时 send/retry/resume/compact 都 reject，且不改 phase、queue、events。验证同上。 | `session.test.ts` | 防止 compact/retry/resume 与 active run 交错修改 transcript，造成不可恢复状态。 |
| store snapshot 写入 | 有效 | 测试注入 MemorySessionStore 后 send，断言 store 收到 snapshots，最后 snapshot 包含 harnessName、cwd 和 user/text/response transcript；另测 store snapshot 的 state 是持久化边界深拷贝，后续 tool mutation 不会改写旧 snapshot，外部改 snapshot 也不会污染 live state。验证：5.4 targeted command，76 pass。 | `session.test.ts` | 防止进程退出或 host 重启后没有可恢复的会话状态，或旧持久化 snapshot 被 live state mutation 污染。 |
| extension state snapshot 通过 lifecycle 写入并持久化 | 有效 | 测试 after_tool_call lifecycle 写入 extension state snapshot，随后检查 store snapshot 中包含 `extension_state_snapshot`、extensionName 和 `{ toolCalls: 1 }`；能发现扩展状态只留在内存。验证同上。 | `session.test.ts` | 防止 todo 等 agent 扩展状态只在内存里存在，恢复后状态丢失。 |
| 从 store snapshot 重建 session 后继续运行 | 有效 | `session-marathon.test.ts` 覆盖旧的 transcript-only restore 不重复执行已完成工具；新增 `AgentSession.fromSnapshot` 官方 loader 测试，断言恢复 state、cwd、model、transcript replay，恢复后 runtime 为 idle 且不自动重放持久化 queue，后续 block 使用恢复时传入的 idFactory，外部修改 snapshot 不污染 restored session，并拒绝 harnessName mismatch。验证同上。 | `session-marathon.test.ts` | 需要发现 snapshot schema、harnessName、state、phase、queue、idFactory 或 transcript replay 在重启后不兼容的问题。 |
| provider error 后恢复不重复发送已完成 tool result | 有效 | 测试先完成一次非幂等 tool，再让后续 provider error 写入 snapshot；恢复后新请求中 tool_result 数量必须仍为 1，并用 `assertNoOrphanToolItems` 检查配对，state.toolCalls 不增加。验证同上。 | `session-marathon.test.ts` | 需要规避重启后重复执行破坏性工具或给模型重复上下文。 |
| abort/retry/resume/compact 组合交错 | 有效 | marathon 覆盖 tool、queued send、provider error、retry、abort、resume、manual compact、post-compact send 的连续状态；`compaction.test.ts` 还覆盖 compaction 期间 queue send/retry/resume，验证 queued action 在 summary commit 后按正确上下文运行。验证同上。 | `session-marathon.test.ts`、`compaction.test.ts` | 组合路径容易暴露单点测试发现不了的 phase、queue、transcript 原子性问题。 |
| 单会话 marathon 覆盖 send/queue/retry/resume/abort/tool/error/compact 累计状态 | 有效 | marathon 单测在同一 session 内跑 9 次 provider request，阶段性断言 request 与 transcript 一致、pending tools 清空、queue 清空、phase idle、invariants 成立；能发现长期累计后状态漂移。验证同上。 | `session-marathon.test.ts` | 用来发现状态只在单点测试里正确，长期累计后 id、phase、pending action、tool 状态或 transcript 顺序漂移。 |
| 每个关键步骤精确断言 provider request | 有效 | marathon 的 provider callbacks 多次断言 `request.items === transcript.collectInferenceItems()`；context-cache 精确断言 stable prefix、effective transcript、retry/resume、snapshot reconstruction；compaction 精确断言 summary request、post-compact request、queued action request。验证同上。 | `session-marathon.test.ts`、`context-cache.test.ts`、`compaction.test.ts` | 防止模型实际看到的上下文与 transcript 看起来正确但不同，尤其是 compact、retry、resume 后的重放内容。 |
| transcript 结构不变量集中校验 | 有效 | `assertTranscriptInvariants` 检查 block id、createdAt、重复 block id、重复 toolUseId、completed tool_call output、compaction marker boundary 引用和 compactedTokens，并被 marathon/preflight compaction 等路径调用；能发现结构性破坏。验证同上。 | `helpers.ts` 的 invariant helper 被 `session-marathon.test.ts`、`compaction.test.ts` 使用，覆盖 block id、createdAt、toolUseId、completed tool output、compaction marker 引用 | 防止出现重复 block id、缺 createdAt、重复 toolUseId、completed tool_call 缺 output，或 compaction marker 指向不存在的 boundary。 |
| provider mock 行为贴近真实 provider 事件顺序 | 有效 / Gated | `RecordingProvider`、`GateProvider`、`HangingProvider`、`ThrowingProvider`、`CompactGateProvider` 覆盖 async stall、provider iterator throw、abort event、summary gate、auto compact after tool_result 等非单步路径；`session.test.ts` 新增 tool_call 后无同轮 response 的 roundtrip，贴近真实 Claude Code control_request 暂停时序；真实 Claude stream/control_request 的随机路径仍由 gated smoke 补充。验证同上。 | `session.test.ts`、`session-marathon.test.ts`、`compaction.test.ts` 覆盖 pending tool、response、iterator exception、auto compact、abort event；真实 Claude 随机路径仍靠 gated smoke | 防止 deterministic 测试用一个过于理想的 provider，真实 Claude Code 路径才暴露 tool/response 时序问题。 |

### 5.5 Transcript 与 Replay

Owner：`packages/agent`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| user/text/response blocks 转 inference items | 有效 | `transcript.test.ts` 断言 text deltas 合并成 text block，并精确断言 `collectInferenceItems()` 输出 preamble+user content 和 assistant text payload；response usage block 不进入 inference items。验证：5.5 targeted command，49 pass。 | `packages/agent/src/__tests__/transcript.test.ts` | 防止 transcript 能显示但不能正确喂回 provider。 |
| pending tool call 发现与完成 | 有效 | 测试 tool_call_requested 后断言 `findPendingToolUseId()` 和 `pendingToolCalls()`，再 `completeToolCall()` 后 pending 清零；能发现 executing tool_call 无法被发现或完成。验证同上。 | `transcript.test.ts` | 防止 tool_call 状态卡在 executing，后续 run 误判还有未完成工具。 |
| completed tool call 转 `tool_use` + `tool_result` | 有效 | `transcript.test.ts` 精确断言 completed tool_call replay 为 `tool_use` + `tool_result`，包含 toolUseId、toolName、nested input、text/image output 和 isError；tool metadata 不进入模型可见 payload。验证同上。 | `transcript.test.ts` | 防止模型下一轮看不到工具输入或工具输出，导致重复调用工具。 |
| non-JSON tool input 安全存储和 replay | 有效 | 测试 undefined tool input 存为 `null`，BigInt+circular input 经 safe stringify 后 replay 为 `{ id: '1', self: '[Circular]' }`；能发现异常输入对象导致序列化或 replay 崩溃。验证同上。 | `transcript.test.ts` | 防止异常输入对象让 transcript 序列化崩溃或 replay 失败。 |
| dangling executing tool call 清理 | 有效 | 测试 completed tool_call 和 dangling executing tool_call 共存时，`removeDanglingToolCalls()` 只移除 executing 项并保留 completed 项；能发现 abort/restart 后 pending tool 残留。验证同上。 | `transcript.test.ts` | 防止 abort/restart 后遗留 executing tool 阻塞 resume。 |
| extension state snapshot latest 查询 | 有效 | 测试同名 extension 写入两次、中间夹杂其他 extension，`latestExtensionStateSnapshot('todo')` 返回 count=2，未传名称时返回最新 snapshot；能发现恢复扩展状态取到旧值。验证同上。 | `transcript.test.ts` | 防止恢复 agent 扩展状态时拿到过期 snapshot。 |
| non-JSON extension state token estimate | 有效 | 测试 extension state 含 BigInt 和 circular reference 时 `estimateContextTokens()` 仍返回正数；能发现 compact token estimate 被非 JSON 状态打断。验证同上。 | `transcript.test.ts` | 防止 token estimate 被 BigInt/cycle 等扩展状态打断，影响 compact 判断。 |
| transcript snapshot 序列化/反序列化等价 | 有效 | `transcript.test.ts` 构造 user/thinking/redacted/tool_result metadata/response/abort/boundary/marker/extension/resume 混合 transcript，经过 JSON stringify/parse 后断言完整 snapshot、block 类型序列、`collectInferenceItems()` 和 latest extension snapshot 都等价；另测 snapshot 是深拷贝，live tool_call 后续完成不会污染旧 snapshot，修改 snapshot 也不会反向污染 live transcript；context-cache 与 marathon 继续覆盖恢复后运行。验证同上。 | `transcript.test.ts`、`context-cache.test.ts`、`session-marathon.test.ts` | 需要发现保存后再加载丢 block、丢 metadata、改变 replay 内容，或旧持久化 snapshot 被 live block mutation 污染的问题。 |
| provider request exact replay 内容 | 有效 | context-cache、marathon、compaction 多处在 provider callback 内精确断言 `request.items` 或断言等于 `Transcript.collectInferenceItems()`，覆盖普通 turns、retry/resume、snapshot restore、compaction summary/recent context、tool history。验证同上。 | `context-cache.test.ts`、`session-marathon.test.ts`、`compaction.test.ts` | 需要发现复杂历史、tool、extension、compact 混合时喂给模型的上下文漂移。 |
| effective replay 只包含模型应看到的 block | 有效 | `context-cache.test.ts` 精确断言 provider request 只包含 latest summary、recent user 和当前 preamble+user，并负向断言旧 compacted history、response usage、error code/message、abort state、compaction marker、extension snapshot 和 compactedTokens 都不泄漏。验证同上。 | `context-cache.test.ts` | 防止 compaction marker、extension snapshot、internal error 状态等内部块进入 provider request。 |
| replay 保持 tool_use/tool_result 成对且顺序正确 | 有效 | `compaction.test.ts`、marathon 和 helper `assertNoOrphanToolItems()` 断言 tool_use 必须先于 matching tool_result 且不能悬空，覆盖 summary input、restore、provider error 后恢复和单 turn compact。验证同上。 | `compaction.test.ts`、`context-cache.test.ts`、`session-marathon.test.ts` | 防止 provider 收到孤立 tool_result、孤立 tool_use 或乱序工具历史。 |
| replay 中 thinking/redacted thinking 能跨 provider 边界保留 | 有效 | `transcript.test.ts` 精确断言 thinking text、signature、redacted data 按 provider 顺序转 inference items；`compaction.test.ts` 断言 summary input 保留 thinking/redacted/tool metadata 边界；`jsonl-output.test.ts` 覆盖 Claude JSONL thinking 输出，并单独断言 Claude replay 跳过 unsigned thinking。验证同上。 | `transcript.test.ts`、`jsonl-output.test.ts`、`compaction.test.ts` | 防止开启 thinking 的真实模型路径在重放或 compact 后丢失签名、redacted thinking 或顺序；同时防止本地可保存但 Claude 不接受的 unsigned thinking 污染真实 provider request。 |

### 5.6 Compaction

Owner：`packages/agent`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| 手动 compact 插入 boundary 和 marker | 有效 | `session.test.ts` 手动 compact 后精确断言 block 序列包含 `compaction_boundary` 和 `compaction_marker`，并检查 collectInferenceItems 的 summary user message；能发现 compact 成功但没有边界标记。验证：5.6 targeted command，66 pass。 | `session.test.ts` | 防止 compact 完成后 transcript 无法标识历史摘要边界。 |
| compact 不删除旧 blocks | 有效 | 同一测试断言 compact 后 transcript 仍保留旧 user/text/response，再追加 boundary/recent user/marker；能发现 compact 破坏审计历史。验证同上。 | `session.test.ts` | 防止 compact 破坏审计历史，影响 UI 查看完整 transcript。 |
| replay 从 latest compaction boundary 开始 | 有效 | `transcript.test.ts` 断言 `replayableBlocks()` 从 boundary 开始，`compaction.test.ts` 的多次 compact 测试断言只 replay 第二个 boundary summary 且不包含 first summary；能发现旧历史继续进上下文。验证同上。 | `transcript.test.ts`、`compaction.test.ts` | 防止 compact 后仍把旧长历史发给 provider，失去压缩意义。 |
| boundary summary 转成下一次 inference user message | 有效 | `transcript.test.ts` 断言 boundary summary 转成 `Previous conversation summary:\n...` 的 user_message；session/compaction 的 post-compact provider request 也精确断言同一结构。验证同上。 | `transcript.test.ts`、`session.test.ts`、`compaction.test.ts` | 防止模型看不到摘要，只收到被截断的 recent context。 |
| usage 接近 context limit 时自动 compact + resume | 有效 | `session.test.ts` 用小 contextWindow 和高 inputTokens 触发 auto compact，断言 transcript 为 user/text/response/boundary/marker/resume/text/response，且 continuation request 为 summary + resume；能发现到上限后未自动恢复。验证同上。 | `session.test.ts` | 防止长会话到上下文边界后直接失败，而不是自动恢复。 |
| compaction summary provider 卡住时 abort 可收敛 | 有效 | `session.test.ts` 让 summary provider 永不 yield，compact 期间 abort 后断言 provider cancel signal 收到、phase idle、transcript 追加 abort，compact Promise 在 timeout 内收敛。验证同上。 | `session.test.ts` | 防止 compact 阶段卡死导致用户无法停止 session。 |
| preflight compact 在 provider request 前发生 | 有效 | `compaction.test.ts` 设置 preflight threshold，send/retry/resume 的第一轮必须是 summary request，第二轮才是普通 model request；send 路径还断言 incoming user 只在普通 request 出现一次。验证同上。 | `compaction.test.ts` | 防止新用户消息、retry 或 resume 已经把上下文推过上限后才请求模型，导致真实 provider 直接 context overflow。 |
| compaction summary request 的模型、thinking、cwd、tools 与契约一致 | 有效 | preflight 测试在 summary provider request 内断言 modelId、cwd、thinking、tools=[]、summary system prompt；能发现 summary 请求使用普通工具或丢 thinking 配置。验证同上。 | `compaction.test.ts` | 防止 summary 用错模型/思考等级、错误 cwd，或把普通工具暴露给总结请求。 |
| cut point 不能切断 `tool_use -> tool_result` | 有效 | `compaction.test.ts` 在 completed tool history 和 single oversized turn 中断言 summary request items 为 user/tool_use/tool_result，并调用 `assertNoOrphanToolItems()`；pending tool 时 compact no-op。验证同上。 | `compaction.test.ts` | 需要防止模型看到孤立 tool_use 或孤立 tool_result，引发 provider 协议错误。 |
| 单个超长 turn 的 split-turn cut point | 有效 | `compaction.test.ts` 构造单 turn 内 user/tool_use/tool_result/recent assistant text，断言 compact 摘要早期 tool history 且 recent assistant text 留在 replay；`transcript.test.ts` 覆盖 cut point 计算。验证同上。 | `compaction.test.ts`、`transcript.test.ts` | 防止整轮超过 recent budget 时无法 compact，或把同一 turn 的早期上下文直接丢掉。 |
| compact 后下一次 provider request 精确等于 summary + recent context | 有效 | preflight send/retry/resume、preflight queued send、manual queued send、auto compact、snapshot restore 等测试都精确断言 post-compact `request.items` 为 summary user message 加 recent/resume/current user，含 preamble 时也断言顺序。验证同上。 | `compaction.test.ts` | 需要发现 summary、recent blocks、preamble 或 tool history 被漏放、重复放、乱序放。 |
| pre-turn compact request shape：incoming user 恰好出现一次，control-only item 不进 summary | 有效 | preflight 测试统计 incoming user 只在普通 request 出现一次，summary request 只包含旧 user/assistant；aborted text summary 测试证明 abort control block 不进入 summary items。验证同上。 | `compaction.test.ts` | 防止当前用户消息被重复注入，或模型切换、cwd/config diff、resume control block 等运行时控制信息污染 summary。 |
| mid-turn compact / auto-recover continuation request shape | 有效 | auto compact after tool result 测试断言 summary request 包含 user/tool_use/tool_result，continuation request 精确等于 summary + resume，并断言 toolCalls 仍为 1、pending tool 清零。验证同上。 | `compaction.test.ts` | 防止工具执行后自动 compact 时丢失 continuation 上下文，或把已完成工具重新放成 pending。 |
| 多次 compact 只 replay latest boundary，不重复旧 summary | 有效 | `multiple compactions` 直接构造两次 boundary/marker 后断言 collectInferenceItems 只包含 second summary 和之后上下文，且 JSON 不含 first summary；manual compact after boundary 也断言旧 question 不再出现。验证同上。 | `compaction.test.ts` | 需要规避摘要套摘要无限膨胀，或旧历史重新进入上下文。 |
| summary provider error/abort 不留下半截 boundary/marker | 有效 | `compaction.test.ts` 对 summary provider error、context overflow error、manual compact abort 都断言不留下 boundary/marker；abort 场景还断言原 blocks 保持为前缀、只追加 abort，并校验 transcript invariant。验证同上。 | `compaction.test.ts`、`session.test.ts` | 需要防止 compact 失败后 transcript 处于既不像旧状态也不像新状态的中间态。 |
| empty summary 不插入 boundary/marker，且 session 可继续 | 有效 | 测试 summary provider 只返回 response 无 text，compact 后断言无 boundary/marker，再 send follow-up 并看到正常回答；能发现空摘要破坏上下文。验证同上。 | `compaction.test.ts` | 防止空摘要把旧历史替换成无信息 boundary，后续模型失去任务上下文。 |
| 没有可压缩历史时 manual compact 明确 no-op 或可解释失败 | 有效 | 测试空 transcript manual compact 不调用 provider、phase idle、blocks 仍为空；pending tool 时 compact 也 no-op 并保留 pending tool。验证同上。 | `compaction.test.ts` | 防止用户触发 compact 后 session 状态变化但没有任何有效 summary，或 UI/AgentClient 收不到可解释结果。 |
| auto compact + resume 不重复执行已完成 tool call | 有效 | auto compact after tool result 测试用计数工具断言 toolCalls 只增加一次，provider requests 为 tool call、summary、continuation 三轮，pending tools 清空；能发现 compact 后重复执行工具。验证同上。 | `compaction.test.ts` | 需要防止上下文压缩后把已完成工具当成待执行工具再次运行。 |
| aborted text、completed tool result 在 compact summary 输入中保留 | 有效 | `compaction summary input keeps aborted text progress` 断言 abort 前 assistant text 进入 summary request；completed tool 测试断言 user/tool_use/tool_result 成对进入 summary request。验证同上。 | `compaction.test.ts` | 防止长任务中被用户停止过的有用进展或已完成工具证据在压缩后消失。 |
| compact/preflight 期间 queued send 能按序 drain | 有效 | `CompactGateProvider` 阻塞 summary 时发送 queued question，分别覆盖 manual compact 和 preflight compact；断言 phase compacting、queue 出现，summary release 后先完成原 action 再 drain queued send，provider request 精确包含 summary/current user/queued user，queue 清空。验证同上。 | `compaction.test.ts` | 防止长 summary 期间用户继续输入后消息丢失、乱序，或 compact 完成后没有继续处理。 |
| compact/preflight 期间 abort、retry、resume 的 action 收敛 | 有效 | 测试覆盖 manual compact summary 卡住时 abort 收敛，send/retry/resume 的 preflight summary 卡住时 abort 在普通 model request 前停止且无 boundary/marker，preflight queued send/retry/resume 按序 drain，retry/resume 自身触发 preflight compact，以及 compact 期间 queued retry/resume 在 summary commit 后运行。验证同上。 | `session.test.ts`、`compaction.test.ts` | 防止停止、重试或继续操作与 summary 写入交错，留下 busy phase、重复 action 或半截 transcript。 |
| compact 过程遇到 context-window 错误与普通 provider 错误分流 | 有效 | `compaction.test.ts` 覆盖 summary context overflow code 保留、不可裁剪时 transcript atomic、可裁剪时重试更小 summary slice 并写入正确 boundary；新增 iterator rejection 形态的 `context_length_exceeded` 覆盖，证明 provider 不以 error event 返回时也会裁剪重试；普通 summary error 只请求一次且 atomic。`context-cache.test.ts` 覆盖 provider context overflow 明确写 error 并可继续发送。验证：`bun test packages/agent/src/__tests__/compaction.test.ts`，28 pass。 | `compaction.test.ts`、`context-cache.test.ts` | 防止可通过裁剪/重试恢复的超限错误被当成普通失败，或普通 provider 错误无限重试。 |
| compact 后持久化再恢复，replayable blocks 保持一致 | 有效 | `compaction boundary and marker survive snapshot reconstruction` 断言 store snapshot 中有 boundary/marker，恢复 Transcript 后下一次 request 精确等于 summary + recent + new user；context-cache 也断言 snapshot reconstruction 后 collectInferenceItems 等价。验证同上。 | `compaction.test.ts`、`context-cache.test.ts` | 需要发现 boundary/marker 在 snapshot 中丢失或恢复后 replay 起点错误。 |
| thinking/redacted thinking/extension state/tool metadata 混合 transcript 下 cut point 正确 | 有效 | mixed transcript 测试把 thinking、signature、redacted thinking、tool metadata、extension state、recent text 放在一起，断言 summary request 保留 thinking/tool pair、排除 extension state，compact 后 recent text 留在 replay。验证同上。 | `compaction.test.ts`、`context-cache.test.ts` | 需要防止非文本 block 或扩展状态让 compact 切点算法误判。 |
| thinking 模型下 summary request 的 thinking 设置有效 | 有效 / Gated | preflight 测试断言 thinking 模型的 summary request 携带 `{ type: 'effort', effort: 'medium' }`；Claude Code CLI contract 测试断言该 request thinking effort 会映射到 `--effort medium`；真实 provider gated smoke 已验证 `claude-opus-4-8` medium thinking summary-shaped request 能返回 thinking 信号、text 和 usage。本机 `claude --help` 显示当前 CLI 没有 per-request max output token flag，因此输出 token hard cap 不作为当前 Claude Code provider 测试缺口。 | `compaction.test.ts`、`cli.test.ts`、`real-cli.e2e.test.ts` gated smoke | 防止 summary 请求因为 thinking effort 丢失而使用错误模型配置；真实 smoke 能发现 CLI 参数组合或 thinking event 映射在本机真实路径退化。 |

### 5.7 模型可见上下文与 Context Cache

Owner：`packages/provider-claude-code`、`packages/agent`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| provider request prefix 在普通多轮对话中稳定 | 有效 | `context-cache.test.ts` 连续三轮 send 后断言后一轮 `items` prefix 等于前一轮完整 items，systemPrompt 和 tools 也保持同一结构；能发现无意义重排破坏 prefix。验证：5.7 targeted command，34 pass / 1 skip。 | `context-cache.test.ts` | 防止 system prompt、tools schema、preamble 或历史 items 无意义重排，破坏 context cache 基线。 |
| provider request 只由 effective transcript 和当前 prompt context 构成 | 有效 | 测试精确断言 request 由 latest summary、recent user、current preamble+user 构成，并排除 old history、response usage、error code/message、abort state、extension state、compaction marker 和 compactedTokens。验证同上。 | `context-cache.test.ts` | 防止 store snapshot、extension state、compaction marker、UI-only 状态进入模型可见上下文。 |
| provider、retry、compact、resume 共用同一个 effective transcript 入口 | 有效 | `context-cache.test.ts` 断言 retry/resume request 等于 `Transcript.collectInferenceItems()`；marathon 多轮断言 request 与 transcript 一致；compaction 断言 summary/recent/queued request 的 exact items。验证同上。 | `context-cache.test.ts`、`session-marathon.test.ts`、`compaction.test.ts` | 防止不同路径各自拼 request，导致正常 send 通过但 compact 或 retry 后真实模型上下文漂移。 |
| 注入内容有明确上限和截断策略 | 有效 | 测试构造超长 preamble、reference 文本、assistant text、tool output，断言 provider-visible JSON 含 truncation marker 和 head/tail，且不含大段原文；随后断言 transcript audit log 仍保留原文。验证同上。 | `context-cache.test.ts` 覆盖 preamble、reference 展开文本、assistant text、tool result 的 provider-visible 截断，且 transcript audit log 保留原文 | 防止大文件引用、大工具输出或过长 preamble 直接撑爆上下文，compact 也来不及恢复。 |
| 记录给模型的 tool output 使用 head/tail + truncation marker | 有效 | 同一截断测试断言 tool output 的 head `tool-head-`、tail `-tool-tail` 和 `[...] truncated` 标记都在 request 中，同时大段重复内容被移除；能发现只保留头部或无标记截断。验证同上。 | `context-cache.test.ts` 覆盖 head/tail 保留和 truncation marker | 防止截断后模型不知道内容被省略，或只保留头部导致错误诊断。 |
| provider context-overflow 错误触发可恢复路径或明确失败 | 有效 | 测试 provider 返回 `context_length_exceeded` 后 `send` reject，transcript 写入带 code 的 error，phase 回 idle，随后再次 send 可成功；能发现 generic error 或不可恢复 busy 状态。验证同上。 | `context-cache.test.ts` | 防止模型上下文超限后 session 只记录 generic error，无法 compact/retry 或向壳子给出明确状态。 |
| provider usage 中 cache read/write token 字段被解析 | 有效 | `jsonl-output.test.ts` 断言 Claude result usage 的 `cache_read_input_tokens`、`cache_creation_input_tokens` 映射成 `cacheReadTokens`、`cacheWriteTokens`；能发现 provider 字段被丢弃。验证同上。 | `jsonl-output.test.ts` | 防止 provider cache 指标被丢弃，后续无法判断真实 cache 行为。 |
| cache usage 被 AgentSession 记录并对外暴露 | 有效 | `context-cache.test.ts` 断言 response blocks 记录 cacheRead/cacheWrite tokens，`server.test.ts` 断言同一 usage 经 transcript patch frame 到达 `AgentClient.transcript()`；`packages/repl/src/__tests__/renderer.test.ts` 断言 REPL stdout 显示 `cache_read` 和 `cache_write` 字段，`process.test.ts` 断言 REPL 会话进程路径显示同一 usage 字段。验证：5.7 targeted command 和 `bun test packages/repl/src`，REPL 8 pass。 | `context-cache.test.ts`、`server.test.ts`、`renderer.test.ts`、`process.test.ts` | 需要发现 usage 在 provider 到 session 到 UI/AgentClient/REPL 呈现链路中丢字段。 |
| cache usage 作为 context pressure 参与 auto compact，但不进入模型上下文 | 有效 / Gated | `auto compaction counts cache usage as context pressure` 证明 `cacheWriteTokens` 足以触发 usage-based auto compact，并断言 summary request 与 post-compact resume request 形状；`cache usage is recorded without leaking into model context or breaking tool loop` 证明 cache usage 被记录、tool loop 可继续、没有可压缩窗口时不会凭空写 boundary。真实 REPL Haiku 4.5 验收在同一会话中 3 次观察到 cache-backed usage 超过 160k 后进入 `state> compacting`，resume 后 usage 回到约 16k。验证同上，真实记录见 `docs/repl-compact-haiku-acceptance.md`。 | `compaction.test.ts`、`context-cache.test.ts`、真实 REPL gated smoke | 防止真实 provider 把大上下文记到 cache read/write 后 agent 不 compact；同时防止 cache 字段作为内部 usage 元数据泄漏进模型可见上下文或破坏 tool loop。 |
| demi 主动保障 stable prompt prefix 时，跨 turn prefix 字节级稳定 | 有效 | `stable prompt prefix` 测试把第二轮 request 的 cache 输入 prefix JSON.stringify 后比较，等价历史下 byte-identical；能发现工具/system/preamble/history 无意义抖动。验证同上。 | `context-cache.test.ts` 断言等价历史下 stable prefix 字节级一致 | 如果要主动利用 cache，该测试能发现无意义重排 tools/system prompt 破坏命中率。 |
| tools/schema/system prompt/model/preamble 改变时 cache 失效规则 | 有效 | 同一测试分别改变 system prompt、首轮 preamble、tool description、model id，并断言 prefix 与 baseline 不同；能发现该失效时没有失效或错误复用 prefix。验证同上。 | `context-cache.test.ts` 断言 tools、system prompt、model、preamble 变化会改变 cache 输入前缀 | 需要防止 cache 命中建立在错误前缀上，或该失效时没有失效。 |
| compact 后 cache prefix 变化与重新稳定 | 有效 | `provider request prefix restabilizes after compaction` 断言 compact 后 first request 以 summary 替换旧历史且不含 old question，第二个 post-compact request 的 prefix 与 first post-compact request byte-identical。验证同上。 | `context-cache.test.ts`、`compaction.test.ts` 覆盖 compact 后 summary prefix 替换旧历史，后续 request 重新形成稳定前缀 | 需要发现 compact 后上下文前缀持续抖动，导致 cache 永远无法稳定命中。 |
| 真实 provider cache 命中 | Gated | `real-cli.e2e.test.ts` 新增 `DEMI_CLAUDE_CODE_CACHE_E2E=1` gated smoke：构造带 tools 的稳定大 system prompt，连续两次真实 `ClaudeCodeProvider` 请求，第一轮必须出现 cache read/write usage，第二轮必须 `cacheReadTokens > 0`，并断言真实 text delta 拼接后包含 `DEMI_CACHE_TOOL_OK` 且没有 tool_call。验证通过：`DEMI_CLAUDE_CODE_CACHE_E2E=1 bun test packages/provider-claude-code/src/__tests__/real-cli.e2e.test.ts --timeout 180000`，cache 用例 pass，同文件其它 gated 用例按 env skip。默认 deterministic 测试仍只证明 prefix 与 usage contract。 | `real-cli.e2e.test.ts` gated smoke，不进默认测试 | 用来确认 deterministic contract 之外的真实 CLI/服务端 cache 行为没有退化，并发现 cache usage 字段虽然有 contract 但真实 provider 不再产生 hit 的问题。 |

### 5.8 Command Registry

Owner：`packages/shell`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| positionals、flags、stdin fields 解析 | 有效 | 测试 `editor create src/foo.ts` 加 stdin，断言 positional `path` 和 stdinField `content` 都进入 parsed values；能发现字段错配或 stdin 丢失。验证：5.8 targeted command，11 pass。 | `packages/shell/src/__tests__/command.test.ts` | 防止模型调用注册命令时参数被错配到错误字段。 |
| long options 校验与数字 coercion | 有效 | 测试 `--old`、`--new`、`--occurrence 2` 解析后 occurrence 为数字 2，并在 invalid values 测试中断言 NaN 被拒绝；能发现数字仍以字符串进入命令。验证同上。 | `command.test.ts` | 防止字符串数字、非法 option 进入命令实现造成隐式行为。 |
| `--json`、boolean、repeated array options | 有效 | 测试 `editor list --json --verbose --tag changed --tag staged`，断言 json=true、boolean 为 true、repeated tag 为数组；能发现结构化模式或 array/boolean 解析错误。验证同上。 | `command.test.ts` | 防止 agent 依赖结构化输出时拿到 raw text 或数组/布尔解析错。 |
| unknown options / invalid values 拒绝 | 有效 | 测试 unknown `--missing` 抛 `Unknown option`，`--occurrence NaN` 抛 invalid value；能发现模型拼错参数被静默忽略或非法值继续执行。验证同上。 | `command.test.ts` | 防止模型拼错参数却被静默忽略，产生看似成功的错误操作。 |
| `CommandSpec` 作为 prompt/help 单一来源 | 有效 | `renderCommandPrompt` 测试从同一个 spec 中断言 summary、effects、success/failure output、positionals、options、stdin/heredoc、examples、JSON 说明全部进入 prompt；能发现说明与 spec 漂移。验证同上。 | `command.test.ts` | 防止 system prompt、`prompt` 子命令和实际 parser 三套说明漂移。 |
| registry 注册命令并渲染 prompt | 有效 | 测试 registry register 后 `get/list/renderPrompt` 返回同一 spec/prompt，并断言重复注册报错；能发现新增命令未进入 prompt 或 registry 覆盖旧命令。验证同上。 | `command.test.ts` | 防止新增命令未进入 agent 可见能力列表。 |
| 注册命令名不能复用 shell/system reserved names | 有效 | 测试枚举当前完整 reserved set，断言每个名字注册时都抛出 `reserved for shell/system commands`；能发现新增注册命令遮蔽系统命令、shell builtin 或常用工具名。验证同上。 | `command.test.ts` | 防止 agent 命令遮蔽常见系统命令或 shell builtin，破坏用户预期。 |
| `<command> prompt` 使用同一个 renderer | 有效 | `runRegisteredCommand(editor prompt)` 断言 stdout 精确等于 `renderCommandPrompt(editorSpec) + '\n'`；能发现 prompt subcommand 与 system prompt renderer 分叉。验证同上。 | `command.test.ts` | 防止模型通过 help 学到的调用方式与 system prompt 不一致。 |
| JSON mode output schema 校验 | 有效 | 测试 `editor list --json` 捕获 command stdout、parse JSON 并输出通过 schema 的 `{ files: [...] }`；另构造非法 JSON 和 schema mismatch 输出，断言 `runRegisteredCommand` 拒绝并且不向外层 stdout 转发坏内容。验证同上。 | `command.test.ts` | 防止注册命令声称 JSON 输出但返回不可解析或结构错误的数据。 |
| 无 JSON schema 的 subcommand 拒绝 JSON mode | 有效 | 测试对没有 JSON schema 的 `editor create --json` 直接 reject `does not define JSON output`；能发现调用方误以为所有子命令都支持结构化输出。验证同上。 | `command.test.ts` | 防止调用方误以为某命令有结构化输出而继续自动处理。 |

### 5.9 Bash Environment 语义

Owner：`packages/shell`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| cwd/env/last status 跨 `shell_exec` 保持 | 有效 | `environment.test.ts` 跨同一 shellId 断言 `cd` 后 pwd 改变、export 后 env 保持，并单独断言 `$?` 跨 exec、list、pipeline 更新；能发现 shell session 退化成一次性执行。验证：5.9 targeted command，73 pass。 | `packages/shell/src/__tests__/environment.test.ts` | 防止 shell session 退化成一次性命令执行，模型后续命令跑在错误目录或环境。 |
| stateful builtins 在当前 session 生效且非法参数不污染状态 | 有效 | 测试覆盖 inline export/cd 对后续命令生效，非法 `cd one two` 不改 cwd，非法 export 只保留合法变量，unset 能删除变量/函数且非法 unset 不误删；能发现半修改状态。验证同上。 | `environment.test.ts` | 防止 `cd/export/unset` 等失败后留下半修改状态。 |
| `read`、stdin、heredoc、here-string 进入当前 shell 语义 | 有效 | read 测试覆盖 here-string、IFS、`-r/-n/-N/-d`、EOF status；heredoc 测试覆盖 registered/system commands，compound input redirection 覆盖 here-string；能发现输入被丢到错误命令。验证同上。 | `environment.test.ts` | 防止交互式或 heredoc 命令的输入被错误丢弃或写到错误命令。 |
| list operators、prefix assignments、assignment-only commands | 有效 | 测试断言 `&&/||/;` 执行顺序和 exit status，prefix assignment 只对单命令生效，assignment-only 持久化并支持 `+=`，command substitution status 用于 assignment-only。验证同上。 | `environment.test.ts` | 防止常见 shell 语法被错误 fallback 到系统 shell 或执行顺序错误。 |
| parameter/arithmetic/command substitution expansion | 有效 | 测试覆盖默认值/赋值/替代/长度/pattern/substring/case/indirect parameter expansion，算术 expansion/commands，以及 command substitution 的 cwd/env 隔离、换行处理、legacy backtick/status。验证同上。 | `environment.test.ts` | 防止变量展开和命令替换与 bash 语义偏离，导致 agent 命令读写错误路径或值。 |
| functions、local、return、function redirection | 有效 | 测试函数跨 exec 保持、函数参数不污染外层 positionals、函数内 cd/export 改 session，local 作用域、return 中断函数/source，function definition redirection 在 call time 生效。验证同上。 | `environment.test.ts` | 防止 sourced script 或函数调用污染外层 scope，或 redirection 应用时机错误。 |
| background jobs、jobs wait、spawn failure | 有效 | 测试后台 job 跨 exec 可由 `jobs` 看到、`wait %1` 收集输出并清空 jobs；不存在命令作为后台 job 时 wait 返回 127 且 stderr 有命令名。验证同上。 | `environment.test.ts` | 防止后台任务丢失、无法等待，或 spawn 失败导致 shell 挂死。 |
| `source`、`set`、`shift`、`eval`、`type`、`command` builtin | 有效 | 测试 source 改 env/cwd、PATH 查找、继承输入重定向和临时 positionals；set 管理 errexit/noglob 与 `set --`；shift 错误不污染；eval 当前 session 执行；type/command 做 lookup 和系统执行。验证同上。 | `environment.test.ts` | 防止这些会改变 session 或 introspection 的 builtin 被错误交给系统命令。 |
| registered command / builtin / function / system command 调度顺序 | 有效 | 测试 registered command 优先于同名 shell function 且记录 registered-command audit，`command echo-cmd` 仍调用 registered，`command` 跳过普通函数转系统查找，`type` 报告 builtin/registered/function/file。验证同上。 | `environment.test.ts` | 防止 agent 注册能力被 shell function 遮蔽，或 `command` 绕过状态 builtin。 |
| glob、redirection、pipeline、compound command、subshell | 有效 | 测试 glob 通过 host 展开且 quoted 不展开；文件/FD redirection、noclobber、registered command redirection、大输出重定向；pipeline 含 compound commands；group/subshell 状态与 redirection 语义。验证同上。 | `environment.test.ts` | 防止常见脚本组合在 agent shell 中与真实 shell 行为严重不一致。 |
| `if`、`[[ ]]`、`case`、for/while/until、group、break/continue | 有效 | 测试覆盖 if/elif/no-match、`[[ ]]` file/string/regex/pattern、case/fallthrough/redirection、for/C-style for/while/until、group command、break/continue 嵌套和非法用法。验证同上。 | `environment.test.ts` | 防止控制流判断错误，尤其是脚本自动化里跳错分支或循环无法退出。 |
| explicit `exit` 不被 negation/control flow 改写 | 有效 | 测试 `! if ... exit`、`! for ... exit`、`! while ... exit`、`! { exit; }`、`! exit` 都保持原 exit code 7；能发现 negation 或控制流吞掉显式 exit。验证同上。 | `environment.test.ts` | 防止用户脚本已经退出但 runner 继续执行后续命令。 |
| unsupported parser constructs 明确拒绝，不整段交给系统 shell | 有效 | 测试 `time printf SHOULD_NOT_RUN` reject `Unsupported shell syntax: timed pipelines`；若错误 fallback 到系统 shell 会产生输出或 audit，因此能直接发现绕过 parser。验证同上。 | `environment.test.ts` | 防止逃过 audit/registered command/Host 抽象，直接用系统 shell 执行不可控脚本。 |
| system command audit events 和 spawn failure | 有效 | 测试 `printf hi` 产生 system-command audit，cwd/args/exitCode 精确断言；不存在命令返回 exited 127 和 stderr，不挂住；能发现审计缺失或 spawn failure 不收敛。验证同上。 | `environment.test.ts` | 防止 UI/AgentClient 审计缺失，或命令不存在时工具调用挂死。 |
| portable file commands 先走 just-bash registry + `Host.fs` | 有效 | `environment.test.ts` 用拒绝 `Host.spawn` 的 `NoSpawnLocalHost` 执行 `cat input.txt | tee copied.txt` 后再 `cat copied.txt`，断言输出来自 workspace 文件、`spawnCalls=0`，并记录 `portable-command` audit。验证：`bun test packages/shell/src/__tests__/environment.test.ts packages/shell/src/__tests__/host-fs.test.ts packages/shell/src/__tests__/root-entry.test.ts`，86 pass。 | `environment.test.ts` | 防止 `cat`/`ls`/`grep`/`tee` 等 fork 已实现命令落回本机 coreutils，导致 virtual/remote Host 不可用或不同系统行为漂移。 |
| 更完整的 bash 兼容性 spec | Gated | 主仓库 5.9 targeted command 只覆盖关键 agent shell 语义；`packages/just-bash` 有独立 vitest/comparison 测试脚本和大量 imported specs，但未纳入本次默认 targeted command，需要在子模块单独运行。 | 主仓库覆盖关键 agent 语义；更完整 spec 在 just-bash 子模块 | 用来发现主仓库关键路径之外的 bash 兼容性回归。 |

### 5.10 Shell 控制面

Owner：`packages/shell`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| `shell_exec` running/yield 后可 `shell_wait` | 有效 | `environment.test.ts` 断言长命令先返回 running，再 `wait` 得到 exited/stdout/audit/last status；`tools.test.ts` 用 AgentSession 验证模型可从 shell_exec tool_result 读出 shellId 并调用 shell_wait。验证：5.10 targeted command，80 pass。 | `environment.test.ts`、`packages/shell/src/__tests__/tools.test.ts` | 防止长命令超出单次等待后失联，模型无法继续观察输出。 |
| 默认 shell 忙时，省略 `shellId` 的 `shell_exec` 使用辅助 shell | 有效 | 测试先在 agent 默认 shell 启动前台长进程，再调用不带 `shellId` 的 `shell_exec` 执行 probe，断言 probe 在新 shell 成功返回；随后 abort 原前台进程，再断言默认 shell 可复用。验证：`bun test packages/shell/src/__tests__/environment.test.ts packages/shell/src/__tests__/tools.test.ts packages/coding-agent/src/__tests__/coding-harness.test.ts packages/coding-agent/src/__tests__/coding-marathon.test.ts`，94 pass。 | `environment.test.ts` | 防止模型验证 dev server 时无法在同一 agent session 里发起 curl probe，只能背景化服务或误杀进程。 |
| output limit 触发 running output_limit | 有效 | 测试设置小 `outputLimitBytes`，断言首次 exec 返回 running、reason=`output_limit`、已有 stdoutDelta，随后 wait 收到剩余输出；能发现大输出撑爆单次 tool result。验证同上。 | `environment.test.ts` | 防止大输出撑爆 tool result 或 transcript。 |
| `shell_input` 可向 foreground process 写 stdin | 有效 | `environment.test.ts` 启动等待 stdin 的 foreground process，再 input `typed\n` 并断言 exited/stdout；`tools.test.ts` 还覆盖 shell_input tool 在 AgentSession abort 路径。验证同上。 | `environment.test.ts`、`tools.test.ts` | 防止交互式命令无法继续，或输入写入错误 shell。 |
| `shell_input` 精确写入 bytes，行式 reader 必须收到 newline 才继续 | 有效 | 测试启动 `read -r line`，先写入 `typed` 并断言进程仍 running 且无 stdout，再写入 `\n` 并断言输出 `typed` 后退出；tool description 和 coding prompt 都断言 line-oriented prompt 要包含 newline。验证同上。 | `environment.test.ts`、`tools.test.ts`、`coding-harness.test.ts` | 防止模型以为发送非空 stdin 就等于按下回车，导致真实 readline/read 进程卡住。 |
| idle foreground process 默认保持 running，不误报 needs_input | 有效 | 测试启动等待 stdin 的安静 foreground process，`shell_wait` 超过 yield window 后仍返回 running，随后 input 可正常完成；能发现空闲进程被误判成特殊 needs_input 状态。验证同上。 | `environment.test.ts` | 防止安静安装、构建、dev server 被误判为需要用户输入。 |
| `shell_wait` 的 `yieldAfterMs` 从每次调用重新计时 | 有效 | 测试先让进程 running，等待 20ms 后再调用 `wait(yieldAfterMs=20)`，断言该 wait 至少等待当前调用窗口而不是立刻返回；能发现累计时间错误导致空轮询。验证同上。 | `environment.test.ts` | 防止 wait 因累计时间错误立刻返回，模型反复空轮询。 |
| `shell_abort` 终止 foreground process，且是 control action | 有效 | `environment.test.ts` 断言 abort running process 返回 status aborted；`tools.test.ts` 断言 shell_abort tool result 文本含 status aborted 且 `isError=false`；能发现停止被当作失败或进程残留。验证同上。 | `environment.test.ts`、`tools.test.ts` | 防止停止 dev server 被当作任务失败，或进程继续占端口。 |
| abort/timeout flush redirected foreground output | 有效 | 测试 abort/timeout 正在写入重定向文件的 foreground process，断言 tool stdout/stderr 不泄漏内容且目标文件分别包含 aborted/timed-out 文本；能发现 flush 丢失或输出错路由。验证同上。 | `environment.test.ts` | 防止本应写入文件的输出泄漏到 tool output，或文件内容丢失。 |
| dispose shell kill foreground/background 并移除 session | 有效 | 测试 disposeShell 会 kill foreground/background 进程、移除 shell、再次 dispose 返回 false、后续 wait/exec 报 unknown shell，并延迟检查泄漏文件不存在；disposeAll 也清空多个 session。验证同上。 | `environment.test.ts` | 防止关闭 session 后仍有子进程残留。 |
| timeout kill foreground process，之后 shell 可复用 | 有效 | 测试 `timeoutMs` 返回 timeout，随后 wait 看到 exited；另一个测试在 timeout 后同 shell 能执行新命令，再 abort 后仍能执行新命令，证明 abort state 不泄漏；tool description 和 coding prompt 断言 `timeoutMs` 是 hard stop，不是 polling 参数。验证同上。 | `environment.test.ts`、`tools.test.ts`、`coding-harness.test.ts` | 防止 timeout 后 abort state 泄漏，导致下一条命令立即失败；也防止模型用 timeout 轮询时意外停止 dev server。 |
| shell tool result 格式保留 metadata 且模型可读 | 有效 | `toToolResult` 测试断言文本包含 status/shellId/exitCode 且 metadata 保留原 ShellToolResult；AgentSession integration 从文本中解析 status/shellId/stdout 再驱动 shell_wait，证明格式可被模型路径消费。验证同上。 | `tools.test.ts` | 防止模型必须解析不稳定 JSON 或看不到 shellId/status/next action。 |
| `shell_input` 拒绝空 stdin，不承担 polling | 有效 | `tools.test.ts` 直接调用 shell_input tool，断言缺 stdin 和空字符串分别 reject，空字符串错误明确提示 use shell_wait to poll；能防止空 input 重新变成 wait。验证同上。 | `tools.test.ts` | 防止空 input 被滥用成 wait，重新引入无意义控制动作。 |
| AgentSession abort signal 传播到 `shell_exec` / `shell_wait` / `shell_input` | 有效 | 三个 AgentSession 测试分别在 shell_exec、shell_wait、shell_input pending 时调用 session.abort，随后延迟检查本应写入的 leaked 文件不存在，证明实际 foreground process 被终止。验证同上。 | `tools.test.ts` | 防止 UI abort 只停止 session 状态，不停止实际前台进程。 |
| 模型在真实长进程场景中稳定选择 wait/input/abort | Gated | 真实 REPL 记录 `docs/repl-acceptance/interactive-shell-control.md`：用 Claude Code provider 跑模糊 Node CLI + foreground HTTP service 任务，要求模型自己选择 shell 控制序列。5 轮真实运行中，前 4 轮分别暴露 line-oriented stdin/newline、`shell_wait timeoutMs` 误当 poll、错误工作目录、重复背景重启服务等问题；第 5 轮通过，且 compact 后仍能在同一 workspace 复核。默认测试只证明 deterministic contract，真实记录负责发现模型策略漂移。 | `docs/repl-acceptance/interactive-shell-control.md`；`environment.test.ts`、`tools.test.ts`、`coding-harness.test.ts` 锁住本次修复契约 | 用来发现 prompt/tool result 虽然单测正确，但真实模型仍然误用控制面的问题；本次记录证明真实 REPL 能发现并回归这些 model-facing contract 缺口。 |

### 5.11 Host、FS 与 Store

Owner：`packages/shell`、`packages/host-local`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| `HostBackedFileSystem` 经 `Host.fs` 完成 read/exists/stat/write/append/readdir/mkdir/rm/realpath | 有效 | `host-fs.test.ts` 同时用真实 `LocalHost.fs` 和记录型 fake `Host.fs` 覆盖 read/readFileBuffer/exists/stat/write/append/readdir/mkdir/rm，并断言 `Host.spawn` 调用数为 0；`LocalHost.fs` 单测覆盖本地 adapter 的 mkdir/write/append/read/readdir/stat/rm/exists。验证：`bun test packages/shell/src/__tests__/environment.test.ts packages/shell/src/__tests__/host-fs.test.ts packages/shell/src/__tests__/root-entry.test.ts`，86 pass；`bun test packages/coding-agent/src/__tests__/coding-harness.test.ts packages/coding-agent/src/__tests__/editor-command.test.ts packages/host-local/src/__tests__/local-host.test.ts`，21 pass。 | `packages/shell/src/__tests__/host-fs.test.ts`、`packages/host-local/src/__tests__/local-host.test.ts` | 防止文件系统能力错误绑定到本机 coreutils，导致 remote/container/memfs/virtual Host 不可用，或 GNU/BSD 差异进入 agent 行为。 |
| shell root entry 只暴露 browser-safe Host contract / FS class | 有效 | `root-entry.test.ts` 从 root entry 导入 Host contract 和 HostBackedFileSystem 并验证可构造/resolvePath；另扫描 shell root entry 的本地静态闭包，断言没有 `node:` import/require、`Buffer` 或 `process.env/cwd`。验证同上。 | `packages/shell/src/__tests__/root-entry.test.ts` | 防止 `@demi/shell` 根入口静态带入 Node-only adapter，破坏 browser/runtime-neutral 包边界。 |
| readFileBuffer 返回 raw bytes | 有效 | `host-fs.test.ts` 写入 `[0x68,0x69,0x0a]` 二进制内容，`readFileBuffer` 断言返回同一 byte array；能发现文本解码损坏二进制。验证同上。 | `host-fs.test.ts` | 防止二进制文件被文本编码损坏。 |
| `LocalHost` spawn capture stdout 和 stdin | 有效 | `local-host.test.ts` 断言 LocalHost spawn `printf` 可收集 stdout，spawn `sh` 后 `writeStdin/closeStdin` 可被子进程读取；能发现本地 adapter I/O 管道断裂。验证同上。 | `packages/host-local/src/__tests__/local-host.test.ts` | 防止本地 adapter 不能正确连接进程输入输出。 |
| `LocalHost` terminate foreground process | 有效 | 测试 spawn `sleep 10` 后 `handle.kill()`，`wait()` 返回 `exitCode:null` 和 `signal:SIGTERM`；能发现 shell abort/timeout 在本地进程层无法终止。验证同上。 | `packages/host-local/src/__tests__/local-host.test.ts` | 防止 shell abort/timeout 在本地进程层失效。 |
| `AgentSessionCommandStorage` 按 agent session id prefix 隔离 keys | 有效 | `storage.test.ts` 用 session-a/session-b 写同名 `todos.json`，断言各自读回独立内容、session list 只见本 session key、底层 store list 带不同 prefix；能发现跨 session 污染。验证同上。 | `packages/shell/src/__tests__/storage.test.ts` | 防止多个 agent session 的 todo 或命令状态互相污染。 |
| storage 拒绝逃逸 session prefix 的 key/session id | 有效 | 测试 `../session-b`、反斜杠 traversal、absolute key 和非法 agentSessionId 都 reject，并确认 session-b 原值未被覆盖；能发现路径逃逸写其他 session。验证同上。 | `packages/shell/src/__tests__/storage.test.ts` | 防止注册命令通过恶意 key 读写其他 session 或 store 根目录。 |
| `LocalDemiStore` 拒绝非相对 store path | 有效 | 测试 LocalDemiStore 拒绝 `../outside`、`nested/../inside`、absolute path、NUL key；能发现本地 store 路径穿越或任意文件写。验证同上。 | `packages/host-local/src/__tests__/local-store.test.ts` | 防止本地 store 被路径穿越写到任意文件。 |

### 5.12 Coding Agent Harness

Owner：`packages/coding-agent`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| coding harness 暴露 Host/commands 并由组装层创建 shell tools | 有效 | 测试 `createCodingAgentHarness({ host })` 后断言默认 commands 为 `editor/todo`；测试局部按 AgentServer 的标准组装创建 `BashEnvironment + createShellSessionTools`，断言 runtime tools 为 `shell_exec/shell_wait/shell_input/shell_abort`，并通过环境执行默认 todo command；同时断言 harness 自身不暴露 `tools()`/`dispose`。验证：5.12 targeted command，6 pass。 | `packages/coding-agent/src/__tests__/coding-harness.test.ts` | 防止 coding harness 回退到 public `BashEnvironment` 注入或自定义 tool 替换，同时防止模型打开后缺少标准 shell 控制面或默认命令未注册。 |
| registered command prompt 注入 system prompt | 有效 | 测试 systemPrompt 包含 editor/todo prompt、effects/success/failure output、todo example、workspace-cwd rule、foreground long-process rules、timeout hard-stop warning、auxiliary shell guidance、line-oriented stdin newline、avoid pkill/killall 和 no redundant restart 等文本，并断言 `editor prompt` 输出同源内容；能发现专属命令或 shell-control 说明未注入。验证同上。 | `coding-harness.test.ts` | 防止模型不知道 `editor`、`todo` 等专属命令的正确调用方式，也防止真实 REPL 里反复出现错误工作目录、误用 timeout 或重复启动已验证服务。 |
| file reference 通过 workspace `Host.fs` 读取 | 有效 | `coding-harness.test.ts` 用 fake `RecordingHost.fs` 断言 reference resolution 调用 `Host.fs.readFile('note.txt', { cwd: '/workspace' })`，并断言 `spawnCalls=0`；真实 `AgentSession.send` 路径仍验证 provider request 和 transcript 都保存展开后的文件内容。验证：`bun test packages/coding-agent/src/__tests__/coding-harness.test.ts packages/coding-agent/src/__tests__/editor-command.test.ts packages/host-local/src/__tests__/local-host.test.ts`，21 pass。 | `coding-harness.test.ts` | 防止引用展开绕过 Host，也防止引用读取依赖本机 `cat`，导致 virtual fs 或远端 fs 场景不可用。 |
| file reference 拒绝 workspace root 外路径 | 有效 | 测试 workspace root 外的 absolute path reference reject `File reference escapes workspace`；能发现引用展开读取工作区外文件。验证同上。 | `coding-harness.test.ts` | 防止通过 reference 读取工作区外文件。 |
| reference resolution 与 AgentSession send 顺序集成 | 有效 | 测试通过真实 `AgentSession.send` 发送 text+reference，provider callback 精确断言 request.items 已展开为 file content 且不含 `reference`，随后 transcript user block 也保存展开内容；能发现先写未展开引用。验证同上。 | `coding-harness.test.ts` 通过 `AgentSession` 断言 provider request 和 transcript 都保存展开后的 file content | 需要发现 coding reference 在真实 session 中是否会先写入未展开内容。 |

### 5.13 Editor Command

Owner：`packages/coding-agent`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| `editor create` 用 heredoc 创建文件 | 有效 | 测试用 heredoc 创建 `src/foo.txt`，断言 stdout、diff metadata、unified diff header 和实际 `cat` 内容；能发现多行 stdin/heredoc 被参数解析破坏。验证：5.13 targeted command，12 pass。 | `packages/coding-agent/src/__tests__/editor-command.test.ts` | 防止模型写文件时多行内容、引号或换行被 shell 参数破坏。 |
| editor 拒绝 workspace root 外路径 | 有效 | 测试 absolute outside 和 `../` relative outside 都 exit 1，stderr 包含 `Path escapes workspace`，并断言外部文件不存在；能发现越权写入。验证同上。 | `editor-command.test.ts` | 防止编辑命令越权修改工作区外文件。 |
| patch escaped path 时写入前拒绝 | 有效 | 测试 patch 先声明修改 workspace 内文件，再声明创建 workspace 外文件；失败后断言内部文件仍为原内容且外部文件不存在。验证同上。 | `editor-command.test.ts` | 防止 unified diff 中一个恶意路径导致部分文件已修改后才失败。 |
| `editor edit` exact replace 和 ambiguous matches 失败 | 有效 | 测试重复文本无 occurrence 时 exit 1 并报 `Multiple matches`，随后用 `--occurrence 2` 精确替换并断言 diff metadata 和实际文件内容；能发现歧义替换误改。验证同上。 | `editor-command.test.ts` | 防止错误替换多个位置或在歧义情况下误改代码。 |
| context disambiguation 只在唯一最近匹配时生效 | 有效 | 测试 context line 2 与两个匹配等距时失败并列出 occurrence，断言文件未改；context line 3 时只替换最近唯一匹配。验证同上。 | `editor-command.test.ts` | 防止模型提供上下文后仍改到错误位置。 |
| empty old text 拒绝且不修改文件 | 有效 | 测试 `--old ""` exit 1，stderr 为 `Old text must not be empty`，并断言原文件内容未变。验证同上。 | `editor-command.test.ts` | 防止空匹配导致在文件所有位置插入内容。 |
| unified diff patch、timestamp headers、多文件创建/删除 | 有效 | 测试普通 unified diff、带 timestamp header、同时修改已有文件并创建新文件、`/dev/null` 删除文件，均断言 stdout/diff metadata/实际文件状态；能发现常见 patch 语义缺失。验证同上。 | `editor-command.test.ts` | 防止常见 patch 格式无法应用，或删除/新增文件语义错。 |
| patch 全量校验后再写入，保证跨文件事务 | 有效 | `editor-command.test.ts` 覆盖两层失败：先验证 patch 在第二个文件校验失败时第一个文件未写；再用 `FailingWriteHost.fs.writeFile` 模拟第二个文件写入失败，断言第一、第二个文件都回滚到原内容且 `Host.spawn` 不可用。验证同 5.12 targeted command，21 pass。 | `editor-command.test.ts` | 防止 patch 中后续文件失败时前面文件已经被部分修改，或失败中的文件留下半写入内容；同时防止 editor 继续依赖 shell command 做文件写入。 |

### 5.14 Todo Command

Owner：`packages/coding-agent`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| `todo add/list/update/done` raw output | 有效 | 测试精确断言 raw `todo add`、`todo list`、`todo update`、`todo done` 输出，覆盖 pending/done marker 和多 todo 顺序；能发现模型可读任务状态反馈退化。验证：5.14 targeted command，3 pass。 | `packages/coding-agent/src/__tests__/todo-command.test.ts` | 防止模型拿不到可读的任务状态反馈。 |
| `todo add/list/update/done` JSON output | 有效 | 测试解析并精确断言 `add --json`、`update --json`、`done --json`、`list --json` 的 `{ todo }` / `{ todos }` 形状、id/text/status 和顺序。验证同上。 | `todo-command.test.ts` | 防止 agent 或 UI 需要结构化 todo 状态时解析失败。 |
| todo 状态按 agent session id 隔离 | 有效 | 测试不同 agent session 各自添加首个 todo 都得到 `T1`，并在 shell 重建测试中断言 `other-agent` 的列表只含自身 todo；能发现跨 session 串线。验证同上。 | `todo-command.test.ts` | 防止不同会话共享 todo，造成用户任务串线。 |
| todo 与 shell id 不混淆 | 有效 | 测试 dispose 旧 shell 后，同一 agent session 在新 shell 中继续累积 `T1/T2`，而另一个 agent session 在第三个 shell 中独立为 `T1`；结合 `BashEnvironment` 的 storage scope 能发现把 todo 绑到 shellId 的回归。验证同上。 | `todo-command.test.ts` 覆盖 agent session 隔离和同一 agent session 下 shell 重建后的 storage 延续 | 需要发现 shellId 和 agentSessionId 再次混淆。 |

### 5.15 Coding Agent 工作流

Owner：`packages/coding-agent`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| StubProvider 通过 shell tools 驱动 editor/todo，真实写文件 | 有效 | 测试用 StubProvider 依次发起 `shell_exec`，通过 `editor create`、`todo add`、`editor edit` 驱动真实 BashEnvironment/LocalHost，并断言工具结果；能发现 AgentSession 到 registered command 的集成断裂。验证：5.15 targeted command，5 pass。 | `packages/coding-agent/src/__tests__/coding-marathon.test.ts` | 能发现 AgentSession、shell tools、registered command、Host 写文件之间的集成断裂。 |
| workflow 中复用同一个 shell session | 有效 | 测试把上一轮 tool result 的 `shellId` 传给后续命令，并断言 result.shellId；新增 workflow 先 `cd pkg`、`export WORKFLOW_TOKEN=kept`，后续同 shellId 读取 `$PWD` 和环境变量，证明 cwd/env 状态连续。真实模型是否稳定选择复用 shellId 仍由 gated smoke 覆盖。验证同上。 | `coding-marathon.test.ts` | 防止模型后续命令丢 cwd/env 或拿不到之前的 shellId。 |
| workflow 后文件内容正确 | 有效 | workflow 结束后真实执行 `cat src/app.ts`，断言内容为 `export const value = 2`；能发现工具结果成功但文件实际未写对。验证同上。 | `coding-marathon.test.ts` | 防止工具调用看似成功但真实 artifact 没写对。 |
| workflow 后 todo 状态在 agent session 下可读 | 有效 | workflow 后用 session id 读取 `todo list --json`，断言 `T1 Run tests` 存在；多轮测试也断言同一 agent session 下 todo 最终为 done。验证同上。 | `coding-marathon.test.ts` | 防止 workflow 中 todo 写到了 shell-local 或全局错误位置。 |
| 创建文件 -> 测试失败 -> 读取错误 -> 修复 -> 测试通过 | 有效 | 测试创建带缺陷的 todo 模块和 bun test，断言首次测试非零且输出包含失败预期，再读源码、用 editor 修复、重跑测试并断言 `1 pass`；能发现失败反馈链路断裂。验证同上。 | `coding-marathon.test.ts` | 用来发现 agent 是否能围绕失败反馈迭代，而不是只验证单步命令。 |
| 长命令 running/yield -> wait -> input/abort 的 agent 级流程 | 有效 | 测试启动长 foreground 命令并 yield 为 running，随后 `shell_wait` 看到 ready，`shell_input` 注入 stdin，`shell_abort` 结束并清空 pending tool calls；能发现多轮 shell 控制状态丢失。验证同上。 | `coding-marathon.test.ts` | 用来发现 shell 控制面在模型多轮工具调用中是否仍保持正确上下文。 |
| tool error 后模型恢复继续执行任务 | 有效 | coding-marathon 覆盖 shell 命令非零退出后 provider 下一轮能看到失败输出、读取源码、修复并继续；agent `session.test.ts` 覆盖 invoke throw 被记录为 error tool result 且后续继续。验证同上，agent 行为已在 5.4 审查。 | `coding-marathon.test.ts` 覆盖命令非零退出后读取、修复、继续；agent 覆盖 invoke throw | 用来发现 coding 命令失败后模型是否有足够上下文修正，而不是 session 停死。 |
| 多轮 user message 对 coding workflow 的影响 | 有效 | 测试连续两次 `session.send`，第二次 provider request 中断言包含两轮 user message、上一轮 assistant text 和 todo state，再继续完成 todo/list/cat；能发现 transcript 或 send 顺序错乱。验证同上。 | `coding-marathon.test.ts` 覆盖两次 `session.send` 后继续读取上一轮文件、tool result 和 todo 状态 | 用来发现 queued send、transcript 和 coding state 在连续用户输入下是否错序。 |
| 多 shell + 同 agent session 的 todo/storage 一致性 | 有效 | `todo-command.test.ts` 断言 dispose 旧 shell 后同一 agent session 在新 shell 延续 todo storage，另一个 agent session 独立；coding-marathon 断言 workflow 内 todo 继续可读。验证：5.14/5.15 targeted commands。 | `todo-command.test.ts` 覆盖同一 agent session 重建 shell 后仍读取同一 todo storage；`coding-marathon.test.ts` 覆盖 agent workflow 内 todo 状态延续 | 用来发现 shellId 和 agentSessionId 再次混淆。 |

### 5.16 AgentServer/transport 协议

Owner：`packages/agent`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| JSON codec 保留 BigInt metadata 和 Uint8Array | 有效 | 测试 `stringifyAgentJson`/`parseAgentJson` 往返 BigInt、空数组、1/2/3 字节 Uint8Array，并断言类型和值；能发现 metadata 和二进制 payload 序列化损坏。验证：5.16 targeted command，30 pass。 | `packages/agent/src/__tests__/json-codec.test.ts` | 防止跨进程/网络传输后 metadata 或二进制数据损坏。 |
| transcript patch 更新 in-place tool_call metadata/status | 有效 | 测试同一位置 tool_call 从 executing 变 completed 且 metadata 改变时产生 remove/add patch，并用 `applyTranscriptPatches` 还原到 next；能发现 UI patch view 不更新。验证同上。 | `packages/agent/src/__tests__/patch.test.ts` | 防止 UI 收不到 tool_call 状态变化，只能靠全量刷新。 |
| transcript diff 处理非 JSON / cyclic metadata | 有效 | 测试 BigInt metadata 和 cyclic metadata 都能 diff/apply，不抛异常；能发现不可 JSON stringify 的 metadata 打断 transcript patch。验证同上。 | `patch.test.ts` | 防止一个不可序列化 metadata 打断整个 transcript 同步。 |
| root entry 不导出 node-only stdio transports | 有效 | 测试 `@demi/agent` root entry 不含 stdio transport，并仍导出 WebSocket transport；能发现 browser bundle 入口污染。验证同上。 | `packages/agent/src/__tests__/root-entry.test.ts` | 防止 browser/client bundle 意外包含 Node-only transport。 |
| AgentClient open/send 经 InProcessTransport 发 transcript/phase | 有效 | 测试 open/send 后 client transcript 为 user/text/response，并收到 snapshot、patch、idle phase；同一测试精确断言 response usage 的 cacheRead/cacheWrite tokens 经 transcript patch 到达 client 本地 view。验证同上。 | `packages/agent/src/__tests__/server.test.ts` | 防止基本协议动作不能驱动 session、本地 view 不更新，或 cache usage 在 AgentClient transcript 同步中丢字段。 |
| client close 清空本地 transcript view | 有效 | 测试 close 前 transcript 非空，close 后收到 closed frame 且 client transcript blocks 为空；能发现 UI 显示旧会话残留。验证同上。 | `server.test.ts` | 防止关闭会话后 UI 还显示旧 transcript。 |
| provider error code 只 forward 一次，并保留 transcript error block | 有效 | 测试 provider error 让 `client.send` reject，事件流里 error frame 精确只有一次，且 transcript 保留 user/error block 与 code；能发现重复报错或 transcript 丢错。验证同上。 | `server.test.ts` | 防止 UI 重复报错，或错误只在 frame 中出现而 transcript 丢失。 |
| shell output、bash audit、generic tool progress frame 映射 | 有效 | 测试 shell tool progress 映射为 shell_output/audit，任意 progress 包括 undefined、BigInt、cyclic object 映射成 text output，并过滤非良构 audit；能发现 REPL/GUI 实时事件缺失或格式破坏。验证同上。 | `server.test.ts` | 防止 REPL/GUI 看不到实时 shell 输出、审计事件或工具进度。 |
| shell_input frames 桥接到 active shell session tool | 有效 | 测试 shell foreground 等待 stdin，`client.shellInput` 后看到 typed stdout 和 shell_input_result，且不重复发 generic tool_progress；能发现输入没有写入 active shell。验证同上。 | `server.test.ts` | 防止用户在壳子里输入内容但没有写入正在等待的进程。 |
| client `shellInput` 等待 result，未 open 时 reject | 有效 | 测试未 open 时 reject `No session is open`，并用 gate 延迟 shell_input invoke，断言 promise 在 result 前不 settle、result 后 resolve；能发现 UI 过早认为输入成功。验证同上。 | `server.test.ts` | 防止 UI 认为输入成功但实际上没有 active session。 |
| retry 产生 transcript patch removals | 有效 | 测试 retry 后 provider 重跑最新 user，client 收到 remove patch，最终 transcript 不保留旧 assistant blocks；能发现 retry 后 UI 残留旧输出。验证同上。 | `server.test.ts` | 防止 retry 后 UI 留着已经被 runtime 截断的旧 assistant blocks。 |
| AgentServer queued send while busy 并按序 drain | 有效 | 测试第一条 send running 时第二条进入 queue 而非 rejected，释放后 provider calls 为 2，transcript user 顺序为 first、second，queue 最终清空；能发现排队顺序错乱。验证同上。 | `server.test.ts` | 防止外部壳子连续发送时 action 顺序与 session 执行顺序不一致。 |
| busy 时 AgentServer 拒绝 retry/resume/compact | 有效 | 测试 running phase 下 retry/resume/compact 都 reject `Session is busy`，并收到对应 rejected frame，provider calls 仍为 1；能发现 active run 中破坏性 mutation 被放行。验证同上。 | `server.test.ts` | 防止外部壳子在 active run 中触发破坏性 mutation。 |
| client queued send promise 按各自 phase cycle resolve | 有效 | 测试三个 sequenced gates 下 first、second、third promise 只在各自 phase cycle 后依次 settle；能发现多个 queued send 被同一个 idle 一起 resolve。验证同上。 | `server.test.ts` | 防止多个 queued send 因同一个 idle 事件一起 resolve。 |
| AgentSession queue item management | 有效 | 测试 `dequeueMessage` 删除 queued send 且不执行、`sendQueuedMessage` 把第三条移到第二条前执行、`steerQueuedMessage` 把 queued send 原子转换成 active turn `user_steer` 且不生成新 user turn、`clearMessageQueue` 清空 queued sends 且不取消 active turn；能发现 queue mutation 误碰 active action 或 transcript 顺序错乱。验证同上。 | `session.test.ts` | 防止 GUI 队列按钮只是改 UI，本体 session 没有真实队列语义。 |
| AgentClient queue item management frames | 有效 | 测试 `dequeueMessage` resolve 被删除的 queued send promise 且 provider 不运行它、`sendQueuedMessage` 重排本地 pending waiter 和服务端 queue、`steerQueuedMessage` resolve 被转换的 queued send promise 并让下一次 provider request 看到 `user_steer`、`clearMessageQueue` resolve 所有 queued send 且 active send 继续；能发现 message id 关联、queue frame、phase FIFO 收敛错位。验证同上。 | `server.test.ts` | 防止删除/清空 queued input 后提交 Promise 悬挂，或 send-now 后 promise/执行顺序和 UI 顺序不一致。 |
| error 后只 reject active action，queued send 继续 | 有效 | 测试第一条 active send provider error 后仅 first reject，second queued send 继续进入第二次 provider call 并等待自身 gate 后 resolve；能发现错误误杀整条队列。验证同上。 | `server.test.ts` | 防止一次 provider error 把后续排队用户消息全部错误取消。 |
| abort idle 返回 false，active 返回 true | 有效 | 测试 idle `abort()` resolve false；active send 中 abort resolve true，provider cancel signal 被触发且 send 收敛；能发现 stop 按钮状态语义错误。验证同上。 | `server.test.ts` | 防止 UI stop 按钮在 idle/active 状态下显示错误结果。 |
| close frame abort active session 并 dispose definition resources | 有效 | 测试 close active run 时 provider cancel 被触发并收到 closed；另测 definition.dispose 清理 shell environment，延迟写文件不会落盘；能发现关闭后后台 run 或 shell 泄漏。验证同上。 | `server.test.ts` | 防止关闭壳子后后台 run 或 shell environment 泄漏。 |
| stdio/WebSocket 基础传输和 AgentClient/AgentServer 端到端 | 有效 | stdio 测试覆盖 NDJSON streams、Uint8Array、AgentClient/AgentServer end-to-end 和 child-process AgentServer；WebSocket 测试覆盖 JSON socket frame 往返、二进制字段和 end-to-end。WebSocket 用 fake socket，不是外部网络服务。验证同上。 | `stdio-transport.test.ts`、`websocket-transport.test.ts` | 防止协议只在 in-process 测试里成立，真实 transport 序列化后失败。 |
| stdio/WebSocket 下 queued send、abort、retry、resume、compact 与 in-process 一致 | 有效 | stdio 和 WebSocket scenario provider 都覆盖 queued send、provider error、retry、abort、resume、compact、after compact send，并断言 summary request 和 post-compact context；能发现 transport frame ordering 差异。验证同上。 | `stdio-transport.test.ts`、`websocket-transport.test.ts` | 需要发现真实 transport latency/frame ordering 下的 action 收敛差异。 |
| close 时 active foreground process 通过真实 transport 被终止 | 有效 | stdio transport 测试启动会延迟写文件的 foreground shell，client close 后断言 shell 已清理且延迟文件不存在；能发现远端 session 关闭后进程继续占资源。验证同上。 | `stdio-transport.test.ts` | 需要防止壳子关闭后远端 session 进程继续占资源。 |

### 5.17 REPL

Owner：`packages/repl`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| 基本渲染、输入、scroll | 有效 / Gated | `renderer.test.ts` 通过注入 output 断言 REPL renderer 会输出 phase、queue、shell stdout/stderr、audit、progress、error、rejected、closed；断言 `/help`、`/abort`、`/retry`、`/resume`、`/compact`、`/input`、unknown、`/exit` 的命令分发和 usage 输出；`runInputLoop` 测试断言普通消息异步发送时仍能继续处理 `/abort`，并断言 send 异步失败会打印 `send failed`；`process.test.ts` 真实启动 REPL `--help` 进程并断言不会打开 provider session，还通过 `--claude-path` 指向 Claude Code 协议 fixture 启动真实 REPL 会话进程，经 stdin prompt 触发 ClaudeCodeProvider、shell tool、AgentServer/transport、renderer，再断言 tool、shell stdout、thinking、assistant text、usage、`/input` 写入 running shell 后的 stdout 和 `/exit` close；high-volume shell output 进程测试断言超过常见 pipe buffer 的 stdout 首尾 sentinel、末尾行、assistant text 和 usage 都能出现；`DEMI_REPL_TTY_E2E=1` gated 用例通过 BSD `script` pseudo-terminal 跑 fixture session，断言 ANSI TTY 输出、tool、shell stdout、thinking、assistant text、usage 和 close。当前 REPL 没有独立 scroll 产品面；若未来引入独立滚屏控件，再按第 6 章补交互滚屏测试。验证：`bun test packages/repl/src`，15 pass / 3 skip；gated TTY 验证：`DEMI_REPL_TTY_E2E=1 bun test packages/repl/src/__tests__/process.test.ts --timeout 30000`，5 pass。 | `packages/repl/src/__tests__/renderer.test.ts`、`packages/repl/src/__tests__/process.test.ts` | 防止核心 AgentServer/transport 事件到了 REPL 后用户仍看不到状态/输出，或输入循环因为等待 send 结果而卡住 `/abort`、`/input` 等控制命令；同时防止真实 REPL 进程入口、readline pipe、provider/AgentClient/session 组装后才暴露链路断裂；高体量输出测试能发现 shell output 写入和 renderer 分行在较大 stdout 下漏掉尾部输出或卡住退出；TTY gated 测试能发现真实终端模式下 readline、ANSI/color 输出或 prompt 交错破坏会话。 |
| REPL 模型选择来自 provider catalog | 有效 / Gated | `renderer.test.ts` 通过 fake provider catalog 断言无 `--model` 时选择 `defaultModelId`、映射 context window / thinking efforts / warning，但 capability 的 `defaultEffort` 固定为 `null`，不会把 provider default 当运行默认；另测显式 full id 不做 catalog rewrite，`opus` 等旧 alias 直接拒绝，显式 `--thinking` 不在 catalog supported list 中时拒绝，provider 广告的 future effort id 可通过，`--service-tier priority` 只在 selected model 广告该 tier 时进入 selection，legacy `fast` 被拒绝。`process.test.ts` 断言 Claude Code REPL 启动不会打印 `claude runtime/auth` preflight 行。真实 smoke 分别以 Claude Code 和 Codex provider 启动 REPL 后 `/exit`，banner 显示 catalog 默认模型 `claude-opus-4-7` 和 `gpt-5.5`。 | `packages/repl/src/__tests__/renderer.test.ts`、`packages/repl/src/__tests__/process.test.ts` | 防止 REPL 继续持有 provider-specific 默认模型、历史 alias、手写 capability、provider default effort 或 legacy fast alias，导致 compact/context limit、thinking/service tier 控制和真实 provider 支持范围不一致；同时防止 REPL 启动阶段额外调用外部 CLI。 |
| thinking/text/tool output 显示 | 有效 | `renderer.test.ts` 构造 thinking、assistant text、tool_call、response blocks，断言 stdout 包含 `thinking>`、`assistant>`、`tool>` 和 `usage>`；随后用同 id 更新 block，断言只输出新增 delta、不重复完整 text、不重复 usage；同一测试还覆盖 error/abort block id 去重，防止 transcript patch replay 重复打印 `error> agent` 或 `state> turn aborted`。新增 errored `tool_call` 输出测试断言 repeated-shell guard 的 `tool-error>` 内容会显示且不会在 patch replay 中重复。真实模型是否产出这些 blocks 仍由真实 provider/REPL smoke 覆盖。验证同上。 | `renderer.test.ts` | 防止真实模型已经产生 thinking/text/tool/error blocks，但 REPL renderer 分流、重复或漏显示；同时防止稳定 abort/error/tool-error block 在长会话 patch 中反复刷屏。 |
| 通过 AgentClient open/send/receive phase/transcript/shell frames | 有效 | AgentServer/transport 5.16 已覆盖 open/send/phase/transcript/shell frames；REPL 层新增 `renderEvent` stdout snapshot 覆盖 phase、queue、transcript、shell_output、audit、tool_progress、error/rejected/closed 的呈现和去重；并用真实 `AgentClient`、`AgentServer`、in-process transport、StubProvider 断言 `attachRenderer(client, renderer)` 能收到 open/send/phase/transcript/closed 事件并渲染 assistant text 和 usage。验证同上。 | `server.test.ts`、`renderer.test.ts` | 需要发现 REPL 自己订阅、状态合并、刷新节奏的问题。 |
| 真实 Claude Code provider 输出真实模型回复 | Gated | Provider 层有 `DEMI_CLAUDE_CODE_E2E=1` 的 real-cli gated 测试；REPL 层新增 `real-repl.e2e.test.ts`，只有 `DEMI_REPL_REAL_E2E=1` 时运行，会以 full id `--model claude-opus-4-8`、`--thinking medium` 启动真实 REPL，要求模型触发 `shell_exec`，每次 run 断言 tool、tool output、assistant text、usage，默认两次 run 中至少一次断言 `thinking>`。默认测试中它是 1 skip，不能替代真实运行记录。本机验证命令：`DEMI_REPL_REAL_E2E=1 bun test packages/repl/src/__tests__/real-repl.e2e.test.ts --timeout 360000`，1 pass。 | `real-repl.e2e.test.ts` gated smoke | 防止验收只跑协议 fixture 或 stub provider，没有确认真实 provider/模型路径。 |
| 真实 Codex REPL steer / queue | Gated；手动验收通过 | 2026-06-23 手动 harness 使用真实 REPL 进程、Codex provider、`gpt-5.4`、medium thinking 和 SSE transport。active turn 先启动 `sleep 12; echo DEMI_REPL_STEER_GATE_RELEASED` foreground shell gate；gate running 时发送普通消息形成 `queue> 1 pending`，再发送 `/steer Same-turn guidance: include STEER_REPL_INCLUDED...`。通过条件：REPL 渲染 `steer>`，active answer 包含 `ACTIVE_DONE STEER_REPL_INCLUDED`，queued answer 随后才出现 `QUEUED_DONE`，进程 exit 0 且 stderr 为空。记录见 `docs/repl-acceptance/codex-repl-steer-queue.md`。 | 手动真实 REPL harness；`renderer.test.ts` 锁住 `/steer` 命令分发和 steer block 呈现；`server.test.ts`、`session.test.ts`、`provider.test.ts` 锁住 runtime/provider ordering | 防止只验证 `AgentSession` 或 fake transport，而最终用户 REPL 路径没有真正调用 `AgentClient.steer`，或 queued 普通输入抢在 steered active turn 前运行。 |
| 真实 REPL 长上下文 compact 连续工作 | Gated；手动验收通过 | 2026-06-19 手动 harness 使用真实 REPL、`claude-haiku-4-5`、200k context selection、`--no-thinking` 和本地大 stdout 压力测试。最初失败暴露 compact 后真实模型认为 shell tool 不可用；修复 provider replay 后又暴露重复 tool loop 和 MCP request id 复用导致历史 tool block 被改写。当前修复后复测通过：同一会话观察到 `state> compacting` 4 次，前三次 compact 后都继续执行真实 `shell_exec` 并完成压力回合，第 4 次 compact 后完成 no-tool continuation；没有 tool unavailable 或重复调用抑制。详细记录见 `docs/repl-compact-haiku-acceptance.md`。 | 手动真实 REPL harness，尚未落成默认或 gated 自动测试；`provider.test.ts` / `jsonl-output.test.ts` 锁住 provider replay 修复；`provider.test.ts` 覆盖重复 MCP request id；`transcript.test.ts` 覆盖重复 tool id 只完成 pending block；`tools.test.ts` 覆盖 repeated-shell guard 返回非 terminal tool result 和 suppression 后 AgentSession 可继续 | 防止只凭 deterministic compact 测试或 `state> compacting` 判定真实长任务能力可用；该验收能暴露 compact summary / provider replay / MCP tool 可用性 / tool result 配对 / 真实模型 tool-loop 在真实模型路径下的组合问题。 |
| 真实 REPL abort/resume/compact 交错 | Gated；手动验收通过 | 2026-06-19 手动 harness 使用真实 REPL、`claude-haiku-4-5`、`--no-thinking`、checkpoint recovery 模糊任务和真实 Claude Code provider。三轮运行中，第一轮暴露 `/resume` 后 unsigned thinking 被 replay 成 Claude invalid input，第二轮暴露 REPL 重复渲染同一 abort block，第三轮通过：`/abort` 回 idle、`/resume` 完成 12 个 checkpoint、usage-based auto compact 后继续 shell tool、manual `/compact` 后 follow-up shell 验证仍可用。详细记录见 `docs/repl-acceptance/compact-abort-resume.md`。 | `docs/repl-acceptance/compact-abort-resume.md`；`jsonl-output.test.ts` 锁住 unsigned thinking replay；`renderer.test.ts` 锁住 abort/error patch 去重；`session.test.ts` 和 `compaction.test.ts` 覆盖 abort/resume/compact runtime 收敛 | 防止 abort/resume 只在 deterministic session 层通过，真实 Claude Code replay 却因 thinking payload 失败；也防止长程 REPL patch replay 把同一个终止块重复呈现成大量噪声。 |
| 真实 REPL context cache stability | Gated；手动验收通过 | `docs/repl-acceptance/context-cache-stability-marathon.md` 使用真实 REPL 和 `claude-haiku-4-5` 跑同一 workspace 中的稳定追加任务。前三个 one-tool turns 按真实 provider 报告 cache 为 0；随后 multi-tool continuation 在 compact 前报告 `cache_read=4112 cache_write=5124`，manual `/compact` 回 idle，compact 后 multi-tool continuation 报告 `cache_read=10686 cache_write=6449`，最终文件从 `CACHE_STEP_01` 连续到 `CACHE_STEP_15`。 | `docs/repl-acceptance/context-cache-stability-marathon.md`；provider/AgentClient/REPL usage tests；`context-cache.test.ts` | 防止只在 deterministic 层验证 cache 字段存在，却不知道真实 provider 何时上报 cache hit；同时验证 compact 后不会让 provider continuation 永久退化成 cold context。 |
| 真实 REPL runaway tool loop guard | Gated；手动验收通过 | `docs/repl-acceptance/runaway-tool-loop-guard.md` 用真实 REPL 和 `claude-haiku-4-5` 跑 sentinel polling 模糊任务。前两轮分别暴露 errored tool output 未显示、suppression 被错误当成 terminal result 后吞掉后续输入；最终通过：第 7 次 identical `cat sentinel.txt` 被 suppression，REPL 显示 `tool-error>` 细节，turn 回 idle，后续 `wc -c sentinel.txt` 直接执行并返回 `8`。 | `docs/repl-acceptance/runaway-tool-loop-guard.md`；`renderer.test.ts` 锁住 `tool-error>` 呈现；`tools.test.ts` 锁住 repeated-shell suppression 非 terminal 和 AgentSession continuation | 防止真实模型在等待型任务里无界重复工具，也防止 guard 虽然触发但 operator 看不到原因，或 guard 破坏 provider continuation 导致下一条用户消息被旧循环吞掉。 |
| 交互式 shell 操作在 REPL 中顺畅 | 有效 / Gated | `process.test.ts` 用 Claude Code 协议 fixture 发起一个等待 stdin 的 `shell_exec`，REPL 进程从 stdout 解析 shellId 后发送 `/input <shellId> typed-from-repl`，断言同一 REPL stdout 出现 `fixture-input:typed-from-repl`；证明最终用户命令路径能写入 active shell 并渲染结果。`real-repl.e2e.test.ts` 的 `DEMI_REPL_FUZZY_SHELL_E2E=1` 真实模型 smoke 已覆盖模糊任务下 wait/input/abort 选择，3 次 attempt 通过。默认验证：`bun test packages/repl/src`，15 pass / 3 skip；gated 验证：`DEMI_REPL_FUZZY_SHELL_E2E=1 bun test packages/repl/src/__tests__/real-repl.e2e.test.ts --timeout 600000`，1 pass / 1 skip。 | `packages/repl/src/__tests__/process.test.ts`、`packages/repl/src/__tests__/real-repl.e2e.test.ts` gated smoke | 用来发现 REPL `/input` 虽然命令分发存在，但真实进程/AgentServer/transport/active shell 写入链路断裂；同时发现真实模型虽然能看到工具说明，但仍可能误用控制面的策略问题。 |

### 5.18 just-bash 子模块

Owner：`packages/just-bash`

| 测试点 | 审查结论 | 审查记录 | 候选覆盖 / 待核对 | 能发现或规避的问题 |
|---|---|---|---|---|
| demi 依赖的 parser protection | 有效 | `bun run test:just-bash-core` 跑 `parser-protection.test.ts`，覆盖超长输入、深嵌套、超多 token、病理 brace/redirection/quote pattern 和执行期 expansion limit；93 个 just-bash core 测试整体通过。 | `bun run test:just-bash-core` | 防止 parser 在安全边界或保护逻辑上退化。 |
| parser edge cases | 有效 | 同一脚本跑 `parser-edge-cases.test.ts`，大量断言 quoting、escape、变量展开、空白、redirection、operator precedence、组合命令的 stdout/exit 行为；能发现 agent 常见脚本语法解析退化。验证同上。 | `bun run test:just-bash-core` | 防止 agent 常见脚本语法在升级 just-bash 后突然解析错误。 |
| parse errors | 有效 | 同一脚本跑 `parse-errors.test.ts`，覆盖 if/for/while/until/function/local/command error，并对未闭合 quote、缺 redirect target、前置 pipe、尾随 pipe、尾随 `&&`、尾随 `||` 精确断言 exit 2 和 syntax stderr；尾随 operator 断言先在旧实现上失败，修复 parser 后通过。验证同上。 | `bun run test:just-bash-core` | 防止非法语法被误解析并执行。 |
| coverage-guided fuzzing 能记录真实解释器特征 | Gated | `fuzz-coverage.test.ts` 打开 interpreter coverage，`CoverageTracker` 以 bash command node、shell builtin、bash expansion、command flag 作为已知基线，记录每轮命中的 feature 和最小 corpus。smoke 验证：`FUZZ_RUNS=1 pnpm vitest run src/security/fuzzing/__tests__/fuzz-coverage.test.ts`，11 pass。 | `packages/just-bash/packages/just-bash/src/security/fuzzing/__tests__/fuzz-coverage.test.ts` | 防止 fuzz 测试看似运行但 coverage 事件丢失、报告永远只统计已覆盖项，无法暴露解释器或命令 flag 的真实覆盖缺口。 |
| upstream bash/awk/sed/grep/jq 等 spec/comparison 测试 | Gated / 条件项 | 子模块 comparison 套件已单独验证通过：默认 `node` wrapper 在该子目录会挂起，改用 Node 22 运行 `/Users/plutonist/.nvm/versions/node/v22.14.0/bin/node node_modules/.bin/vitest run --config vitest.comparison.config.ts`，33 files / 538 tests passed。完整 `test:run` 对应命令已尝试，出现大量现存 failures/timeouts（security/js-exec/python/sqlite/browser bundle、部分 bash spec、help/tar bundle 等），运行 2 分多后停止；它不属于当前最小稳定 agent 默认 green gate，只有扩大 bash 兼容范围时才进入 triage。 | comparison 可作为已记录 gated evidence；完整 upstream `test:run` 是第 6 章条件项，不进入默认入口 | comparison 能发现主仓库关键路径以外的命令兼容性回归；完整 upstream 失败记录防止误以为子模块全量套件已经可靠通过。 |

## 6. 当前剩余优先补测顺序

1. 如果后续引入独立 REPL scroll UI，再补交互滚屏测试。
2. 如需扩大 bash 兼容范围，先 triage `5.18` just-bash 完整 `test:run` 的现存 failures/timeouts；主仓库默认脚本只覆盖 demi 当前依赖的 parser 核心子集。

## 7. 新增测试放置规则

- 单模块行为放在 owner package 的 `src/__tests__/`。
- 跨模块行为放在最接近不变量 owner 的 package：session/runtime 放 `agent`，coding workflow 放 `coding-agent`，协议收敛放 `agent`。
- 涉及真实 CLI、真实模型、网络、本机登录状态或交互 UI 的测试必须 gated，不进入默认 `bun run test`。
- 测试应断言真实 artifact、provider request、transcript blocks、events、session state、transport frames 或文件内容。
- 新增测试点必须说明能发现或规避的具体问题；说不清风险的问题不进入矩阵。
- 新增 architecture/workflow 约束时，先更新 `docs/agent-rewrite-plan.md` 或本文件，再补测试。

## 8. 测试有效性门禁

新增或重写测试时，必须先证明测试本身有意义。覆盖率、快照数量、mock 调用次数都不能单独作为有效性证据。

| 门禁 | 要求 | 适用位置 | 防止的问题 |
|---|---|---|---|
| 风险先行 | 每个测试先写清楚要捕获的失败模式，再写 fixture 和断言 | 所有测试矩阵项 | 防止为了覆盖率或已有实现形状凭空写测试。 |
| 产品边界 oracle | 优先断言 provider request、transcript、session state、transport frame、真实文件、真实 shell output | agent/session/provider/AgentClient/coding workflow | 防止只验证 mock 自洽，真实用户路径仍然坏。 |
| 反证检查 | P0 测试落地后，用临时破坏实现或 fixture 的方式确认测试会失败 | compact、context cache、tool pair、marathon | 防止测试永远绿但不能发现实际回归。 |
| Mutation / fault injection | 人为注入 tool pair 断裂、summary 缺失、usage 错误、provider stream 异常、storage 丢 block | agent、provider、store/AgentClient | 防止只覆盖 happy path，错误路径没有有效 oracle。 |
| Property-based / stateful | 对 append-only transcript、queue、phase、tool_call 状态生成多步 action 序列并校验不变量 | AgentSession、Transcript、Shell control | 防止少量手写场景漏掉 action 交错和状态漂移。 |
| Metamorphic testing | 同一任务经过 retry/resume/compact/persist/reopen 后，模型可见上下文满足等价或明确变化规则 | replay、compact、context cache | 防止只测单一路径，无法发现等价操作后的上下文偏移。 |
| Differential / reference testing | shell 语义对比 just-bash/upstream 或真实 shell；provider message shape 对比参考 fixture | shell、provider conversion | 防止本地实现悄悄偏离已有可靠语义。 |
| Scenario / marathon | 单会话累计执行 send、queue、tool、error、abort、resume、retry、compact，并逐步断言不变量 | agent、coding workflow | 防止单点测试都过，但长任务累计后状态损坏。 |
| Golden snapshot 有结构化入口 | snapshot 只用于 request/frame 这类结构化 artifact，并配合字段级断言 | compact request shape、transport | 防止快照变成不可审查的大文本批准。 |
| Gated smoke 只补充不替代 | 真实 Claude/REPL smoke 验证真实模型随机路径；默认 deterministic 测试仍负责契约 | REPL、真实 provider | 防止把不可复现的人工验收当成核心正确性证明。 |

P0 测试合入前至少要满足：有明确失败模式、有产品边界 oracle、有一次反证检查；涉及状态机或上下文压缩时，还要覆盖多步 action 序列或等价路径。

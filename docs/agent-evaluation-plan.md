# Agent 性能评估体系设计

| | |
|---|---|
| 日期 | 2026-06-23 |
| 状态 | 草案 |
| 范围 | agent task benchmark、Evaluator 监督验收循环、评分、artifact、CI/gated 验收 |

## 1. 目标

建立一套可重复、可审计的 agent 性能评估体系，用来回答两个问题：

1. Worker agent 在无人监督时能独立完成多少任务。
2. 当同一个 Evaluator agent 负责验收和催促时，需要多少监督成本才能把任务推到完成。

评估体系必须覆盖：

- 任务达成率；
- 达成任务所需用户/监督轮次；
- provider request、tool call、wall time、token 和 cache 成本；
- Worker 假完成、卡住、错误收敛、被催促后继续完成的行为；
- queue、steer、retry、resume、compact、abort 这些 session working 方式对完成率和成本的影响。

本体系不是只跑一次 prompt 看最终输出。真实 agent 工作流里，模型经常会提前停下、没有验证、遗漏要求、卡在长命令、或声称完成但没有完成。因此评测运行模型必须内建监督验收循环，并把监督介入本身计入分数。

## 2. 核心原则

1. **Evaluator 同时是裁判和监督者。** 不拆成 Judge 和 Supervisor 两个 agent。Evaluator 每轮先验收，未通过时生成下一条监督指令。
2. **Oracle 是证据来源，不是 agent。** 测试命令、文件断言、HTTP check、schema check、workspace diff、运行日志是 evidence。Evaluator 基于 evidence 做 verdict 和 intervention。
3. **最终结果和监督成本分开报告。** 同样 pass，零监督完成和靠多次高强度提示完成不是同一质量。
4. **干预必须结构化记分。** 每次 nudge、defect report、directional hint、solution hint、steer、abort/resume/retry 都要记录类型、通道、内容和 assistance score。
5. **默认不让 Evaluator 代做。** Evaluator 可以运行 oracle、读 transcript、指出缺口、催继续；默认不能直接编辑 Worker workspace 或给完整 patch。若某个 benchmark 允许 solution hint，必须重扣分并单独报告。
6. **Worker 和 Evaluator provider 资源分开计量。** Worker 的完成成本和 Evaluator 的监督成本分别统计，再汇总成 total cost。
7. **评测 artifact 必须可复盘。** 每次 run 保存 case spec、worker transcript、Evaluator decisions、oracle logs、workspace diff、metrics 和最终 verdict。
8. **deterministic benchmark 和 real-provider gated benchmark 并存。** deterministic 用于 CI 锁契约；真实 provider 用于发现模型策略、网络、provider event 和长任务真实路径问题。
9. **评测走产品协议边界。** Eval runner 通过 `AgentServer` / `AgentClient` 运行 Worker，避免直接绕过 session protocol；`AgentSession` 仍只由 `AgentServer` 实例化。

## 3. 包边界

新增 leaf package：

```text
@demi/agent-eval
```

职责：

- 读取 eval case；
- 准备 workspace fixture；
- 组装 `AgentServer`、`AgentClient`、`LocalHost`、coding harness 和 provider registry；
- 驱动 Worker task；
- 运行 oracle；
- 调用 Evaluator driver 产出 verdict / intervention；
- 向 Worker 发送 follow-up、steer、retry、resume、abort 等控制动作；
- 采集 metrics；
- 写出 run artifact 和聚合报告。

允许依赖：

- `@demi/agent`
- `@demi/coding-agent`
- `@demi/core`
- `@demi/host-local`
- `@demi/provider`
- `@demi/provider-claude-code`
- `@demi/provider-codex`
- `@demi/shell`

不得：

- 被任何 production package import；
- 把评测逻辑放进 `@demi/agent`、provider 包、REPL 或 web；
- 为了评测绕过 provider config 白名单；
- 直接实例化 `AgentSession`；
- 修改 Worker workspace，除非 action 是 Worker 自己通过 agent tools 完成，或 case 显式声明 oracle 命令会产生副作用。

REPL/web 只做少量端到端 smoke。主 benchmark 直接通过 `AgentClient` 跑 Worker，这样能稳定采集协议事件、transcript、queue/steer、tool、usage 和 artifact。

## 4. 角色模型

### 4.1 Worker Agent

被评测对象。Worker 通过 Demi agent runtime 执行任务，所有文件操作、shell 操作和 registered commands 都经正常 agent tools 完成。

Worker 的可配置项：

- provider；
- model；
- thinking effort；
- service tier；
- cwd / fixture；
- system prompt / harness；
- budgets；
- 是否允许 queue / steer / retry / resume / compact。

### 4.2 Evaluator Agent

Evaluator 是同一个裁判和监督者。它每轮检查 Worker 状态和 oracle evidence，然后输出结构化决策：

- `pass`：任务完成，结束；
- `partial`：有部分完成但不满足全部验收，可选择继续或结束；
- `fail`：明确失败但预算内可继续；
- `continue`：尚未完成，给出下一条监督指令；
- `timeout`：预算耗尽或卡死，结束。

Evaluator 可以是：

- `scripted`：规则化 evaluator，用 oracle 结果和模板生成 intervention，用于 deterministic CI；
- `llm`：模型 evaluator，模拟真实监督者，适合 gated 或 nightly；
- `human-recorded`：人工验收记录导入，用于少量复杂真实 REPL/web 验收归档。

无论是哪种实现，输出 schema 一致。

### 4.3 Oracle

Oracle 是证据来源。常见 oracle：

- shell command：`bun test ...`、`npm test`、`git diff --check`；
- file assertion：文件存在、内容包含、JSON/schema 匹配；
- HTTP assertion：dev server 响应、页面文本、API schema；
- transcript assertion：是否出现 tool call、steer、queue、abort、resume；
- workspace diff assertion：只改允许路径、没有删除 fixture、没有生成大文件；
- artifact assertion：截图、日志、产物路径。

Oracle 不负责打分，只给 evidence。Evaluator 基于 evidence 判定和催促。

## 5. Eval Case Schema

每个 case 是一个可版本化文件，建议放在：

```text
packages/agent-eval/cases/<category>/<case-id>.json
```

核心 schema：

```ts
interface AgentEvalCase {
  id: string
  title: string
  category:
    | 'coding'
    | 'debugging'
    | 'shell-control'
    | 'long-context'
    | 'steer-queue'
    | 'web'
    | 'refactor'
  difficulty: 'small' | 'medium' | 'large'
  fixture: {
    source: string
    copyMode: 'fresh-copy'
    ignore?: string[]
  }
  worker: {
    provider: 'stub' | 'claude-code' | 'codex'
    modelId?: string
    thinkingEffort?: string | null
    serviceTierId?: string | null
  }
  evaluator: {
    driver: 'scripted' | 'llm' | 'human-recorded'
    provider?: 'claude-code' | 'codex'
    modelId?: string
    rubric: string[]
    interventionPolicy: InterventionPolicy
  }
  task: {
    prompt: string
    successCriteria: string[]
    explicitNonGoals?: string[]
  }
  budgets: {
    maxWallMs: number
    maxWorkerTurns: number
    maxProviderRequests: number
    maxToolCalls: number
    maxEvaluatorChecks: number
    maxInterventions: number
    maxAssistanceScore: number
  }
  oracle: OracleSpec[]
  scoring?: Partial<ScoringPolicy>
}
```

Oracle spec：

```ts
type OracleSpec =
  | {
      type: 'command'
      name: string
      command: string[]
      cwd?: string
      timeoutMs: number
      expectedExitCode?: number
      stdoutIncludes?: string[]
      stderrExcludes?: string[]
    }
  | {
      type: 'file'
      path: string
      mustExist?: boolean
      textIncludes?: string[]
      jsonPathEquals?: Array<{ path: string; value: unknown }>
    }
  | {
      type: 'transcript'
      assertions: TranscriptAssertion[]
    }
  | {
      type: 'diff'
      allowedPaths?: string[]
      forbiddenPaths?: string[]
    }
```

## 6. 运行循环

运行循环由 EvalRunner 驱动：

```text
load case
  -> create run directory
  -> copy fixture to isolated workspace
  -> open Worker AgentClient
  -> send initial task
  -> wait for checkpoint
  -> run oracle
  -> Evaluator decides
      -> pass: finalize
      -> continue/fail/partial within budget: deliver intervention
      -> timeout/budget exhausted: finalize
  -> repeat
```

Checkpoint 触发条件：

- Worker phase 回到 `idle`；
- Worker 明确输出完成声明；
- Worker 长时间无 transcript/tool progress；
- Worker 进入错误状态；
- Worker 有 queued input drain 完成；
- case 要求 active-turn check，例如 steer/queue benchmark 中 shell gate running 后注入 steer。

Evaluator intervention 的投递通道：

```ts
type InterventionChannel =
  | 'send'      // Worker idle 或需要下一 turn 继续
  | 'steer'     // Worker active turn 需要同 turn 纠偏
  | 'retry'
  | 'resume'
  | 'abort'
  | 'compact'
```

通道选择规则：

- Worker idle 且未完成：默认 `send`，记为 supervised follow-up。
- Worker active 且目标是影响当前 turn：使用 `steer`，计入 steer intervention。
- Worker active 且无进展超过 case budget：Evaluator 可 `steer` 催促；若仍无进展，可 `abort`，再 `resume` 或 `send`，这些都计分。
- Worker error 后：可 `retry` 或 `send`，视 case policy。
- context pressure 或 oracle 要求：可 `compact`，但要计 control intervention。

## 7. Evaluator Decision Schema

每次 Evaluator 检查写一条 JSONL：

```ts
interface EvaluatorDecision {
  checkIndex: number
  timestamp: string
  verdict: 'pass' | 'partial' | 'fail' | 'continue' | 'timeout'
  confidence: 'low' | 'medium' | 'high'
  evidence: Array<{
    source: 'oracle' | 'transcript' | 'diff' | 'runtime'
    ref: string
    summary: string
  }>
  missingRequirements: string[]
  falseDoneDetected: boolean
  intervention?: {
    channel: InterventionChannel
    type:
      | 'nudge'
      | 'oracle_evidence'
      | 'defect_report'
      | 'directional_hint'
      | 'solution_hint'
      | 'control_action'
    message: string
    assistanceScore: number
    rationale: string
  }
}
```

Evaluator 输出约束：

- 必须引用 oracle 或 transcript evidence；
- verdict 为 `pass` 时，所有 `successCriteria` 必须被 evidence 覆盖；
- verdict 非 `pass` 且还有预算时，必须给出 intervention 或明确说明等待原因；
- 不允许输出完整 patch，除非 case policy 显式允许 `solution_hint`；
- 不允许把 rejected steer 自动转成 queue，必须记录 rejected result 和下一步选择；
- 若 Worker 声称完成但 oracle 未通过，`falseDoneDetected=true`。

## 8. 干预强度和计分

干预强度建议默认表：

| 类型 | 说明 | 默认分 |
|---|---|---:|
| `nudge` | 泛化催促，如“继续完成验收”“你还没跑测试” | 2 |
| `oracle_evidence` | 提供原始测试/命令失败输出，不解释修法 | 4 |
| `defect_report` | 指出缺失 requirement 或具体失败点 | 6 |
| `directional_hint` | 指向模块、路径、可能原因 | 12 |
| `solution_hint` | 接近给出修法或代码结构 | 25 |
| `control_action` | abort/retry/resume/compact 等控制动作 | 6 |

通道额外成本：

| 通道 | 额外分 |
|---|---:|
| `send` | 0 |
| `steer` | 1 |
| `retry` | 2 |
| `resume` | 2 |
| `abort` | 4 |
| `compact` | 1 |

最终综合分只是摘要，原始指标必须保留。

```text
outcome_points:
  pass      100
  partial    45
  fail        0
  timeout     0

score =
  outcome_points
  - assistance_score
  - false_done_count * 8
  - over_budget_penalty
```

成功率报表必须至少包含：

- `autonomous_success_rate`：无 intervention pass；
- `supervised_success_rate`：有 intervention 后 pass；
- `strict_success_rate`：pass 且 `assistance_score <= threshold`；
- `pass_after_1_intervention_rate`；
- `fail_after_budget_rate`。

## 9. 指标

### 9.1 Outcome Metrics

- `status`: `pass | partial | fail | timeout`
- `autonomousPass`: boolean
- `supervisedPass`: boolean
- `strictPass`: boolean
- `falseDoneCount`
- `flaky`: 同一 case 多次 run 结果不一致

### 9.2 Interaction Metrics

- `workerTurns`: 初始 task + Evaluator follow-up send 次数；
- `queuedTurns`;
- `steerCount`;
- `retryCount`;
- `resumeCount`;
- `abortCount`;
- `compactCount`;
- `evaluatorChecks`;
- `interventionCount`;
- `assistanceScore`;
- `interventionByType`;
- `interventionByChannel`。

### 9.3 Runtime Metrics

Worker 与 Evaluator 分开统计：

- `providerRequests`;
- `toolCalls`;
- `shellCommands`;
- `wallMs`;
- `activeMs`;
- `idleWaitMs`;
- `inputTokens`;
- `outputTokens`;
- `cacheReadTokens`;
- `cacheWriteTokens`;
- `providerErrors`;
- `contextCompactions`。

### 9.4 Efficiency Metrics

- `providerRequestsToPass`;
- `workerTurnsToPass`;
- `interventionsToPass`;
- `wallMsToPass`;
- `toolCallsToPass`;
- `tokenCostToPass`;
- `costPerPass`;
- `supervisorBurdenScore = evaluatorChecks + interventionCount + assistanceScore`。

## 10. Artifact Layout

每次运行写入：

```text
eval-runs/
  20260623T120000Z/
    summary.json
    summary.md
    cases/
      coding.fix-parser/
        attempt-01/
          case.json
          run.json
          metrics.json
          worker-events.ndjson
          worker-transcript.json
          evaluator-decisions.jsonl
          oracle/
            001-bun-test.stdout
            001-bun-test.stderr
            001-bun-test.json
          workspace.diff
          final-workspace/
```

`run.json`：

```ts
interface EvalRunArtifact {
  runId: string
  caseId: string
  attempt: number
  startedAt: string
  finishedAt: string
  worker: ProviderRunConfig
  evaluator: EvaluatorRunConfig
  finalStatus: 'pass' | 'partial' | 'fail' | 'timeout'
  finalScore: number
  metrics: EvalMetrics
  artifactPaths: Record<string, string>
}
```

Artifacts 必须足够支持事后审计：为什么判 fail、Evaluator 给了哪些帮助、Worker 何时假完成、queue/steer 顺序是否正确。

## 11. CLI 和运行方式

最终 CLI：

```sh
bun run agent-eval run --case packages/agent-eval/cases/coding/fix-parser.json
bun run agent-eval run --suite packages/agent-eval/suites/core.json --attempts 5
bun run agent-eval report eval-runs/20260623T120000Z
```

常用模式：

```sh
# deterministic CI
bun run agent-eval run --suite core-deterministic --provider stub

# real provider gated
DEMI_CODEX_EVAL=1 bun run agent-eval run --suite codex-smoke --provider codex --attempts 3

# compare models
bun run agent-eval compare --suite coding-small --models codex:gpt-5.4,claude-code:claude-opus-4-8
```

CLI exit code：

- `0`: suite 完成且满足 configured pass threshold；
- `1`: runner/system error；
- `2`: suite 完成但低于 pass threshold；
- `3`: case schema invalid；
- `4`: gated provider auth/config missing。

## 12. Benchmark Suite 结构

建议 suite 分层：

### 12.1 Core Deterministic

默认 CI 可跑，使用 stub provider 或 scripted provider。

覆盖：

- evaluator loop；
- false done；
- intervention scoring；
- steer vs queue；
- retry/resume/abort accounting；
- artifact 写出；
- oracle command failure。

### 12.2 Coding Small

真实 coding harness，轻量 fixture：

- failing test 修复；
- 小 refactor；
- editor/todo command；
- shell command + file assertion。

### 12.3 Shell Control

覆盖：

- long foreground command；
- wait；
- input；
- abort；
- no-progress nudge；
- repeated command guard。

### 12.4 Long Context

覆盖：

- preflight compact；
- tool-heavy compact；
- resume after abort；
- cache usage；
- multi-turn continuation。

### 12.5 Steer / Queue

覆盖：

- active turn running 时 Evaluator `steer`；
- 普通 message 进入 queue；
- steer materialize 后 active turn 先完成；
- queued turn 后完成；
- rejected steer 不自动 queue。

### 12.6 Real Provider Smoke

gated / nightly：

- Codex real provider；
- Claude Code real provider；
- real REPL smoke；
- optional web smoke；
- 每个 provider 少量高信号 case，避免把随机真实模型行为放进默认 CI。

## 13. 测试覆盖计划

实现时新增测试模块并在 `docs/testing.md` 登记：

| 模块 | 覆盖 |
|---|---|
| `packages/agent-eval/src/__tests__/case-schema.test.ts` | case schema validation、budget defaults、invalid oracle rejection |
| `packages/agent-eval/src/__tests__/evaluator-loop.test.ts` | pass、false done、continue、budget exhausted、Evaluator intervention loop |
| `packages/agent-eval/src/__tests__/intervention-scoring.test.ts` | intervention type/channel 分值、assistance score、strict pass threshold |
| `packages/agent-eval/src/__tests__/metrics.test.ts` | provider requests、tool calls、turns、steer、retry/resume/abort/compact、tokens 聚合 |
| `packages/agent-eval/src/__tests__/artifact.test.ts` | run directory、events、transcript、oracle logs、workspace diff、summary report |
| `packages/agent-eval/src/__tests__/oracle.test.ts` | command/file/transcript/diff oracle |
| `packages/agent-eval/src/__tests__/workspace-fixture.test.ts` | fixture fresh copy、ignore rules、final diff |
| `packages/agent-eval/src/__tests__/agent-client-integration.test.ts` | 通过 `AgentServer`/`AgentClient` 跑 Worker，不直接实例化 `AgentSession` |
| `packages/agent-eval/src/__tests__/steer-queue-eval.test.ts` | Evaluator 使用 steer 催 active turn，queued input 不抢跑 |
| `packages/agent-eval/src/__tests__/real-provider.e2e.test.ts` | gated real Codex / Claude smoke，默认 skip |

验收命令：

```sh
bun test packages/agent-eval/src
bun run typecheck
```

如果新增 package，需要同步：

- root `package.json` workspaces；
- `docs/package-boundaries.md`；
- `docs/testing.md`；
- `packages/core/src/__tests__/platform-entrypoints.test.ts` 中的 package graph / manifest coverage。

## 14. 接受标准

该评估体系完成时必须满足：

1. 可以用一个 CLI 跑单个 case、suite 和多 attempt；
2. 可以区分 autonomous pass、supervised pass、partial、fail、timeout；
3. Evaluator 每次检查都有结构化 decision；
4. Evaluator 既负责判定也负责催促，不引入第二个 judge/supervisor agent；
5. Oracle evidence 和 Evaluator verdict 可审计；
6. 每次 intervention 有 type、channel、message、assistance score；
7. Worker 与 Evaluator 的 provider request、tool、token、wall time 分开统计；
8. queue / steer / retry / resume / abort / compact 都进入指标；
9. run artifact 足以重放评测结论；
10. deterministic suite 可进 CI；
11. real-provider suite gated，不污染默认稳定测试；
12. docs/testing.md 明确记录测试模块与覆盖意图。

## 15. 与现有系统的关系

- `@demi/agent` 继续只负责 runtime、AgentServer、AgentClient 和 session protocol。
- `@demi/provider-*` 继续只负责 provider mapping，不感知评测。
- `@demi/repl` 和 `@demi/web` 可以作为少量 e2e smoke 入口，但不是主评测 runner。
- `@demi/agent-eval` 是 product leaf，类似 REPL/web，只负责组装和运行评测。
- 评测结果可以反向指导 prompt、runtime、provider、shell 和 UI 改进，但不得把 benchmark 特化逻辑塞进 Worker runtime。

## 16. 需要避免的错误设计

- 只用最终 LLM judge 判定，不跑 oracle；
- 把 Supervisor 和 Judge 分成两个互相不一致的 agent；
- 只报告 pass rate，不报告 intervention cost；
- 把 steer 算作普通 user turn，导致 queue/steer 语义被掩盖；
- Evaluator 给 patch 后仍按自主完成计分；
- 真实 provider smoke 替代 deterministic CI；
- 为了评测直接操作 `AgentSession` 或 provider internals；
- artifact 只保存 summary，无法复查 false done 或高强度帮助。

## 17. 初始 Case 建议

第一批 case 应覆盖最小高信号面：

1. `coding.fix-failing-test.small`：给一个 failing test，要求 Worker 修复并跑测试。
2. `coding.false-done.small`：Worker scripted 第一次声称完成但 oracle fail，Evaluator 催继续。
3. `shell.long-running-input.medium`：长命令需要 wait/input，记录 tool control 成本。
4. `steer-queue.active-turn.medium`：active turn gate 中 Evaluator steer，同时普通输入 queue。
5. `long-context.tool-compact.medium`：工具结果后触发 compact，继续完成。
6. `web.smoke.small`：web server e2e，确认协议路径和 UI action 基本可用。
7. `real.codex.steer-queue.gated`：复用真实 Codex steer / queue 验收思想，作为 gated suite。

这些 case 不分 MVP 阶段；它们共同定义最终体系需要覆盖的能力面。实现可以按 checkpoint 交付，但文档和 schema 应一次性对齐最终形态。

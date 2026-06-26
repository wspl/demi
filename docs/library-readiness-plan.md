# Demi 库化与开源就绪方案

| | |
|---|---|
| 日期 | 2026-06-26 |
| 状态 | 架构 Review + 改造计划 |
| 范围 | 整个 monorepo:打包/分发、代码质量、文档、开源工程化 |

本文件记录一次完整的架构 Review 发现,以及把 Demi 从"快糙猛实现"改造成**可维护、可作为库被各种软件引用、可开源、可广泛使用**的总纲。它是后续改造工作的计划来源(配合 `docs/package-boundaries.md` 与 `docs/agent-rewrite-plan.md`)。

## 1. 目标与结论

目标:让这套代码**架构合理、复用合理、质量高**,别人能 `npm install` 直接用,能开源,能被广泛采用。

结论:**架构骨架已经相当好,而且是被测试强约束的,不需要推倒重来。** 真正缺的是四类工作:

1. 把它**打包成可分发的库**(目前只能作为 Bun workspace 裸 TS 源码消费)——这是 blocker;
2. **拆几个巨型类**、**provider 去重**(质量/可维护性);
3. **用户文档 / 示例 / 扩展指南**(采用门槛);
4. **开源工程化**(license、CI、发布、贡献者文档)。

底层分层是对的,所以改造风险低、可分阶段交付。

## 2. 现状评估

### 2.1 已经达标(不要重做)

- **包边界**:`docs/package-boundaries.md` 是一份成熟的边界契约,并由 `packages/core/src/__tests__/platform-entrypoints.test.ts` **强制执行**:环依赖检测、Node-only 静态泄漏检测、public export 泄漏检测、依赖声明检测、`core`/`provider` 不得含具体 provider/catalog/backend 名。
- **依赖图无环、分层正确**:`core → none`;`provider → core`;`shell → just-bash`;`host-local → shell`;`agent → core/provider/shell`;`coding-agent → agent/core/shell`;具体 provider `→ core/provider`;`repl`/`web`/`agent-eval` 是叶子产品。
- **依赖卫生干净**:test-only 的上行依赖正确放在 devDependencies(如 `provider-codex`/`provider-claude-code` 的 `@demi/agent`、`@demi/shell`),不污染安装体积。
- **Host 抽象**:`Host = { defaultCwd, fs, process, store }` 平台中立,`@demi/host-local` 是 Node 参考实现,为 remote/容器/sandbox 后端预留扩展(`docs/package-boundaries.md` §`@demi/shell`)。
- **Provider 契约 + 传输协议**:抽象 `AgentProvider`/`ProviderRun`/`InferenceRequest`/`ProviderEvent`;传输无关的 `AgentClient`/`AgentServer`(stdio + websocket 两种 transport);`web-ui` 消费注入的 `AgentClient`。扩展点设计到位。
- **测试纪律 + 开源意识**:全量 ~448 deterministic 测试 + gated 真机验收(`docs/testing.md`、`docs/repl-acceptance/`);`scripts/check-public-registry.ts` 已在防止内部 npm 镜像泄漏(说明开源已是既定意图)。

### 2.2 主要差距(概览)

打包/分发(blocker)、巨型类、provider 重复、用户文档缺失、开源工程化缺失。详见 §3。

## 3. 详细发现与改造(分阶段)

severity 含义:**blocker** = 不做就不能当库用 / 不能开源;**high** = 严重影响可维护性或采用;**medium/low** = 改善项。

### Phase A —— 可分发(packaging blockers)

| # | severity | 问题 | 证据 | 改造 |
|---|---|---|---|---|
| A1 | blocker | **没有构建产物**。所有包通过根 `tsconfig.json` 的 `paths` 映射到 `packages/*/src/index.ts`,`noEmit:true`,且 `moduleResolution:bundler` + `types:["bun"]` + `verbatimModuleSyntax` 绑死 Bun/bundler。`exports` 全指向 `./src/index.ts`(裸 TS)。别人 `npm install @demi/agent` 拿到的是 `.ts`,没有 `.js`/`.d.ts`,非 Bun 消费者用不了。 | `tsconfig.json`;各 `packages/*/package.json` 的 `exports` | 引入构建(`tsup`/`unbuild`/`tsc -b`)产出 **ESM `.js` + `.d.ts`**;`exports` 改成 `import`/`types` 条件导出到 `dist`;`prepublishOnly` 跑构建;每包独立 `tsconfig`。 |
| A2 | blocker | 全部 `private:true`、`version:"0.0.0"`、无 `description`/`repository`/`license`/`keywords`/`author`。不可发布。 | 所有 `packages/*/package.json` | 补元数据 + 真实版本 + `publishConfig.access:"public"`;引入 **changesets** 管理独立版本与发布。 |
| A3 | blocker | **无 LICENSE**;`just-bash` 是 git submodule(独立 monorepo `just-bash-monorepo`),消费者 `npm install` 装不到它,fork 来源/许可证也未交代。 | `.gitmodules`;`docs/package-boundaries.md:15`;`packages/just-bash/package.json` | 选 license(见 §7);把 `just-bash` 作为独立 npm 包发布,或 vendored 进 `@demi/shell`;交代 fork provenance 与许可证。 |
| A4 | low | 根 tsconfig 有两条死 `paths`:`@demi/shell/local-host` → `packages/shell/src/local-host.ts`、`@demi/shell/store` → `packages/shell/src/store.ts`,二者均不存在(只有 `storage.ts`)。 | `tsconfig.json` | 删除/修正这两条映射。 |

### Phase B —— 质量 / 可维护性(拆巨型类 + 去重)

| # | severity | 问题 | 证据 | 改造 |
|---|---|---|---|---|
| B1 | high | **`AgentSession` god-object**:turn 生命周期 + action 队列 + steer + compaction + yield-wakeup + provider 流式 + 工具执行 + 分层 abort + transcript 提交,全在一个类。难单测、难读、难复用。 | `packages/agent/src/session.ts`(1480 行) | 拆为协作者(详见 §4.1),`AgentSession` 退化为薄编排层。 |
| B2 | high | **`BashEnvironment` god-object**:shell session + 前台/后台进程 + 输出 sink + command record + artifact 持久化 + `/@` 虚拟 FS。 | `packages/shell/src/environment.ts`(1367 行) | 拆为 `ShellSessionManager`/`ForegroundController`/`CommandRecordStore`/`CommandArtifactStore`+`VirtualCommandFs`(详见 §4.2)。 |
| B3 | high | **Provider 重复**:4 个 provider 各自重写 SSE 读取、event 映射、错误归一、usage 映射、endpoint/env/key 解析、catalog 形状;`provider-openai-api/provider.ts` 单文件 1013 行。 | `packages/provider-openai-api/src/provider.ts`(1013)、`provider-anthropic-api`(542)、`provider-claude-code`(502)、`provider-codex/*` | 抽 `@demi/provider-kit` 或 `@demi/provider` 内部 helpers(详见 §4.3)。直接改善"加新 provider"体验。 |
| B4 | medium | `repl/src/index.ts` 既是 878 行应用(arg parse + render + input loop + 组装),又是包的 `index.ts`。 | `packages/repl/src/index.ts`(878 行) | 拆 `args`/`renderer`/`loop`/`compose` 模块,`index.ts` 变薄入口。 |
| B5 | medium | **无 lint/format**(无 eslint/biome/oxlint/prettier 配置,虽装了 `oxc-parser`)。风格一致性靠人。 | 根目录 | 上 Biome 或 oxlint + prettier(见 §7),纳入 CI,降低贡献摩擦。 |
| B6 | low | public export 普遍**缺 TSDoc**;`@demi/core` 的 `Block` 等类型是事实上的公共契约但注释稀疏。 | `packages/*/src/index.ts`、`packages/core/src/index.ts` | 给所有 public export 补 TSDoc(feeds typedoc + 编辑器悬浮)。 |

### Phase C —— 文档 / DX / 采用

- **C1 顶层 README + Quickstart + `examples/`**:目前 `docs/` 全是**内部设计记录**(`agent-rewrite-plan`、各 `*-research`、`tool-rendering-spec` 等),没有一篇面向使用者。最高杠杆:一个 ~20 行能跑的最小示例(组装 providers + `LocalHost` + coding harness + 一个 transport)。
- **C2 每包 README + typedoc API 参考**(配合 B6 的 TSDoc)。
- **C3 三篇关键扩展指南**:**Add a Provider**、**Implement a Host**、**Embed the UI**(供 `AgentClient`)。这三条正是"被各种软件引用"的核心扩展点。
- **C4 文档分家**:内部设计记录移到 `docs/design/`,用户文档独立(可单独建站)。

### Phase D —— 开源工程化

- **D1 CI**(目前无 `.github/`):PR 跑 typecheck + test + lint + build;真机 provider 测试单独 gated;release workflow。
- **D2** `LICENSE`、`CONTRIBUTING.md`、`CODE_OF_CONDUCT.md`、`SECURITY.md`、`CHANGELOG`、issue/PR 模板。
- **D3** changesets 自动发版;在文档中交代密钥/隐私处理(`.test-cache`、claude wire log 会把全量 prompt/输出写到临时目录)。

## 4. 巨型类拆分提案(具体)

### 4.1 `AgentSession`(`session.ts`)

按现有方法归并为以下协作单元,`AgentSession` 仅保留 public API(`send`/`retry`/`resume`/`compact`/`steer`/`abort`/`subscribe`)+ transcript 持有 + 事件/快照,并编排下列单元:

- **`ActionQueue` / `TurnWorker`**:`pendingActions`、`queued`、`enqueue`、`kickWorker`、`runWorker`、`executeAction`、`dequeue`/`sendQueued`/`clearQueue`、`waitUntilDone`、idle resolvers。
- **`ProviderTurnLoop`**:`executeProviderTurn`、`streamProviderOnce`、`executePendingTools`、`providerEvents`、`buildInferenceRequest`、`applyProviderEvent`、auto-recover。
- **`SteerController`**:`steer`、`steerInternal`、`steerDelivery`、`pendingSteers`、`materializePendingSteers*`、`cancelPendingSteer`、`canceledPendingSteerIds`。
- **`CompactionController`**:`executeCompaction`、`generateCompactionSummary`、`executePreflightCompaction`、`compactToFitModel`、`applyPendingModelSwitch`。
- **`YieldScheduler`**:`scheduleYieldWakeup`、`pendingYieldWakeups`、`armPendingYieldWakeups`、`deliverYieldWakeup`、`takePendingYieldWakeup`、`enqueueHiddenSend`、`cancelOnePendingYieldWakeup`、`clearPendingYieldWakeups`。
- **分层 abort helper**:`abort`、`activeAbortTarget`、`abortQueuedAction`、`canAbortAgain`、`recordAbort`。

目标:每个单元可独立单测,`session.ts` 降到合理体量;现有 `session.test.ts` 作为回归兜底,小步重构。

### 4.2 `BashEnvironment`(`environment.ts`)

- **`ShellSessionManager`**:`shells`、`createShell`/`disposeShell`/`disposeAllShells`/`killShell`/`requireShell`、`defaultShell`/`availableDefaultShell`/`defaultShellByAgentSessionId`。
- **`ForegroundController`**:`hostSpawn`、`raceForeground`、`waitForBoundary`、`write`、`abort`、`collectAborted*`、前台进程生命周期与 boundary(timeout/aborted/foreground_appeared)。
- **`CommandRecordStore`**:`commandsById`、`createCommandRecord`、`snapshotCommand`、`requireCommand`、`releaseCommand`、`collectExited`/`finishExited`。
- **`CommandArtifactStore` + `VirtualCommandFs`**:`persistCommandArtifact`、`lookupVirtualArtifact`、`commandArtifact*`、`artifactStorageByScope`、`/@` overlay。
- **背景 job**:`startBackgroundJob`/`listBackgroundJobs`/`waitForBackgroundJob`(可独立)。
- 输出已部分在 `environment-output.ts`,继续收敛。
- `BashEnvironment` 变 facade,保留 `exec`/`status`/`write`/`abort` 对外 primitives。

### 4.3 Provider Kit(去重)

抽到 `@demi/provider-kit`(或 `@demi/provider` 内部 helpers,仅内部消费):

- **SSE / event-stream reader**(逐事件解析)。
- **stream → `ProviderEvent` 映射脚手架**(text/thinking/tool_use/response/usage/error 的通用骨架)。
- **HTTP + auth + endpoint/env/key 解析**(`anthropic-api` 与 `openai-api` 共享最多)。
- **usage 映射、model-catalog helpers**。
- 说明:`codex`/`claude-code` 走 CLI transport(不同),但 event 映射仍可共享;`anthropic-api`/`openai-api` 共享 HTTP+SSE 主体。

## 5. 开源就绪检查清单

- [ ] `LICENSE`(并解决 `just-bash` 许可证/来源)
- [ ] 顶层 `README` + 每包 `README`
- [ ] 构建产物(ESM `.js` + `.d.ts`)+ 正确 `exports`,解除 Bun 绑定
- [ ] 包元数据 + 真实版本 + changesets 发布
- [ ] `examples/` 最小可跑示例
- [ ] API 参考(typedoc)+ 扩展指南 ×3(Provider / Host / UI)
- [ ] lint/format + CI(typecheck/test/lint/build)+ 贡献者文档
- [ ] 内部设计记录与用户文档分离

## 6. 执行顺序与里程碑

1. **里程碑 1 — 能被当库用(Phase A)**:构建管线 + 元数据/版本 + license + just-bash 策略。独立、低风险,完成后 `npm install` 可用。
2. **里程碑 2 — 质量(Phase B1/B2/B3)**:拆 `AgentSession`、拆 `BashEnvironment`、provider 去重。主体工作,每个巨型类单独 checkpoint,现有测试兜底;顺带上 lint/format。
3. **里程碑 3 — 文档与采用(Phase C)**:README + examples + API 参考 + 扩展指南。
4. **里程碑 4 — 开源工程化(Phase D)**:CI + 贡献者文档 + 发布自动化,最后公开。

## 7. 待决问题(需要拍板)

- **License**:MIT(最大采用)还是 Apache-2.0(含专利授权)?需与 `just-bash` fork 的上游许可证兼容。
- **`just-bash`**:作为独立 npm 包发布,还是 vendored 进 `@demi/shell`?涉及 submodule 是否保留。
- **构建工具**:`tsup`(快、简单)/ `unbuild`(rollup,产物干净)/ `tsc -b`(零额外依赖)。
- **Lint/format**:Biome(一体、快)还是 oxlint + prettier?
- **是否抽共享渲染层**:`repl` 与 `web-ui` 的 block 渲染/工具分发是否提取为共享 presentation 包(`tool-rendering-spec.md` 已定义规则,但两端各自实现)。
- **是否仍支持非 Bun 运行时**:构建产物面向 Node 消费者是必须的;运行时(`@demi/host-local` 用 Node API)默认 Node 即可,但需明确"core/agent/provider 不绑 Bun runtime"。

# Demi 库化与开源就绪方案

| | |
|---|---|
| 日期 | 2026-06-26 |
| 状态 | 架构 Review + 改造计划 |
| 范围 | 整个 monorepo:打包/分发、**代码复用/去重**、巨型类拆分、文档、开源工程化 |

本文件记录一次完整的架构 Review 发现,以及把 Demi 从"快糙猛实现"改造成**可维护、可作为库被各种软件引用、可开源、可广泛使用**的总纲。它是后续改造工作的计划来源(配合 `docs/package-boundaries.md` 与 `docs/agent-rewrite-plan.md`)。

## 1. 目标与结论

目标:让这套代码**架构合理、复用合理、质量高**,别人能 `npm install` 直接用,能开源,能被广泛采用。

结论:**架构骨架已经相当好,而且是被测试强约束的,不需要推倒重来。** 真正缺的是四类工作:

1. 把它**打包成可分发的库**(目前只能作为 Bun workspace 裸 TS 源码消费)——blocker;
2. **代码复用/去重**:消灭散落各处、重复或同义不同名的工具函数,把通用代码收进 `@demi/utils`,并**合并相似函数**(本轮 Review 的头号关切,见 §4);
3. **拆几个巨型类**(质量/可维护,见 §5);
4. **用户文档 / 示例 / 扩展指南** 与 **开源工程化**。

底层分层是对的,改造风险低、可分阶段交付。

## 2. 现状评估

### 2.1 已经达标(不要重做)

- **包边界**:`docs/package-boundaries.md` 是成熟的边界契约,并由 `packages/core/src/__tests__/platform-entrypoints.test.ts` **强制执行**(环依赖、Node 泄漏、public export 泄漏、依赖声明、`core`/`provider` 不含具体 provider 名)。
- **依赖图无环、分层正确**;test-only 上行依赖正确放在 devDependencies。
- **Host 抽象**(`{defaultCwd, fs, process, store}`)+ Node 参考实现 `@demi/host-local`,为 remote/容器/sandbox 预留扩展。
- **Provider 契约 + 传输协议**(stdio/websocket,`web-ui` 消费注入的 `AgentClient`)。
- **测试纪律 + 开源意识**(~448 测试、gated 真机验收;`check:registry` 防内部镜像泄漏)。

### 2.2 主要差距(概览)

- **代码复用极差**:全仓库**没有任何共享 utils 包**(只有 `shell/src/bytes.ts` 和 `agent/src/__tests__/helpers.ts` 两个局部文件);通用 helper 靠复制粘贴,且大量同义不同名(详见 §4)。
- 打包/分发(blocker)、巨型类、provider 重复、用户文档缺失、开源工程化缺失。详见 §3 / §4 / §5。

## 3. 详细发现与改造(分阶段)

severity:**blocker** = 不做就不能当库用 / 不能开源;**high** = 严重影响可维护性或采用;**medium/low** = 改善项。

### Phase A —— 可分发(packaging blockers)

| # | severity | 问题 | 证据 | 改造 |
|---|---|---|---|---|
| A1 | blocker | **没有构建产物**。所有包通过根 `tsconfig.json` 的 `paths` 映射到 `src/index.ts`,`noEmit`,且 `moduleResolution:bundler` + `types:["bun"]` + `verbatimModuleSyntax` 绑死 Bun。`exports` 全指向 `./src/index.ts`(裸 TS)。`npm install @demi/agent` 拿到的是 `.ts`。 | `tsconfig.json`;各包 `exports` | 引入 **`tsdown`**(Rolldown + oxc,Vite/VoidZero 生态)产出 **ESM `.js` + `.d.ts`**,`exports` 用条件导出到 `dist`,`prepublishOnly` 跑构建。 |
| A2 | blocker | 全部 `private:true`、`version:"0.0.0"`、无 description/repository/license/keywords。 | 所有 `packages/*/package.json` | 补元数据 + 真实版本 + `publishConfig.access:"public"` + **changesets**。 |
| A3 | blocker | **无 LICENSE 文件**(`just-bash` 已声明 `Apache-2.0` 但缺文件);`just-bash` 是 git submodule(自有仓库 `wspl/just-bash`),消费者装不到。 | `.gitmodules`;`packages/just-bash/packages/just-bash/package.json` | 采用 **Apache-2.0**;为 Demi 与 just-bash 各补 `LICENSE`+`NOTICE`;`just-bash` 独立发 npm 包。 |
| A4 | low | 根 tsconfig 两条死 `paths`(`@demi/shell/local-host`、`@demi/shell/store` 指向不存在文件)。 | `tsconfig.json` | 删除/修正。 |

### Phase B —— 质量 / 可维护性

| # | severity | 问题 | 证据 | 改造 |
|---|---|---|---|---|
| B1 | **high** | **代码复用极差 / 大量重复与同义函数**(头号问题)。 | 见 §4(`isRecord`×14、error 家族×13、截断族×6+ 等) | 建 `@demi/utils` + `@demi/testkit`,**合并相似函数**,测试强制(详见 §4)。 |
| B2 | high | **`AgentSession` god-object**。 | `packages/agent/src/session.ts`(1480 行) | 拆为协作者(详见 §5.1)。 |
| B3 | high | **`BashEnvironment` god-object**。 | `packages/shell/src/environment.ts`(1367 行) | 拆为协作者(详见 §5.2)。 |
| B4 | high | **Provider 重复**(SSE/event 映射/usage/endpoint 解析/catalog)。 | `provider-openai-api/src/provider.ts`(1013)、anthropic(542)、claude-code(502) | 抽 `@demi/provider-kit`(详见 §5.3),与 §4 共同消化 provider 内的重复。 |
| B5 | medium | `repl/src/index.ts` 既是 878 行应用又是包 `index.ts`。 | `packages/repl/src/index.ts` | 拆 `args`/`renderer`/`loop`/`compose`,`index.ts` 变薄。 |
| B6 | — | **无 lint/format**。 | 根目录 | **本阶段不做**(后续如需再议)。 |
| B7 | low | public export 缺 TSDoc。 | `packages/*/src/index.ts` | 补 TSDoc(feeds typedoc)。 |

### Phase C —— 文档 / DX

- **C1** 顶层 README + Quickstart + `examples/`(~20 行能跑的最小示例:providers + `LocalHost` + coding harness + transport)。
- **C2** 每包 README + typedoc API 参考。
- **C3** 三篇扩展指南:**Add a Provider**、**Implement a Host**、**Embed the UI**。
- **C4** 文档分家:内部设计记录移 `docs/design/`,用户文档独立。

### Phase D —— 开源工程化

- **D1** CI(无 `.github/`):typecheck + test + lint + build;真机 provider 测试单独 gated;release。
- **D2** `LICENSE`/`CONTRIBUTING`/`CODE_OF_CONDUCT`/`SECURITY`/`CHANGELOG`/模板。
- **D3** changesets 自动发版;文档交代密钥/隐私(`.test-cache`、claude wire log 写全量 prompt 到临时目录)。

## 4. 代码复用:`@demi/utils`、合并相似函数、测试强制(头号工作)

### 4.1 现状(实证)

全仓库**没有任何共享 utils 包**。通用 helper 靠复制粘贴,且大量"同义不同名"。实测重复:

| helper / 函数族 | 重复 | 情况 |
|---|---|---|
| `isRecord` | **14×** | **逐字符完全相同**,散在 5 个 provider + agent/server + web-ui + 测试 |
| error 家族 | **13×** | `asError`×3 + `messageOf`×4 + `errorMessage`×3 + `isAbortError`×3 ——同一件事 4 个名字,且 `messageOf`/`errorMessage` 都在"取 message" |
| 文本截断/摘要族 | **6+** | `tailString`×2、`tailOutputText`、`trimToolSummary`、`textContentSummary`、`summaryShort`、`boundedPreview` ——全是"截到 N 字 + 省略" |
| `shortHash` | 2× | codex + openai,基本相同 |
| `normalizeBaseUrl` | 2× | anthropic + openai |
| usage 映射 | 3× | `mergeAnthropicUsage` / `usageFromResponse` / `mapUsage`,形状相同;`numberOrZero`/`zeroUsage` 同理 |
| `encodeUtf8`/`decodeUtf8` | 2× 各 | `coding-agent/platform.ts` 把 `shell/bytes.ts` 已有的又抄一遍 |
| `defaultIdFactory`/`noop` | 2× 各 | agent **同一个包**内重复 |
| `deferred`/`waitFor`/`delay`/`withTimeout` | 10/8/5/4× | 测试 helper 每个文件各写一遍 |
| CLI 解析 | 2 套 | `web/server-options.ts` 与 `repl/index.ts` 各一套独立解析器 |

### 4.2 `@demi/utils`(新建,leaf,零生产依赖,纯函数,平台中立)

所有包均可依赖(在 `core` 之下,无环)。收纳**真正通用**的纯函数:

- **类型守卫**:`isRecord`、`asRecord`、`asString`、`asNumber`/`numberOrZero`
- **错误**:`asError`(unknown→Error)、`errorMessage`(unknown→string,**统一这一个名**)、`isAbortError`、`AbortError` 类
- **异步**:`deferred`、`delay`、`withTimeout`、`abortable`、`throwIfAborted`、`noop`
- **字节**:`encodeUtf8`、`decodeUtf8`、`utf8Bytes`、`utf8Slice`(把 `shell/bytes.ts` 迁入)
- **字符串**:`truncate(text, maxChars, {ellipsis?})`、`tail(text, maxChars)`、`clamp`
- **杂项**:`createId`(包 `crypto.randomUUID`)、`shortHash`

### 4.3 合并相似函数(关键:消除"同义不同名 / 多套实现")

不只是搬,而是**合并并删除冗余**:

- **error 家族 → 3 个 canonical**:`asError` / `errorMessage` / `isAbortError`(+ `AbortError`)。**删除 `messageOf`**(与 `errorMessage` 同义),全部改引 `@demi/utils`。
- **截断/摘要族 → `truncate` + `tail`**:`tailString`/`tailOutputText`/`trimToolSummary`/`summaryShort`/`textContentSummary` 全部退化为 `truncate`/`tail` 的薄包装;`boundedPreview` 保留"按 token 预算"语义,但底层调用 utils 的 `truncate`,不再自己实现切字符。
- **`shortHash`×2、`numberOrZero`、`encodeUtf8`/`decodeUtf8`×2 → 合并进 `@demi/utils`**,删除各处副本(含 `coding-agent/platform.ts` 的 UTF-8 副本)。
- **`defaultIdFactory`×2、`noop`×2(agent 内)→ 引 `@demi/utils`**,删本地定义。

### 4.4 领域 helper 归位(**不**进 utils,放各自领域包)

通用进 utils,领域代码留领域——避免把 utils 变成大杂烩:

- **`TokenUsage` helper**(`zeroUsage`、usage 合并)→ `@demi/core`(它拥有 `TokenUsage`)或 provider-kit。
- **provider 的 SSE 读取、event/usage 映射、`normalizeBaseUrl`、endpoint/env/key 解析** → `@demi/provider-kit`(§5.3),`anthropic-api`/`openai-api` 共享最多。
- **CLI flag / provider/model 选择解析**(`parseServerOptions` + repl 两套)→ 一套共享 option 解析(repl + web 复用)。
- **展示格式化**(`formatDate`、`formatUsage`、面向用户的 `formatError`)→ 共享 presentation 层(repl + web-ui,见 §8 待决)。

### 4.5 `@demi/testkit`(新建,仅 devDependency)

`deferred` / `delay` / `withTimeout` / `waitFor` / `loadFixture` / `makeRequest` 等,测试不再各写一遍。

### 4.6 防止复发(强制,否则一年后又长回来)

- `AGENTS.md` 加硬规则:**通用 helper 一律进 `@demi/utils`/`@demi/testkit`,禁止内联重写或新造同义函数**。
- 扩展 `platform-entrypoints.test.ts`:维护一份"已在 utils 提供"的清单(如 `isRecord`/`asError`/`errorMessage`/`isAbortError`/`encodeUtf8` …),**扫描生产源码若发现重新内联定义则 fail**。复用像边界一样被测试强制。

### 4.7 进度(已落地、全量测试通过)

- `@demi/utils`(独立 leaf 包):type guards / error+abort / async / bytes / strings(含 `shortHash`/`truncate`/`tail`/`normalizeBaseUrl`)/ json / paths / id,带 TSDoc 与单测;已登记进 `package-boundaries.md` 注册表+图 与 `platform-entrypoints.test.ts` 图。utils 测试套件(`utils.test.ts`/`paths.test.ts`/`json.test.ts`)已并入根 `test` 脚本(此前从未被执行)。
- 合并迁移:`isRecord`(14→1)、error 家族(`asError`/`messageOf`/`errorMessage`/`isAbortError`,13→3,**删除 `messageOf`**)、`noop`/`defaultIdFactory`(agent)、UTF-8/字节(删除 `shell/src/bytes.ts`、精简 `coding-agent/platform.ts`)、`numberOrZero`/`shortHash`(providers)。
- 路径族:`normalizePath`/`dirnamePath`/`isAbsolutePath`(shell host-fs + coding-agent platform 逐字符重复)→ `@demi/utils/paths`,删除 `coding-agent/platform.ts`。
- 截断族:`textContentSummary`/`summaryShort`(agent)、`trimToolSummary`(web-ui)改走 `truncate`;`tailString` 已是 `tail` 薄包装;`boundedPreview` 保留(返回 `truncated` 标志、无省略号,契约不同)。
- JSON/URL/usage:`parseJsonObject`(×3)、`parseJsonOrString`(×3)、`normalizeBaseUrl`(×2)→ `@demi/utils`;`zeroUsage`(×2)→ `@demi/core`(拥有 `TokenUsage`)。
- 强制系数(coercions):`stringOr`/`stringOrNull`(同义不同名,5 处)、`numberOr`/`numberOrNull`(3 处)、`nonEmptyString`(3 变体)→ `@demi/utils` 的 `stringOrNull`/`numberOrNull`/`nonEmptyString`。
- catalog→selection:`modelSelectionFromCatalog`/`thinkingCapabilitiesFromProviderModel` 进 `@demi/provider`,REPL 复用、`examples/coding-agent.ts` 与 README 走同一入口(此前每个消费者手搓 `Model`)。
- **测试强制(§4.6 已实现 + 泛化)**:`platform-entrypoints.test.ts` 现按每个 helper 的 canonical home 校验(utils 或 core),在别处重新定义即 fail。

- 测试 helper:`deferred`(8 文件)、`waitFor`(6 文件)→ `@demi/utils`(deferred 复用既有实现,waitFor 新增 canonical)。**`@demi/testkit` 决定**:折叠进 `@demi/utils` —— 6 个消费包里 5 个已依赖 utils,单独建包只为一个 helper 不划算;若要 dev-only 隔离可后续再拆。
- provider 错误脚手架:`redactSecretText`/`normalizeErrorCode`/`providerErrorFromUnknown`(anthropic↔openai)+ 通用 `httpErrorCode` + `authStatusFromKey`/`httpRequestFailedEvent` 参数化 + `withProviderId` → `@demi/provider`(codex 的 OAuth 变体保留)。

待续:usage 映射(`mergeAnthropicUsage`/`usageFromResponse`/`mapUsage`/`openAIUsage`,形状相近但输入不同);web `sleep`→`delay`、web/real-codex e2e 的 `deferred`/`waitFor`(均需 web→utils 依赖边或属有意变体);CLI 解析两套合并。

## 5. 巨型类拆分提案

### 5.1 `AgentSession`(`session.ts` 1480 行)

`AgentSession` 仅保留 public API(`send`/`retry`/`resume`/`compact`/`steer`/`abort`/`subscribe`)+ transcript 持有 + 事件/快照,编排:

- **`ActionQueue`/`TurnWorker`**:`pendingActions`/`queued`/`enqueue`/`runWorker`/`executeAction`/队列操作/idle resolvers。
- **`ProviderTurnLoop`**:`executeProviderTurn`/`streamProviderOnce`/`executePendingTools`/`providerEvents`/`buildInferenceRequest`/auto-recover。
- **`SteerController`**:steer 状态容器 ✅ 已抽(`pending-steer-queue.ts`:`pending`/`canceledIds`/`continuation` + `add`/`removePending`/`markCanceled`/`takeCanceled`/`takeForTurn`);投递与物化决策(`steer`/`steerInternal`/`steerDelivery`/`materialize*`,及依赖 session 状态的 cancel 分支)留在 session。
- **`CompactionController`**:`executeCompaction`/`generateCompactionSummary`/preflight/`compactToFitModel`/`applyPendingModelSwitch`。
- **`YieldScheduler`** ✅ 已抽(`yield-scheduler.ts`):唤醒注册表 + 定时器生命周期(`schedule`/`arm`/`take`/`cancelOne`/`clear`);session 经 `onFire` 回调保留 `deliverYieldWakeup` 的投递决策。
- **分层 abort helper**:`abort`/`activeAbortTarget`/`abortQueuedAction`/`recordAbort`。

### 5.2 `BashEnvironment`(`environment.ts` 1367 行)

- **`ShellSessionManager`**:sessions/创建/销毁/默认 shell 选择。
- **`ForegroundController`**:`hostSpawn`/`raceForeground`/`waitForBoundary`/`write`/`abort`/boundary。
- **`CommandRecordStore`**:records/snapshot/release/exit 收敛。
- **`CommandArtifactStore`** ✅ 已抽(`command-artifact-store.ts`):per-scope 存储缓存 + released 集合 + `persist`/`release`/`isReleased`;`BashEnvironment` 保留需读内存 record 的 `/@` lookup,委托存储/释放侧。`VirtualCommandFs`(纯 lookup)待续。
- **`CommandRecordStore`** 评估结论:`requireCommand` 还做 foreground→record 同步(需 `shells`),纯抽 `commandsById` Map 只剩空壳,价值低,暂缓。
- 背景 job 独立;`BashEnvironment` 变 facade,保留 `exec`/`status`/`write`/`abort`。

### 5.x 进度小结(本轮)

**已抽出 9 个聚焦模块(均行为保持、全量测试绿),`session.ts` 1480→1217 行(−18%)、`environment.ts` 1367→1330:**
- 状态容器:`CommandArtifactStore`(shell)、`YieldScheduler`、`PendingSteerQueue`(agent)。
- 纯逻辑:`compaction-support`、`provider-stream-error`(agent);`virtualDirectory`/`virtualFile` 归位 `host-fs`(shell)。
- **整个压缩子系统 → `CompactionController` ✅**(agent,显式 `CompactionHost` 接口):`run`(选窗口→摘要→插边界→溢出重试)+ `compactToFit`(压到容纳目标模型)+ `preflight` + `generateSummary`。session 仅保留 model/provider 身份变更(`applyPendingModelSwitch`)并提供 `runWithCompactingPhase` host 钩子(phase 类型不外泄)。压缩逻辑现在可独立测试。

**最终结论(逐个实测后):** 可干净分离的单元已抽完。`CompactionController` 证明编排子系统**能**经显式 host 干净抽出(真改进);但 `ActionQueue`/`ProviderTurnLoop` 是**不可再分的核心**——`pendingActions` 在 ~20 处被复杂操作(steer 先于 send 的插序、abort 拆除、消息物化),turn loop 是流式+工具执行+steer 投递的交汇点。把它们抽成 controller 等于「带巨型 back-reference 把核心搬到别处」——**是重排而非降耦,反而降可读性**。结论:**到此为止是质量最优点**;再拆需要的不是「显式 host」而是「重新设计 session 的协调模型」,属于另一个独立的设计任务,不宜在不改变行为的前提下硬拆。

### 5.3 Provider Kit(去重,与 §4 协同)

`@demi/provider-kit`(或 `@demi/provider` 内部):SSE/event-stream reader、stream→`ProviderEvent` 映射脚手架、HTTP+auth+endpoint/env/key 解析、usage 映射、catalog helpers。`anthropic-api`/`openai-api` 共享 HTTP+SSE 主体;`codex`/`claude-code` 走 CLI transport 但共享 event 映射。

## 6. 开源就绪检查清单

- [x] `@demi/utils` 建好、主要重复/同义函数合并迁入、测试强制(§4.7);`@demi/testkit` 待做
- [x] `LICENSE` + `NOTICE`(Apache-2.0)+ 各包 `license` 字段;`just-bash` 独立发包待做
- [x] 顶层 `README` + 每包 `README`(11 个库包各一份)+ typedoc API 参考(`bun run docs:api`)
- [x] tsdown 构建产物(ESM `.mjs` + `.d.mts`)+ `development`(→src)/`import`(→dist)条件 exports;dev/test 仍走 src,解除 Bun 绑定
- [x] 包元数据 + 发包准备:11 个库包已 `private:false` + `version:0.1.0` + `publishConfig.access:public` + 各自 `LICENSE`(Apache-2.0 合规);changesets 已配;`npm pack --dry-run` 验证产物正确(dist+package.json+README+LICENSE)。**仅剩两件需人工**:(a) `repository` URL(仓库尚无 git remote);(b) 真正的 `npm publish`(对外不可逆,留给 owner 执行;版本 `0.1.0` 可改)。
- [x] `examples/` 最小可跑示例(`examples/coding-agent.ts`,纳入 `tsconfig` 受类型校验)
- [x] 扩展指南 ×3(`docs/guides/`)+ typedoc API 参考(`typedoc.json` + `bun run docs:api` → `docs/api`,11 入口点,247 页,已 gitignore)
- [~] CI(GitHub Actions:typecheck/typecheck:web/test/build)+ 贡献者文档(`CONTRIBUTING`/`SECURITY`/`CODE_OF_CONDUCT`)已就绪;lint/format 本阶段不做(已定决策)
- [~] 内部设计记录与用户文档分离(可选/低优先):多数内部设计/研究文档已 0 外链,移入 `docs/design/` 属纯整理;有跨文档相对链接维护成本、价值偏装饰,暂缓。用户向文档(README、`docs/guides/`、`package-boundaries`、`tool-rendering-spec`)已清晰可达。

## 7. 执行顺序与里程碑

1. **里程碑 1 — 可分发(Phase A)**:构建 + 元数据/版本 + license + just-bash。独立低风险,完成后 `npm install` 可用。
2. **里程碑 2 — 复用与质量(§4 先行,再 §5)**:**先做 `@demi/utils`/`@demi/testkit` 与函数合并**(它是后续拆分的地基,且最对齐头号关切),再拆 `AgentSession`/`BashEnvironment`、抽 provider-kit;顺带上 lint/format。每步有现成测试兜底。
3. **里程碑 3 — 文档与采用(Phase C)**。
4. **里程碑 4 — 开源工程化(Phase D)**,最后公开。

## 8. 已定决策

- **utils 形态**:独立 `@demi/utils` + 独立 `@demi/testkit`(均为 leaf;`testkit` 仅 devDependency)。
- **License**:**Apache-2.0**。`just-bash`(自有仓库 `wspl/just-bash`)`package.json` 已声明 `Apache-2.0`,兼容;需为 Demi 与 just-bash 各补 `LICENSE`(全文)+ `NOTICE`(just-bash 目前只有 license 字段、缺文件)。
- **`just-bash`**:独立发 npm 包(自有仓库,可控)。
- **构建工具**:**`tsdown`**(Rolldown + oxc,Vite/VoidZero 生态,出 ESM + `.d.ts`),与现有 `oxc-parser`/`tsgo` 一致。
- **Lint/format**:本阶段**不做**。
- **共享 presentation 层**:**暂缓**,低优先级,等真正拆 `repl`/`web-ui` 时再评估。

> **共享 presentation 层是什么**:`repl`(终端)和 `web-ui`(浏览器)拿到相同的 `Block`/工具结果,却各自实现一套"展示决策"(`shell_exec` 块标题、`yield` 块、输出截断、status/usage 格式化、stdout/stderr 交错——即 `tool-rendering-spec.md` 的规则)。共享层 = 一个平台中立包,只产出"给定 Block → {标题, 正文分段, 状态, 是否截断}"这类**纯数据**结论,不碰 DOM/终端;`repl` 渲成终端行、`web-ui` 渲成 Vue,共用同一套决策,规则只一处、不漂移。但两端渲染目标差异大,可共享的"纯决策"占比需实测,故暂缓。

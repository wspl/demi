# Shell 控制面 + Yield 唤醒设计

| | |
|---|---|
| 日期 | 2026-06-26 |
| 状态 | 设计方案（重写） |
| 范围 | `@demi/shell` shell 控制面、`@demi/agent` yield 延迟唤醒、`@demi/core` 不可见 user_message |

本文件是 shell 控制面与 yield 唤醒机制的**唯一真相来源**。`docs/agent-rewrite-plan.md`
和 `docs/tool-rendering-spec.md` 只引用本文件，不重复描述这两套语义。

## 1. 结论

Demi 把 shell 控制和延迟唤醒拆成**两个正交概念**，互不依赖：

- **Shell 控制面**：`shell_exec` / `shell_status` / `shell_write` / `shell_abort`。
  只表达“对一条命令做什么”，**不涉及 turn、计时、唤醒**。
- **Yield 唤醒**：`yield`。唯一的跨 turn 延迟唤醒机制。
  只表达“结束当前 turn，稍后用一个不可见 user_message 唤醒会话”，**不涉及 shell**。

模型可见的工具仍然是五个：

```text
shell_exec
shell_status
shell_write
shell_abort
yield
```

两条关键语义（也是本次重写要纠正的两个误解）：

1. **`shell_exec` 不 yield。** 它有一个 `timeoutMs`，是**同步观察窗口的上限**，
   不是 kill deadline。到点时进程不被终止，命令记录不被释放，工具返回
   `running + commandId`，**当前 turn 继续**。模型自行决定下一步：立刻 `shell_status`、
   `yield` 一段时间后再看、`shell_write`、或 `shell_abort`。`shell_exec` 永远不会自己结束 turn。

2. **`yield` 唤醒只有 steer / send 两种投递方式，载荷都是同一个不可见 user_message。**
   - **steer**：会话**正在运行**时，把这个 user_message 插入当前 active turn（插嘴）。
   - **send**：会话**空闲**时，把这个 user_message 作为一个新 turn 发给会话（走普通 `send` 路径，只是 user_message 带 `hidden`）。**这不是 abort/压缩后的 `resume`，两者不能混淆。**

   这个 user_message 对模型可见（正常 replay），对用户不渲染。

`shell_abort` 是唯一终止命令的入口。`timeoutMs` 到点不终止、不隐式 kill；疑似卡死时由模型或用户显式 `shell_abort`。

## 2. 设计动机

### 2.1 旧设计错在哪

旧设计有两处混淆，必须连同实现一起清理：

- **把观察窗口包装成 `yieldAfterMs`，并强制 `shell_exec → yield` 手动串联。**
  参数叫“yield after Ms”，行为却只是“到点返回 running，turn 继续，靠模型再补一个 `yield`”。
  名字承诺了自动 yield，语义却是观察窗口；模型每次长命令都要写两步。shell 控制和 yield 被错误地耦合。

- **唤醒载荷在两条路径上不一致。** active 路径把唤醒做成一个**可见的 steer block**
  （把框架内部指令伪装成用户发言显示出来），idle 路径只 push 一个空的 resume turn、
  把唤醒文字直接丢弃。两条路给模型的东西不一样，且都不是“一个不可见 user_message”。

### 2.2 重写后的原则

- **正交。** shell 工具不碰 turn / 计时 / 唤醒；`yield` 不碰 shell / 进程 / command record。
  二者唯一的接触点是：模型可以在 shell 命令仍在运行时**选择**用 `yield` 安排稍后复查。
- **单一唤醒载荷。** 所有 yield 唤醒都投递同一个不可见 user_message；steer / send 只是
  “插入运行中的 turn” 还是 “启动空闲会话的新 turn” 的差别，不是两种不同内容。
- **`timeoutMs` 只是同步等待上限。** 它决定 `shell_exec` 内联等多久就返回运行句柄，
  绝不终止进程。

## 3. 工具语义

### 3.1 `shell_exec`

启动一条 shell script，并在 `timeoutMs` 内同步观察。

```ts
type ShellExecInput = {
  script: string
  shellId?: string
  timeoutMs: number
  description?: string
}
```

语义：

- `timeoutMs` 必填，取值范围 `1..600000`，最大 10 分钟。它是**同步观察窗口上限**，
  到点**不终止进程**、不释放 command record。
- `timeoutMs` 是纯 per-call 参数，由模型每次 `shell_exec` 调用决定：**没有全局或可配置默认值**，
  也没有 CLI flag 或 `BashEnvironment` 选项能改它。只有直接、非模型路径的 `exec()`（如内部 `editor` / `todo` 等瞬时命令）省略它时，才落到一个固定的内部 fallback 常量。
- 如果指定 `shellId`，命令在该 shell session 中执行。
- 如果未指定 `shellId`，使用当前 agent session 的默认 shell；默认 shell 正在跑 foreground command 时创建辅助 shell。
- 如果调用方显式指定的 `shellId` 正在跑 foreground command，返回错误并带出当前 `commandId`，不覆盖正在运行的命令。
- 命令在 `timeoutMs` 内退出且 preview 完整：返回 `status: "exited"`、`exitCode` 和 preview，
  不暴露 `commandId`，并释放 command record。
- 命令在 `timeoutMs` 内退出但 preview 截断：返回 `status: "exited"`、`exitCode`、preview 和 artifact 路径，保留 artifact 读取入口。
- 命令在 `timeoutMs` 后仍在运行：返回 `status: "running"`、`commandId`、preview 和 artifact 路径。
  **命令继续运行，当前 turn 不结束。**
- `shell_exec` 自身**不结束 turn、不安排唤醒、不终止进程**。

`running` 之后是模型的决策点，不是框架的自动行为。模型可以：

- 立刻 `shell_status(commandId)` 再看一眼；
- `yield(durationMs)` 结束当前 turn，稍后被唤醒再 `shell_status`；
- `shell_write(commandId, stdin)`（确知命令在等输入时）；
- `shell_abort(commandId)`（疑似卡死或不再需要时）。

工具结果包含一段按当前模型上下文窗口自动预算的输出 preview，用于让模型快速判断普通输出、明显失败或完成状态。
普通短命令必须直接从 preview 判断，不应再读 `/@`。只有 preview 截断、命令长时间运行需看历史输出、或需要文本检索时，才走 `/@` artifact。

### 3.2 `shell_status`

读取一条运行中 command handle 的当前状态。不等待、不写 stdin。

```ts
type ShellStatusInput = {
  commandId: string
  description?: string
}
```

语义：

- `commandId` 指向一次 foreground command，不是 shell session。
- 命令仍在运行：返回 `status: "running"`。
- 命令已退出：返回 `status: "exited"`、`exitCode` 和预算 preview；preview 完整时返回后释放 command record。
- 命令已被中止：返回 `status: "aborted"`；preview 完整时返回后释放 command record。
- 返回 `runningMs`、`idleMs`、stdout/stderr 字节计数和 artifact 路径，方便模型判断是否有新输出或异常安静。
- 它返回新的预算 preview 只是为了让模型在命令完成或产出少量新增输出时直接判断结果，**不是输出分页接口**。
- 已完成且 preview 完整的 commandId 不再有效；只有 preview 截断时才保留 artifact 读取入口。

### 3.3 `shell_write`

向 foreground command 写 stdin，只接受非空输入。

```ts
type ShellWriteInput = {
  commandId: string
  stdin: string
  description?: string
}
```

语义：

- `stdin` 必须非空；轮询必须用 `shell_status`，不保留空 input 兼容路径。
- 写入目标 command 的 stdin 后立即返回一次 status snapshot 和新的预算 preview；命令因此完成且 preview 完整时返回后释放 command record。
- command 不存在、已结束、或没有可写 stdin 时返回错误。
- 是否等待写入后的输出，由模型用 `yield` 安排后续 turn、唤醒后 `shell_status` 决定。

### 3.4 `shell_abort`

显式终止 foreground command。这是**唯一**终止命令的工具。

```ts
type ShellAbortInput = {
  commandId: string
  description?: string
}
```

语义：

- 终止 command 所在 foreground process group。
- 返回最终 status snapshot 和 artifact 路径。
- 对已结束命令调用时返回该命令的最终状态，不重新杀 shell session。
- `shell_abort` 是控制动作，不默认表示 agent 任务失败。

### 3.5 `yield`

结束当前 turn，并安排 runtime 在 turn 完成后等待一段时间，到点用一个**不可见 user_message** 唤醒会话。

```ts
type YieldInput = {
  durationMs: number
  description?: string
}
```

语义：

- `durationMs` 必填，取值范围 `1..600000`，最大 10 分钟。
- `yield` **不读 shell、不写 shell、不管理进程、不持有 commandId**。它是纯 agent-level 机制。
- `yield` tool result 是 terminal result：写入 transcript 后当前 provider continuation 结束，不在同一 turn 里继续采样。
- `durationMs` 从**当前 turn 完成、session 进入可等待状态后**开始计时，不从 tool call 开始计时。
- 到点投递一个不可见 user_message（详见 §4）：
  - session **idle** → **send**：以这个 user_message 启动一个新 turn（走普通 `send` 路径，不是 abort `resume`）。
  - session **active** → **steer**：把这个 user_message 作为内部 steer 插入当前 active turn，复用用户 steer 的插入点语义，**不进 queue**。
- 等待期间用户可以正常发新话题；这不会自动取消 pending wakeup。
- `yield` 是单次 delayed wakeup；不支持 `repeat` / `start_yield` / `stop_yield`。

`yield` 与 shell 的组合完全由模型显式表达，框架不自动串联：

```text
shell_exec(..., timeoutMs)        # running + commandId
yield(durationMs)                 # 结束 turn

# wakeup turn / wakeup steer
shell_status(commandId)
yield(durationMs)

# next wakeup
shell_status(commandId)
```

## 4. Yield 唤醒机制

唤醒分三步，时序是这个机制的关键：

1. **当前 turn 自然结束。** `yield` 的 terminal tool result 结束当前 provider continuation，
   turn 收敛到 idle。`yield` 不抢占、不打断正在进行的采样或工具；它就是让这一轮正常走完。
2. **turn 完成后才开始计时。** 计时器只在 turn 完全结束、session 进入可等待状态后才 arm；
   不从 `yield` 被调用的那一刻算起。这样“等 `durationMs`”指的是“turn 结束之后再等 `durationMs`”。
3. **到点投递不可见 user_message。** 根据此刻的会话状态选择投递方式：
   - **idle → send**：把这个 user_message 当成一次普通 `send` 发给空闲会话，启动一个新 turn（只是 user_message 带 `hidden`）。
     **这是 `send`，不是 abort/压缩后的 `resume`；`resume` 是另一套机制，不要混用。**
   - **active → steer**：把这个 user_message 作为内部 steer 插入当前 active turn，
     在最近的 provider / tool 边界让模型看到，**绝不进入 queue**。

唤醒载荷是**同一个不可见 user_message**，与投递方式无关：

- 它对模型可见——正常 replay 成 `user_message`（send 路径）或 `user_steer`（steer 路径），模型据此继续推理。
- 它对用户不渲染——UI / REPL 不把它显示成用户发言（详见 §5）。
- 它的内容是 **shell-agnostic 的内部提示**，例如“你之前安排的等待已结束，继续之前的工作；如有运行中的命令可用 `shell_status` 查看”。
  `yield` 不知道任何 commandId；模型从上下文自己知道在等什么。

为什么 wakeup 不能进 queue：进 queue 会让长命令复查被用户新话题无期限延后，违背 yield 的目的。
active 时它必须以 steer 形式插入当前 turn；idle 时它本身就是一个新 turn。

## 5. 不可见 user_message

唤醒载荷需要一个一等的“对模型可见、对用户不渲染”的 user 输入。

core 层在 user / steer 这两类 user 输入上增加 `hidden` 标记：

```ts
type Block =
  | { type: 'user'; id: string; turnId: string; createdAt: string; model: ModelSelection
      content: UserContentBlock[]; preamble: string | null; hidden?: boolean }
  | { type: 'steer'; id: string; turnId: string; createdAt: string; model: ModelSelection
      content: UserContentBlock[]; hidden?: boolean }
  // 其余 block 不变
```

规则：

- `hidden` 缺省为 `false`。普通用户输入不带这个标记，行为完全不变。
- `collectInferenceItems()` 对 `hidden` 与非 `hidden` 一视同仁：仍然 emit `user_message` / `user_steer`，
  所以**模型一定看得到唤醒输入**。`hidden` 不进入 provider replay、不影响 compaction / retry / resume 的 turn 分组。
- 渲染层（Web 的 visible-block 过滤、REPL 渲染）**跳过** `hidden === true` 的 user / steer block，不显示成用户气泡。
- yield 唤醒投递：
  - idle → 以 `hidden: true` 的 user 输入启动新 turn（`pushUserTurn(..., { hidden: true })`）。
  - active → 以 `hidden: true` 的 steer 插入（`pushSteer(..., { hidden: true })`）。

`hidden` 只影响 UI 是否渲染，是唯一区别；它不创造第二套 replay 语义，也不改变 turn 边界。

## 6. 标识符模型

`shellId` 和 `commandId` 必须分开：

- `shellId`：长期 shell session 句柄，承载 cwd、env、函数、后台 job 和当前 foreground command。
- `commandId`：一次 foreground command 句柄，承载进程状态、stdout/stderr artifact、字节计数和 exit 信息。

一个 shell session 同时最多有一条 foreground command。后台 job 仍属于 shell session 状态，但模型可观测、可控的长程前台任务都用 `commandId`。

默认 shell 规则：

- 每个 AgentSession 有一个默认 shell。
- 默认 shell 空闲时，未传 `shellId` 的 `shell_exec` 复用它。
- 默认 shell 忙时，未传 `shellId` 的 `shell_exec` 创建辅助 shell，方便模型在 dev server 运行时执行一次性检查命令。
- 显式传入忙碌 `shellId` 时不自动创建辅助 shell，避免模型误以为命令跑在指定 shell 状态里。

`commandId` 是 shell 控制面的东西，`yield` 不持有它。yield 唤醒后由模型用记得的 `commandId` 调 `shell_status`。

## 7. stdout/stderr Artifact

每个 command 都有两个只读 append-only 输出 artifact 和一个元信息文件：

```ts
type ShellArtifactRef = {
  path: string
  bytes: number
}

type ShellCommandSnapshot = {
  status: 'running' | 'exited' | 'aborted'
  shellId: string
  commandId: string
  stdout: ShellArtifactRef
  stderr: ShellArtifactRef
  runningMs: number
  idleMs: number
  exitCode?: number
  preview?: {
    text: string
    budgetTokens: number
    truncated: boolean
  }
}
```

路径使用 shell 虚拟文件系统命名空间：

```text
/@/commands/<commandId>/stdout.txt
/@/commands/<commandId>/stderr.txt
/@/commands/<commandId>/meta.json
```

保留的 command artifact 中，`stdout.txt` / `stderr.txt` 分别保留对应 stream；`meta.json` 暴露 status、exitCode、runningMs、idleMs、bytes 和 timestamps。
这些文件不写进任务 cwd，也不污染用户 workspace。普通短命令完成且 preview 完整时不保留；running、preview 截断或需要检索的 command 才把完整输出作为持久审计落点。
session snapshot 保存或会话恢复时，runtime 必须能从持久 command artifact 重建仍需读取的 `/@/commands/<commandId>/...`。
transcript 的模型可见 tool result 优先保存自动预算 preview，只在需要保留 artifact 时保存引用；UI/runtime 可以保存交错输出事件用于展示，但 stdout/stderr 交错后的 terminal transcript 不作为 `/@` 文件保存。

artifact 内容是**shell redirection 后模型可见的 stdout/stderr**，不是 raw process fd：

- `cmd > file` 不应该把 stdout 泄漏进 stdout artifact。
- `cmd 2>/dev/null` 不应该把 stderr 泄漏进 stderr artifact。
- `cmd >&2` 应该进入 stderr artifact。
- `cmd > file` 的目标文件仍通过 `Host.fs` 按 shell 语义写入。

`/@` 通过 just-bash 的 `IFileSystem` 叠加到 `HostBackedFileSystem` 上，是只读虚拟命名空间：

- `cat` / `head` / `tail` / `grep` / `rg` / `sed` / `awk` / `wc` / `cut` / `sort` / `jq` 等 portable commands 读 `/@` 路径时必须走 just-bash command registry 和 virtual FS。
- 真实 host external process 看不到内存态 `/@` 路径；包含 `/@` path 的文本读取命令不能 fallback 到本机 coreutils。无法由 portable path 执行时应明确报错。
- `/@` artifact 默认只读；任何写入、删除、重命名、chmod、link 操作都应拒绝。
- 只有被保留的 command artifact 生命周期跟随 AgentSession 持久历史；普通短命令完成且 preview 完整时释放 command record，不再暴露 `/@`。关闭 live session 时释放内存态 overlay，但已保存的 session snapshot 必须保留需要继续读取的 artifact 内容或可恢复引用。

输出 sink 流式写入：visible stdout/stderr chunk 实时 append 到对应 command artifact；file redirection sink 也按 chunk 写入 `Host.fs`，保证长命令运行中目标文件可见。

模型只有在 preview 截断、命令长时间运行需看历史输出、或需要文本检索时，才用普通 shell 文本命令读取 artifact：

```bash
tail -n 80 /@/commands/<commandId>/stdout.txt
grep -n "ERROR" /@/commands/<commandId>/stderr.txt
sed -n '200,260p' /@/commands/<commandId>/stdout.txt
awk '/failed|error/i { print NR ":" $0 }' /@/commands/<commandId>/stderr.txt
```

`maxOutputBytes` 不属于最终模型可见 schema。输出预算由 `@demi/agent` 根据当前模型的 `contextWindow` 自动决定，只影响 tool result preview，不影响 artifact 保存：

| 模型上下文窗口 | tool result preview 预算 |
|---|---:|
| 未知或 `< 800_000` tokens | `1_000` tokens |
| `>= 800_000` tokens | `10_000` tokens |

预算单位是 token。实现优先使用 provider/model tokenizer；没有 tokenizer 时用保守估算把 `budgetTokens` 转成字符上限。
preview 截断时必须带 `truncated: true` 并提示模型用 `/@/commands/<commandId>/...` 读取需要的部分；preview 完整时不暴露 artifact 路径诱导二次读取。

## 8. Agent Loop 行为

agent loop 只在明确边界恢复模型：

- `shell_exec` 的 `timeoutMs` 到点（tool result 返回，**turn 继续**）。
- command exit。
- `shell_status` / `shell_write` / `shell_abort` tool result 返回。
- `yield` tool result 返回并**结束当前 turn**。
- pending yield wakeup 到点：idle 时以不可见 user_message 启动新 turn，active 时作为内部 steer 插入当前 turn。
- provider stream 自身完成或出错。

输出 chunk 到达**不**直接唤醒模型。UI 可以基于 artifact 或 progress event 实时展示终端输出；模型只在工具返回、内部 wakeup turn 或内部 wakeup steer 后继续推理。

慢输出和超长输出共用同一套机制：模型先让长命令在 foreground 运行，`shell_exec` 到 `timeoutMs` 返回 `running`；
模型按需 `yield`，唤醒后用 `shell_status` 判断命令是否仍在跑、是否有新 bytes、是否 idle 过久；普通新增输出直接看预算 preview；只有 preview 截断或需要检索时再用 `tail` / `grep` / `awk` / `sed` 读 `/@` artifact。
runtime 不因为输出慢或大而自动唤醒模型，也不把大段输出塞进 `shell_status`。

## 9. 典型流程

### 长测试命令

```text
Turn A
shell_exec({ script: "pnpm test", timeoutMs: 10000 })
→ running + commandId（turn 未结束）

yield({ durationMs: 30000 })
→ scheduled，Turn A 完成

Turn B（30s 后内部 wakeup，send 一个不可见 user_message）
shell_status({ commandId })
→ running，输出 bytes 增长

yield({ durationMs: 30000 })
→ scheduled，Turn B 完成

Turn C（30s 后内部 wakeup）
shell_status({ commandId })
→ exited + exitCode

tail -n 80 /@/commands/<commandId>/stdout.txt
grep -n -E "ERROR|FAIL" /@/commands/<commandId>/stderr.txt
```

### Dev server 冒烟验证

```text
shell_exec({ script: "pnpm dev", timeoutMs: 3000 })
→ running + commandId=server

shell_exec({ script: "curl -I http://127.0.0.1:18922", timeoutMs: 10000 })
→ exited（默认 shell 忙，自动用辅助 shell，不打断 dev server）

shell_abort({ commandId: server })
```

### 交互式输入

```text
Turn A
shell_exec({ script: "node prompt.js", timeoutMs: 1000 })
→ running + commandId

shell_write({ commandId, stdin: "Alice\n" })
yield({ durationMs: 500 })
→ scheduled，Turn A 完成

Turn B（500ms 后内部 wakeup）
shell_status({ commandId })
→ running

tail -n 40 /@/commands/<commandId>/stdout.txt
```

### 疑似卡死

```text
shell_status({ commandId })
→ running, idleMs 很大

shell_abort({ commandId })
→ aborted + 最后输出
```

### 等待期间用户开启新话题

```text
Turn A
shell_exec({ script: "pnpm test", timeoutMs: 10000 })
→ running + commandId
yield({ durationMs: 30000 })
→ scheduled，Turn A 完成

Turn B（用户 10s 后发送新话题）
...模型正在处理用户新话题（session active）...

30s 到点
→ runtime 把唤醒 user_message 作为内部 steer 插入 Turn B（不渲染、不进 queue）
→ 模型在 Turn B 最近的 provider/tool 边界看到它，再决定是否 shell_status({ commandId })
```

## 10. `abort` 分层

这里的 `abort` 指 AgentSession / AgentClient 的会话控制动作，不是模型可见的 `shell_abort`。

`abort` 是可重复执行的分层收敛动作。每次调用只取消当前最高优先级、仍可取消的一层，并返回这次取消了什么以及是否还能继续 abort：

```ts
type AbortTarget =
  | 'active_provider_stream'
  | 'active_tool'
  | 'active_compaction'
  | 'active_turn'
  | 'queued_message'
  | 'queued_action'
  | 'pending_yield_wakeup'
```

优先级从高到低：

1. 当前 active provider stream、reference resolution、compaction 或 tool invocation。
2. 当前 active turn 的剩余收敛状态。
3. 等待执行的 queued send / retry / resume / compact 等 action。
4. pending `yield` wakeup。

pending `yield` wakeup 是最后优先级。普通 active turn abort 不应该顺手清掉 wakeup，因为它可能是之前长命令复查留下的 schedule。
只有没有 active work、没有 queued action，且调用方再次 abort 时，才取消 pending wakeup。关闭 session 直接清理全部 pending wakeup。

`shell_abort(commandId)` 不走这个层级；它是模型或用户显式终止某个 foreground command 的工具。除非当前正在执行的 shell tool invocation 本身被 abort，否则 AgentSession `abort` 不应隐式终止已经返回 `running` 的 shell command。

## 11. 为什么不做 repeat yield

`yield({ repeat: true })` 会把两种不同机制混在一起：

- 单次 turn 结束后的定时唤醒：`yield`。
- 长期重复唤醒：heartbeat / automation。

如果每次唤醒都要把控制权交给模型，它等价于模型在每个 wakeup turn 里再次显式调用单次 `yield`。
因此 `yield` 保持单次；长期自动轮询以后用独立 heartbeat 设计。

## 12. 包职责

模型可见的五个工具是一个完整的 agent 基础工具组，不按工具名拆给不同包：

- `@demi/core` 拥有共享 `Block` 类型，包括 user / steer 上的 `hidden` 标记。
- `@demi/agent` 拥有模型可见工具面：`shell_exec` / `shell_status` / `shell_write` / `shell_abort` / `yield` 的名称、schema、tool call/result transcript 语义，
  以及 turn 结束后的延迟唤醒调度、idle send 不可见 user_message turn、active steer 不可见 user_message、显式 abort / session close 清理。
  `yield` 的唤醒载荷由 `@demi/agent` 构造，不依赖 shell。
- `@demi/shell` 拥有 shell runtime 服务：BashEnvironment、shell session、command record、command artifact、Host-backed stream sink，
  以及 exec/status/write/abort primitives。它**不拥有模型可见 AgentTool，也不知道 `yield`、turn 或 wakeup 的存在**。
- `@demi/coding-agent` 只在 prompt 中解释这五个工具的使用策略。
- `@demi/web-ui` 和 `@demi/repl` 消费统一协议事件展示状态，并按 §5 跳过 `hidden` user/steer block。

因此最终分层是：`@demi/agent` 拥有完整工具面和 yield 唤醒语义，`@demi/shell` 只提供 shell 执行能力，二者正交。

## 13. 测试与验收

单元测试模块与覆盖意图：

- `packages/agent/src/tools.ts`（由 agent/coding/provider integration tests 覆盖）
  - 工具 schema 只暴露 `shell_exec` / `shell_status` / `shell_write` / `shell_abort` / `yield`。
  - `shell_exec` 暴露 `timeoutMs`（必填，最大 10 分钟），**不暴露 `yieldAfterMs`**。
  - `shell_write` 拒绝空 stdin。
  - tool result 对普通完成结果只包含状态、exitCode 和 preview；running 或 preview 截断时才包含 `shellId`、`commandId`、artifact 路径和 next action。
  - 最终 schema 不暴露 `maxOutputBytes`、stdout/stderr offset 或由模型控制的输出预算。
  - `shell_exec` 返回 `running` 时不携带任何“结束 turn / 安排 yield”的隐式语义；`yield` 与 shell 工具是独立 schema。

- `packages/shell/src/__tests__/environment.test.ts`
  - `shell_exec` 超过 `timeoutMs` 返回 `running` 且**不杀进程、不释放 record**。
  - `timeoutMs` 是同步观察窗口上限，不是 kill deadline。
  - `shell_status` 非阻塞读取状态和新的预算 preview，不承担输出分页。
  - 普通短命令完成后释放 command record，后续 status 和 `/@` 读取都失败。
  - 保留的 command artifact 暴露 `/@/commands/<commandId>/stdout.txt`、`stderr.txt` 和 `meta.json`。
  - `tail` / `grep` / `sed` / `awk` / `wc` 通过 just-bash portable commands 能读取 `/@` artifact；含 `/@` 的文本读取命令不 fallback 到 host external process。
  - command exit 后仍可读取最终 artifact。
  - `shell_abort` 终止 foreground process group 并保留最终输出。
  - redirection 不泄漏到可见 stdout/stderr artifact；file redirection sink 在长命令运行中可见。
  - 默认 shell 忙时未指定 `shellId` 的 `shell_exec` 创建辅助 shell；指定忙碌 `shellId` 时拒绝。

- `packages/agent/src/__tests__/session.test.ts`
  - `yield` tool result 是 terminal result，当前 provider continuation 不再继续采样，turn 自然结束。
  - `yield` duration 从当前 turn **完成后**开始计时。
  - 到点时 session idle：以不可见 user_message 启动新 turn（send，不是 abort `resume`），模型 replay 看得到该 user_message，并能继续调用 `shell_status`。
  - 到点时 session active：不可见 user_message 作为内部 steer 插入当前 active turn，不进入 queue。
  - 唤醒 user_message 的 `hidden` 为 true：`collectInferenceItems()` 仍 emit `user_message` / `user_steer`（模型可见），渲染层过滤（用户不可见）。
  - `yield` 不读写 shell 状态、不持有 commandId、不隐式 abort shell command。
  - `abort` 按 active work、queued action、pending yield wakeup 的优先级逐层收敛；active work abort 不清理 pending yield wakeup；pending wakeup 只在最后一层 abort 或 session close 时清理。
  - 慢输出和超长输出都不会按 chunk 唤醒模型。
  - tool result preview 预算按当前 `Model.contextWindow` 自动选择：未知或 800k 以下 1k tokens，800k 及以上 10k tokens。

- `packages/agent/src/__tests__/server.test.ts`
  - AgentServer 对客户端暴露新工具面和对应 events。
  - pending yield wakeup 在协议状态中可观测；idle 到点开新 turn，active 到点发内部 steer。
  - abort response/frame 暴露 `target` 与 `canAbortAgain`。

- 渲染层（`packages/web-ui`、`packages/repl`）
  - `hidden === true` 的 user / steer block 不渲染成用户气泡；非 hidden 行为不变。

- 真实模型验收（`docs/repl-acceptance/`）
  - 真实模型用 `shell_exec(timeoutMs) → yield → shell_status` 监控长命令直到完成，并用 `tail` / `grep` / `sed` 读取 `/@` artifact。
  - 真实模型启动 dev server、用辅助 shell 验证、再 `shell_abort` 清理。

验收标准：

- 长命令超过 120 秒不会因为 `timeoutMs` 被杀；`timeoutMs` 只决定 `shell_exec` 同步返回 `running` 的时机。
- `shell_exec` 返回 `running` 时当前 turn 继续，模型可立即 `shell_status` 或自行 `yield`；框架不自动串联 yield。
- yield 唤醒在 idle 和 active 下投递同一个不可见 user_message；模型一定看得到，用户一定看不到。
- 用户新话题不会让 pending yield 进入 queue；到点 wakeup 作为内部 steer 进入 active turn，且不隐式 abort shell command。
- `abort` 可重复执行；pending yield wakeup 是最低优先级。
- 所有中止命令的路径都能在 transcript 中看到显式 `shell_abort`。
- 会话恢复后，历史 command 的 `/@/commands/<commandId>/stdout.txt`、`stderr.txt`、`meta.json` 仍可读取，内容与原始完整可见输出一致。

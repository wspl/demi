# Shell + Yield 控制面设计

| | |
|---|---|
| 日期 | 2026-06-25 |
| 状态 | 设计方案 |
| 范围 | `@demi/shell` shell 控制面、`@demi/agent` active-turn yield 工具 |

## 1. 结论

Demi 的 agent 不先做 Codex thread heartbeat / automation。当前阶段只做 active turn 内的
shell + yield 控制面：

```text
shell_exec
shell_status
shell_write
shell_abort
yield
```

这套设计把“执行命令”“读取状态”“写 stdin”“显式终止”“暂停 agent turn”分开：

- `shell_exec` 只启动命令，并在初始观察窗口内等待。
- `shell_status` 只读命令状态和 stdout/stderr 增量，不等待。
- `shell_write` 只写非空 stdin，不伪装成轮询。
- `shell_abort` 是唯一终止命令的工具。
- `yield` 是 agent-level 单次 sleep，不属于 shell，不做 repeat，不启动新 turn。

删除 `timeoutMs` 作为 shell tool 的默认控制语义。命令不应因为等待时间过长被隐式杀掉；
疑似卡死时由模型或用户显式调用 `shell_abort`。

## 2. 设计动机

现有 `shell_exec` / `shell_wait` / `shell_input` / `shell_abort` 能处理长命令，但有三个问题：

- `shell_wait` 同时承担“等待”和“读取状态”，容易形成忙轮询，也让 agent 缺少显式暂停点。
- `timeoutMs` 是硬终止，而且当前默认值会让长任务在后续 wait 中被意外杀掉。
- `shell_input` 这个名字偏交互输入；我们真正需要的是对 foreground command 写 stdin，空输入轮询必须消失。

新方案把等待从 shell 中移到 `yield`。shell 工具只表达 shell 控制动作，agent 等待节奏由模型通过
`yield` 决定。

## 3. 工具语义

### 3.1 `shell_exec`

启动一条 shell script。`yieldAfterMs` 必填，取值范围 `1..600000`，最大 10 分钟。

```ts
type ShellExecInput = {
  script: string
  shellId?: string
  yieldAfterMs: number
  maxOutputBytes?: number
}
```

语义：

- 如果指定 `shellId`，命令在该 shell session 中执行。
- 如果未指定 `shellId`，使用当前 agent session 的默认 shell；如果默认 shell 正在跑 foreground command，则创建辅助 shell。
- 如果目标 shell 正在跑 foreground command，且调用方显式指定了这个 `shellId`，返回错误并带出当前 `commandId`，不覆盖正在运行的命令。
- 命令在 `yieldAfterMs` 内退出，返回 `status: "exited"`。
- 命令在 `yieldAfterMs` 后仍在运行，返回 `status: "running"`，命令继续运行。
- `shell_exec` 不接受 `timeoutMs`，也不会隐式终止进程。

### 3.2 `shell_status`

读取一条命令的当前状态和 stdout/stderr 增量。它不等待、不写 stdin、不改变命令生命周期。

```ts
type ShellStatusInput = {
  commandId: string
  stdoutOffset?: number
  stderrOffset?: number
  maxOutputBytes?: number
}
```

语义：

- `commandId` 指向一次 foreground command，不是 shell session。
- 未传 offset 时，返回自上次成功 snapshot 后的新输出。
- 传 offset 时，从指定字节位置重新读取，支持模型或 UI 重新拉取历史输出。
- 如果命令仍在运行，返回 `status: "running"`。
- 如果命令已退出，返回 `status: "exited"` 和 `exitCode`。
- 如果命令已被中止，返回 `status: "aborted"`。
- 已完成命令的状态和 stdout/stderr artifact 在 AgentSession 生命周期内可继续读取。

### 3.3 `shell_write`

向 foreground command 写 stdin。它替代 `shell_input`，并且只接受非空输入。

```ts
type ShellWriteInput = {
  commandId: string
  stdin: string
  maxOutputBytes?: number
}
```

语义：

- `stdin` 必须非空；轮询必须用 `shell_status`。
- 写入目标 command 的 stdin 后，立即返回一次 status snapshot。
- 如果 command 不存在、已结束、或没有可写 stdin，返回错误。
- 是否等待写入后的输出，由模型通过 `yield` 再 `shell_status` 决定。

### 3.4 `shell_abort`

显式终止 foreground command。

```ts
type ShellAbortInput = {
  commandId: string
}
```

语义：

- 终止 command 所在 foreground process group。
- 返回最终 status snapshot 和 stdout/stderr 增量。
- 对已结束命令调用时返回该命令的最终状态，不重新杀 shell session。
- `shell_abort` 是控制动作，不默认表示 agent 任务失败。

### 3.5 `yield`

暂停当前 active turn 一段时间，然后把控制权交回模型。

```ts
type YieldInput = {
  durationMs: number
}
```

语义：

- `durationMs` 必填，取值范围 `1..600000`，最大 10 分钟。
- `yield` 不读 shell、不写 shell、不管理进程。
- 如果等待期间有用户 steer、abort、queue promotion 或其他 active-turn input，`yield` 提前结束并返回 `interrupted`。
- 如果时间到，返回 `completed`。
- `yield` 是单次 sleep；不支持 `repeat`、`start_yield` 或 `stop_yield`。

重复检查长命令时由模型显式组合：

```text
shell_exec(...)
yield(...)
shell_status(...)
yield(...)
shell_status(...)
```

跨 turn 的定时唤醒属于后续 heartbeat / automation 机制，不放进 `yield`。

## 4. 标识符模型

`shellId` 和 `commandId` 必须分开：

- `shellId`：长期 shell session 句柄，承载 cwd、env、函数、后台 job 和当前 foreground command。
- `commandId`：一次 foreground command 句柄，承载进程状态、stdout/stderr artifact、读取 offset 和 exit 信息。

一个 shell session 同时最多有一条 foreground command。后台 job 仍属于 shell session 状态，但模型可观测和可控的长程前台任务都用 `commandId`。

默认 shell 规则：

- 每个 AgentSession 有一个默认 shell。
- 默认 shell 空闲时，未传 `shellId` 的 `shell_exec` 复用它。
- 默认 shell 忙时，未传 `shellId` 的 `shell_exec` 创建辅助 shell，方便模型在 dev server 运行时执行一次性检查命令。
- 显式传入忙碌 `shellId` 时不自动创建辅助 shell，避免模型误以为命令跑在指定 shell 状态里。

## 5. stdout/stderr Artifact

每个 command 都有两个 append-only artifact：

```ts
type StreamArtifact = {
  path: string
  offset: number
  delta: string
  tail: string
  bytes: number
  truncated: boolean
}

type ShellCommandSnapshot = {
  status: 'running' | 'exited' | 'aborted'
  shellId: string
  commandId: string
  stdout: StreamArtifact
  stderr: StreamArtifact
  runningMs: number
  idleMs: number
  exitCode?: number
}
```

`path` 是 shell runtime 管理的 artifact path，不默认写进任务 cwd。LocalHost 可以把 artifact materialize 到本机磁盘，但运行时代码必须通过 Host 能力写入，不能绕过 Host 直接访问本机 fs。

artifact 内容是**shell redirection 后模型可见的 stdout/stderr**，不是 raw process fd：

- `cmd > file` 不应该把 stdout 泄漏进 stdout artifact。
- `cmd 2>/dev/null` 不应该把 stderr 泄漏进 stderr artifact。
- `cmd >&2` 应该进入 stderr artifact。
- `cmd > file` 的目标文件仍通过 `Host.fs` 按 shell 语义写入。

现有实现里 file sink 先缓存在内存，command settle 或 abort 时 flush。最终实现应改成流式写入：

- visible stdout/stderr chunk 实时 append 到 command artifact。
- file redirection sink 也应按 chunk 写入 `Host.fs`，保证长命令运行中目标文件可见。
- tool result 只返回 bounded delta/tail；完整输出通过 artifact + offset 重读。

offset 使用字节位置。`shell_status` 返回下一次读取应使用的 offset；当返回内容超过
`maxOutputBytes` 时设置 `truncated: true`，但 artifact 仍保留完整输出。

## 6. Agent Loop 行为

agent loop 只在明确边界恢复模型：

- `shell_exec` 的 `yieldAfterMs` 到点。
- command exit。
- `shell_status` / `shell_write` / `shell_abort` tool result 返回。
- `yield` 完成或被打断。
- provider stream 自身完成或出错。

输出 chunk 到达不直接唤醒模型。UI 可以基于 artifact 或 progress event 实时展示终端输出，但模型只有在工具返回后才继续推理。

## 7. 典型流程

### 长测试命令

```text
shell_exec({ script: "pnpm test", yieldAfterMs: 10000 })
→ running + commandId

yield({ durationMs: 30000 })
shell_status({ commandId })
→ running，返回新增 stdout/stderr

yield({ durationMs: 30000 })
shell_status({ commandId })
→ exited + exitCode
```

### Dev server 冒烟验证

```text
shell_exec({ script: "pnpm dev", yieldAfterMs: 3000 })
→ running + commandId=server

shell_exec({ script: "curl -I http://127.0.0.1:5173", yieldAfterMs: 10000 })
→ exited

shell_abort({ commandId: server })
```

默认 shell 忙时，第二个 `shell_exec` 会自动使用辅助 shell，不打断 dev server。

### 交互式输入

```text
shell_exec({ script: "node prompt.js", yieldAfterMs: 1000 })
→ running + commandId

shell_write({ commandId, stdin: "Alice\n" })
yield({ durationMs: 500 })
shell_status({ commandId })
```

### 疑似卡死

```text
shell_status({ commandId })
→ running, idleMs 很大

shell_abort({ commandId })
→ aborted + 最后输出
```

## 8. 为什么不做 repeat yield

`yield({ repeat: true })` 会把两种不同机制混在一起：

- active turn 内暂停：`yield`
- turn 结束后的定时唤醒：heartbeat / automation

如果 repeat 不把控制权交给模型，模型没有机会检查 `shell_status`。如果每次都交还给模型，它等价于单次 `yield`。因此 `yield` 保持单次；长期跨 turn 轮询以后用独立 heartbeat 设计。

## 9. 包职责

模型可见的五个工具是一个完整的 agent 基础工具组，不按工具名拆给不同包：

- `@demi/agent` 拥有模型可见工具面：`shell_exec` / `shell_status` / `shell_write` / `shell_abort` / `yield` 的名称、schema、tool call/result transcript 语义、与 active turn / steer / abort / queue 的集成，以及对 provider 暴露这些工具的装配。
- `@demi/shell` 拥有 shell runtime 服务：BashEnvironment、shell session、command record、command artifact、Host-backed stream sink，以及可被 agent 工具调用的 exec/status/write/abort primitives。它不拥有模型可见 AgentTool，也不决定 `yield` 语义。
- `@demi/coding-agent` 只在 prompt 中解释这五个工具的使用策略，不替换 agent 基础工具组、shell runtime 或 yield 实现。
- `@demi/web-ui` 和 `@demi/repl` 消费统一协议事件展示状态，不直接读 shell 内部对象。

因此最终分层不是“shell 工具在 `@demi/shell`、yield 工具在 `@demi/agent`”，而是：
`@demi/agent` 拥有完整工具面，`@demi/shell` 提供 shell 执行能力。

## 10. 测试与验收

单元测试模块与覆盖意图：

- `packages/shell/src/__tests__/tools.test.ts`
  - 工具 schema 只暴露 `shell_exec` / `shell_status` / `shell_write` / `shell_abort`。
  - `yieldAfterMs` 必填且最大 10 分钟。
  - `shell_write` 拒绝空 stdin。
  - tool result 包含 `shellId`、`commandId`、stdout/stderr artifact、offset 和 next action。

- `packages/shell/src/__tests__/environment.test.ts`
  - `shell_exec` 超过 `yieldAfterMs` 返回 `running` 且不杀进程。
  - `shell_status` 非阻塞读取新增 stdout/stderr。
  - `shell_status` 支持 offset 重读和 bounded delta。
  - command exit 后仍可读取最终 artifact。
  - `shell_abort` 终止 foreground process group 并保留最终输出。
  - redirection 不泄漏到可见 stdout/stderr artifact。
  - file redirection sink 在长命令运行中可见。
  - 默认 shell 忙时未指定 `shellId` 的 `shell_exec` 创建辅助 shell；指定忙碌 `shellId` 时拒绝。

- `packages/agent/src/__tests__/session.test.ts`
  - `yield` 完成后同一 active turn 继续 provider loop。
  - steer 会打断 `yield`，并在后续同一 turn continuation 中进入模型上下文。
  - abort 会取消 `yield`。
  - `yield` 不创建新 user turn、不写 shell 状态。

- `packages/agent/src/__tests__/server.test.ts`
  - AgentServer 对客户端暴露新工具面和对应 events。
  - 旧 `shell_wait` / `shell_input` frame 不再作为 final API。

- `packages/repl/src/__tests__/real-repl.e2e.test.ts`
  - 真实模型可用 `shell_exec → yield → shell_status` 监控长命令直到完成。
  - 真实模型可启动 dev server、用辅助 shell 验证、再 `shell_abort` 清理。

验收标准：

- 长命令超过 120 秒不会因为默认 timeout 被杀。
- 模型不需要空 stdin 或 `shell_wait` 也能读取长命令输出。
- stdout/stderr 大输出不会被塞满 transcript；tool result 只给 delta/tail，完整内容留在 artifact。
- 用户 steer 可以打断 `yield`，但不会隐式 abort shell command。
- 所有中止命令的路径都能在 transcript 中看到显式 `shell_abort`。

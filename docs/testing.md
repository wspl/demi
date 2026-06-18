# 测试模块说明

| | |
|---|---|
| 日期 | 2026-06-19 |
| 状态 | 草案 |
| 范围 | demi agent、shell、provider、RPC 与 coding agent |

## 1. 测试目标

demi 的测试要证明 agent 壳子真的能在协议、上下文、工具调用、shell 状态和持久化之间稳定工作。测试不只覆盖函数返回值，还要覆盖真实工作流里的关键不变量：provider request 内容、transcript 结构、tool call 收敛、shell session 生命周期、RPC action 顺序、abort/retry/resume/compact 的状态边界。

新增测试优先验证行为契约。实现细节可以被重构，但这些契约不能悄悄变化。

## 2. 常用命令

- `bun run typecheck`：类型检查。
- `bun run test`：运行 demi 主仓库自动化测试。
- `bun run test:just-bash-core`：运行 just-bash 子模块中 demi 依赖的核心解析保护测试。
- `bun run check:registry`：检查 public registry / package boundary。

真实 provider 或 TUI 冒烟测试不作为默认自动化测试入口。它们用于验收真实模型、真实 CLI 和交互壳子的组合行为，应该有明确的手动或 gated 运行方式。

## 3. 当前自动化测试模块

### 3.1 `packages/core`

负责平台边界和包分层约束。

应测试：

- browser-safe/runtime-neutral 根入口不静态依赖 Node-only 能力。
- 只有 RPC host 在运行时代码里直接实例化 `AgentSession`。
- package manifest 不引入越层依赖。
- demi 使用 forked just-bash 包，不在主仓库维护第二份上游源码快照。

### 3.2 `packages/provider`

负责 provider 抽象的基础设施。

应测试：

- `ProviderRegistry` 的注册、查找、创建和 snapshot 通知。
- `StubProvider` 的多轮事件脚本、函数式脚本、tool roundtrip 和脚本耗尽错误。

### 3.3 `packages/provider-claude-code`

负责 Claude Code CLI provider 适配层。

应测试：

- CLI 检测、认证状态读取、参数构造和环境变量契约。
- demi inference items 到 Claude JSONL input messages 的转换。
- Claude stdout message 到 demi provider events 的转换，包括 text、thinking、tool call、usage、cache usage。
- control request / SDK MCP control request / assistant tool_use 的跨 run 状态机。
- malformed tool request、缺失 tool_result、CLI 非零退出、stdout 迭代失败和 abort 的错误收敛。
- 与 `AgentSession` 和 shell tools 的集成路径。
- 真实 CLI e2e 只能作为 gated 测试，不能让默认测试依赖本机登录状态或网络稳定性。

### 3.4 `packages/base-agent`

负责通用 agent session runtime 和 transcript。

`session.test.ts` 应测试：

- `send` 写入 user turn、构造 provider request、记录 response。
- provider error 的 transcript、event、iterator close 行为。
- reference resolution 的执行顺序和 abort 行为。
- abort 对 provider、tool invocation、compaction summary 的收敛。
- store snapshot 和 extension state snapshot 的持久化入口。
- tool call 执行、tool error 转 tool result、tool progress event。
- queued send 的顺序与独立收敛。
- retry、resume、pending tool cleanup。
- compact 的基础边界插入和 usage 接近上限时的自动 compact + resume。
- mutation guard 对 busy/reserved 状态的保护。

`transcript.test.ts` 应测试：

- user、assistant text、response、tool_use、tool_result 的 inference item 转换。
- non-JSON tool input / extension state 的安全存储与 token estimate。
- dangling executing tool call 清理。
- compaction boundary / marker 的插入和 latest boundary replay。
- extension state snapshot 查询。

### 3.5 `packages/shell`

负责 Bash Environment、Host 抽象、注册命令、shell 工具和 session-local storage。

`command.test.ts` 应测试：

- `CommandSpec` 是 prompt/help、解析、校验和 JSON output schema 的单一来源。
- 参数、flag、stdin field、`--json`、数组、数字和布尔值解析。
- 注册命令名不复用 shell/system 保留命令。

`environment.test.ts` 应测试：

- cwd/env/function/job/positional parameter/last exit status 在 shell session 内持续。
- `cd`、`export`、`unset`、`read`、`source`、`set`、`eval`、`type`、`pushd/popd`、`exit` 等状态类 builtin 不落到系统 shell。
- expansion、redirection、pipeline、list operator、compound command、loop、subshell、function、本地变量和 `break/continue/return` 的 shell 语义。
- registered command、state builtin、function、system command 的调度顺序和 audit。
- parser 不支持的语法明确拒绝，不整段交给系统 shell。
- foreground command 的 running/yield、`shell_wait`、`shell_input`、`shell_abort`、timeout、output limit 和 dispose。
- abort/timeout 后 shell session 可继续复用，且重定向输出不会泄漏到 tool output。

其他 shell 测试应覆盖：

- `HostBackedFileSystem` 通过 `Host.spawn` 完成读写、stat、exists、readdir 和二进制读取。
- `LocalHost` 的 spawn、stdin 和 terminate。
- `AgentSessionCommandStorage` 按 agent session id 隔离状态。
- shell tool result 的模型可读格式和 abort signal 传播。

### 3.6 `packages/agent-coding`

负责 coding agent definition 和 agent 专属注册命令。

应测试：

- coding agent 暴露 shell session tools，并把注册命令说明注入 prompt。
- file reference 通过 workspace host 解析，并拒绝越出 workspace root 的路径。
- definition dispose 清理 shell sessions。
- `editor create/edit/patch` 的路径安全、精确替换、上下文消歧、空 old text 拒绝、unified diff、跨文件事务和删除文件。
- `todo add/list/update/done` 的 raw/JSON 输出，以及按 agent session id 隔离状态。
- 端到端 coding workflow：StubProvider 通过 shell tools 驱动 editor/todo，真实写文件并更新 session-local todo。

### 3.7 `packages/rpc`

负责协议层、client/host 状态同步和 transports。

应测试：

- JSON codec 保留 BigInt metadata 和 Uint8Array。
- transcript diff/patch 能处理 in-place tool metadata 变化、移除和非 JSON metadata。
- in-process、stdio、WebSocket transport 端到端传输 RPC frames。
- open/send/close/abort/retry/resume/compact 的 phase、transcript、error 和 action promise 收敛。
- provider error code 只转发一次，并保留 transcript error block。
- shell output、bash audit 和 generic tool progress 的 frame 映射。
- `shell_input` frame 能桥接到 active shell session tool。
- queued send 按各自 phase cycle resolve/reject，不共享同一个 idle 事件。
- close/dispose 能中止 active session 并释放 definition 资源。

### 3.8 `packages/tui`

TUI 是交互壳子，当前不在 `bun run test` 默认自动化覆盖内。

应测试：

- 基本渲染、输入、scroll、tool/thinking/text 输出和错误显示。
- 通过 RPC client 打开 session、发送消息、接收 transcript/phase/shell frames。
- 手动或 gated 冒烟验证真实 provider、真实模型回复、thinking 输出、tool use 输出和交互式 shell 操作。

### 3.9 `packages/just-bash`

just-bash 是子模块，demi 只消费其中的 workspace package。

应测试：

- demi 依赖的 parser protection、edge cases 和 parse errors。
- upstream spec/comparison 测试属于子模块自己的质量网；demi 主仓库默认只运行必要核心集。
- 修改子模块行为时，在子模块内提交对应测试和代码，再在主仓库提交 submodule pointer。

## 4. 必补覆盖缺口

### 4.1 Agent scenario tests

需要新增跨 `AgentSession + coding definition + BashEnvironment + StubProvider` 的真实场景测试。

应覆盖：

- 创建文件、运行测试失败、读取错误、修复代码、测试通过。
- 长命令 running/yield 后继续 wait，必要时通过 shell input 交互。
- tool error 后模型继续恢复，而不是 session 卡死。
- 多轮 user message 与 queued send 的 transcript 顺序。
- agent session id 与 shell id 不混淆，todo/storage 始终按 agent session 隔离。

### 4.2 Compact 正确性

当前 compact 只覆盖基础结构路径，还不够。

必须补：

- cut point 不能切断 `tool_use -> tool_result`。
- compact 后下一次 provider request 必须精确等于 summary + 保留的 recent context。
- 多次 compact 只从 latest boundary replay，不重复旧 summary。
- compaction summary 失败、abort 或 provider error 时 transcript 不留下半截 boundary/marker。
- auto compact + resume 不重复执行已完成 tool call。
- compact 后持久化再恢复，replayable blocks 和 provider request 保持一致。
- thinking、redacted thinking、extension state snapshot、tool metadata 混在 transcript 中时仍然正确。

### 4.3 Context cache

context cache 目前还没有明确 contract。已有测试只验证 provider usage 中 cache read/write token 字段能被解析，不等于保证 cache 命中或 cache 前缀稳定。

先定义 contract，再补测试：

- 如果 cache 只是 provider 透明优化，测试应保证 cache usage 被解析、记录、暴露，并且不会影响 agent 行为。
- 如果 demi 要主动保障 cache 命中，测试应保证 system prompt、tools schema 和稳定 transcript prefix 在跨 turn 时保持字节级稳定。
- tools/schema/system prompt/model/preamble 改变时，应明确是否失效 cache，并测试对应行为。
- compact 后 cache prefix 如何变化必须有测试：第一次 compact 改变 prefix，后续 turn 再次稳定。
- 真实 provider 的 cache 命中检查只能作为 gated smoke，不能作为默认 deterministic 测试。

### 4.4 Persistence and recovery

需要新增 session 重建测试。

应覆盖：

- 从 store snapshot 重建后继续 send/retry/resume/compact。
- abort 后恢复不会保留 dangling active tool。
- provider error 后恢复不会重复发送已完成 tool result。
- compact 后恢复只 replay latest boundary 之后的内容。

### 4.5 RPC plus real transports

需要补更贴近外部壳子的协议测试。

应覆盖：

- stdio/WebSocket transport 下的 queued send、abort、retry、resume、compact 行为与 in-process 一致。
- shell_output/audit/tool_progress 在真实 transport 序列化后不丢字段、不乱序。
- close 时 active foreground process 被终止，definition dispose 只执行一次。

## 5. 新增测试放置规则

- 单包内部行为放在对应 package 的 `src/__tests__/`。
- 跨包 agent 行为放在最靠近行为 owner 的 package；通用 session 行为放 `base-agent`，coding 工作流放 `agent-coding`，协议行为放 `rpc`。
- 涉及真实 CLI、真实模型、网络或本机登录状态的测试必须 gated，不进入默认 `bun run test`。
- 测试应断言真实 artifact、provider request、transcript blocks、events、session state 或文件内容；避免只断言 mock 函数被调用。
- 新增 architecture/workflow 约束时，先更新 `docs/agent-rewrite-plan.md` 或本文件，再补测试。

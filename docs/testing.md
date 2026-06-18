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

## 2. 默认测试入口

- `bun run typecheck`：类型检查。
- `bun run test`：主仓库默认自动化测试。
- `bun run test:just-bash-core`：demi 依赖的 just-bash 核心解析保护测试。
- `bun run check:registry`：public registry / package boundary 检查。

## 3. 模块测试点

### 3.1 平台与包边界

Owner：`packages/core`

| 测试点 | 状态 | 现有覆盖 / 待补 |
|---|---|---|
| 根入口保持 browser-safe / runtime-neutral，不静态依赖 Node-only 能力 | 已覆盖 | `packages/core/src/__tests__/platform-entrypoints.test.ts` |
| 只有 RPC host 在运行时代码中直接实例化 `AgentSession` | 已覆盖 | `platform-entrypoints.test.ts` |
| package manifest 不引入越层依赖 | 已覆盖 | `platform-entrypoints.test.ts` |
| 主仓库使用 forked just-bash package，不维护第二份上游源码快照 | 已覆盖 | `platform-entrypoints.test.ts` |

### 3.2 Provider 抽象

Owner：`packages/provider`

| 测试点 | 状态 | 现有覆盖 / 待补 |
|---|---|---|
| `ProviderRegistry` 注册、查找、创建 provider | 已覆盖 | `packages/provider/src/__tests__/registry.test.ts` |
| `ProviderRegistry` registration snapshot 通知 | 已覆盖 | `registry.test.ts` |
| `StubProvider` 多轮 scripted events | 已覆盖 | `packages/provider/src/__tests__/stub.test.ts` |
| `StubProvider` function script 能读取 request 并驱动 tool roundtrip | 已覆盖 | `stub.test.ts` |
| `StubProvider` 脚本耗尽时报错 | 已覆盖 | `stub.test.ts` |

### 3.3 Claude Code Provider

Owner：`packages/provider-claude-code`

| 测试点 | 状态 | 现有覆盖 / 待补 |
|---|---|---|
| Claude CLI path/version 检测 | 已覆盖 | `packages/provider-claude-code/src/__tests__/cli.test.ts` |
| Claude auth/runtime state 读取，不依赖 SDK | 已覆盖 | `cli.test.ts` |
| Claude CLI args/env 构造契约 | 已覆盖 | `cli.test.ts` |
| demi inference items 转 Claude JSONL input messages | 已覆盖 | `packages/provider-claude-code/src/__tests__/jsonl-output.test.ts` |
| assistant turn 与 tool result grouping | 已覆盖 | `jsonl-output.test.ts` |
| binary media content 转 Claude base64 source | 已覆盖 | `jsonl-output.test.ts` |
| Claude stdout 映射 text/thinking/tool call/usage | 已覆盖 | `jsonl-output.test.ts` |
| cache read/write usage 字段解析 | 已覆盖 | `jsonl-output.test.ts` 只覆盖 usage 字段映射 |
| malformed assistant tool_use 报错 | 已覆盖 | `jsonl-output.test.ts`、`provider.test.ts` |
| control_request 跨 run 状态机 | 已覆盖 | `packages/provider-claude-code/src/__tests__/provider.test.ts` |
| SDK MCP control_request 跨 run 状态机 | 已覆盖 | `provider.test.ts` |
| assistant tool_use 跨 run 状态机 | 已覆盖 | `provider.test.ts` |
| 缺失 tool_result / pending control_request 收敛为 provider error | 已覆盖 | `provider.test.ts` |
| malformed tools/call request 不进入 pending state | 已覆盖 | `provider.test.ts` |
| CLI 非零退出且没有 result message 时报告错误 | 已覆盖 | `provider.test.ts` |
| stdout 迭代失败后清理 active transport | 已覆盖 | `provider.test.ts` |
| abort 会 kill active transport | 已覆盖 | `provider.test.ts` |
| 与 `AgentSession` 和 shell tools 的 provider 集成 | 已覆盖 | `provider.test.ts` |
| 真实 Claude CLI e2e | 手动/Gated | `real-cli.e2e.test.ts`，不属于默认稳定测试 |
| 真实模型 thinking/tool use/text 输出验收 | 手动/Gated | 需要 TUI/CLI smoke 流程 |

### 3.4 Agent Session Runtime

Owner：`packages/base-agent`

| 测试点 | 状态 | 现有覆盖 / 待补 |
|---|---|---|
| `send` 写入 user turn，再构造 provider request | 已覆盖 | `packages/base-agent/src/__tests__/session.test.ts` |
| provider response 写入 transcript | 已覆盖 | `session.test.ts` |
| provider error 写入 transcript、发 event、reject action | 已覆盖 | `session.test.ts` |
| provider error 后关闭 provider iterator | 已覆盖 | `session.test.ts` |
| reference resolution 在写入 user turn 和 provider request 前执行 | 已覆盖 | `session.test.ts` |
| reference resolution 卡住时 abort 可收敛 | 已覆盖 | `session.test.ts` |
| provider 不 yield 时 abort 可收敛 | 已覆盖 | `session.test.ts` |
| long-running tool invocation 卡住时 abort 可收敛 | 已覆盖 | `session.test.ts` |
| tool call 执行后继续 provider roundtrip | 已覆盖 | `session.test.ts` |
| tool invocation throw 转成 error tool result 并继续 | 已覆盖 | `session.test.ts` |
| tool progress event 发出 | 已覆盖 | `session.test.ts` |
| queued send 排队并按顺序 drain | 已覆盖 | `session.test.ts` |
| retry 截断最后 assistant response 并 rerun latest user turn | 已覆盖 | `session.test.ts` |
| resume 标记 abort 为 resumed，并追加 resume turn | 已覆盖 | `session.test.ts` |
| abort 后 resume 前清理 pending tool calls | 已覆盖 | `session.test.ts` |
| mutation guard 拒绝 busy/reserved 期间 mutation | 已覆盖 | `session.test.ts` |
| store snapshot 写入 | 已覆盖 | `session.test.ts` |
| extension state snapshot 通过 lifecycle 写入并持久化 | 已覆盖 | `session.test.ts` |
| 从 store snapshot 重建 session 后继续运行 | 未覆盖 | 需要 persistence/recovery 测试 |
| provider error 后恢复不重复发送已完成 tool result | 未覆盖 | 需要恢复场景测试 |
| abort/retry/resume/compact 组合交错 | 部分覆盖 | 单点已测，组合路径不足 |

### 3.5 Transcript 与 Replay

Owner：`packages/base-agent`

| 测试点 | 状态 | 现有覆盖 / 待补 |
|---|---|---|
| user/text/response blocks 转 inference items | 已覆盖 | `packages/base-agent/src/__tests__/transcript.test.ts` |
| pending tool call 发现与完成 | 已覆盖 | `transcript.test.ts` |
| completed tool call 转 `tool_use` + `tool_result` | 已覆盖 | `transcript.test.ts` |
| non-JSON tool input 安全存储和 replay | 已覆盖 | `transcript.test.ts` |
| dangling executing tool call 清理 | 已覆盖 | `transcript.test.ts` |
| extension state snapshot latest 查询 | 已覆盖 | `transcript.test.ts` |
| non-JSON extension state token estimate | 已覆盖 | `transcript.test.ts` |
| transcript snapshot 序列化/反序列化等价 | 部分覆盖 | 有 store snapshot 写入测试，缺少重建后 replay 等价测试 |
| provider request exact replay 内容 | 部分覆盖 | 简单 send 覆盖，复杂 transcript 未覆盖 |

### 3.6 Compaction

Owner：`packages/base-agent`

| 测试点 | 状态 | 现有覆盖 / 待补 |
|---|---|---|
| 手动 compact 插入 boundary 和 marker | 已覆盖 | `session.test.ts` |
| compact 不删除旧 blocks | 已覆盖 | `session.test.ts` |
| replay 从 latest compaction boundary 开始 | 已覆盖 | `transcript.test.ts` |
| boundary summary 转成下一次 inference user message | 已覆盖 | `transcript.test.ts` |
| usage 接近 context limit 时自动 compact + resume | 已覆盖 | `session.test.ts` |
| compaction summary provider 卡住时 abort 可收敛 | 已覆盖 | `session.test.ts` |
| cut point 不能切断 `tool_use -> tool_result` | 未覆盖 | 需要 transcript cut-point invariant 测试 |
| compact 后下一次 provider request 精确等于 summary + recent context | 未覆盖 | 需要 exact request 测试 |
| 多次 compact 只 replay latest boundary，不重复旧 summary | 未覆盖 | 需要 multi-compact 测试 |
| summary provider error/abort 不留下半截 boundary/marker | 部分覆盖 | abort 收敛已测，transcript 原子性未完整断言 |
| auto compact + resume 不重复执行已完成 tool call | 未覆盖 | 需要 tool + compact 场景测试 |
| compact 后持久化再恢复，replayable blocks 保持一致 | 未覆盖 | 需要 persistence roundtrip 测试 |
| thinking/redacted thinking/extension state/tool metadata 混合 transcript 下 cut point 正确 | 未覆盖 | 需要复杂 transcript fixture |

### 3.7 Context Cache 与 Usage

Owner：`packages/provider-claude-code`、`packages/base-agent`

| 测试点 | 状态 | 现有覆盖 / 待补 |
|---|---|---|
| provider usage 中 cache read/write token 字段被解析 | 已覆盖 | `jsonl-output.test.ts` |
| cache usage 被 AgentSession 记录并对外暴露 | 部分覆盖 | response usage 记录路径存在，缺少专门 cache usage 断言 |
| cache 只是 provider 透明优化时，不影响 agent 行为 | 未覆盖 | 需要定义 contract 后测试 |
| demi 主动保障 stable prompt prefix 时，跨 turn prefix 字节级稳定 | 未覆盖 | 需要先定义是否要求主动保障 |
| tools/schema/system prompt/model/preamble 改变时 cache 失效规则 | 未覆盖 | 需要先定义 contract |
| compact 后 cache prefix 变化与重新稳定 | 未覆盖 | 需要 compact + cache contract 测试 |
| 真实 provider cache 命中 | 手动/Gated | 只能做真实 provider smoke，不进默认 deterministic 测试 |

### 3.8 Command Registry

Owner：`packages/shell`

| 测试点 | 状态 | 现有覆盖 / 待补 |
|---|---|---|
| positionals、flags、stdin fields 解析 | 已覆盖 | `packages/shell/src/__tests__/command.test.ts` |
| long options 校验与数字 coercion | 已覆盖 | `command.test.ts` |
| `--json`、boolean、repeated array options | 已覆盖 | `command.test.ts` |
| unknown options / invalid values 拒绝 | 已覆盖 | `command.test.ts` |
| `CommandSpec` 作为 prompt/help 单一来源 | 已覆盖 | `command.test.ts` |
| registry 注册命令并渲染 prompt | 已覆盖 | `command.test.ts` |
| 注册命令名不能复用 shell/system reserved names | 已覆盖 | `command.test.ts` |
| `<command> prompt` 使用同一个 renderer | 已覆盖 | `command.test.ts` |
| JSON mode output schema 校验 | 已覆盖 | `command.test.ts` |
| 无 JSON schema 的 subcommand 拒绝 JSON mode | 已覆盖 | `command.test.ts` |

### 3.9 Bash Environment 语义

Owner：`packages/shell`

| 测试点 | 状态 | 现有覆盖 / 待补 |
|---|---|---|
| cwd/env 跨 `shell_exec` 保持 | 已覆盖 | `packages/shell/src/__tests__/environment.test.ts` |
| stateful builtins 在同一 script 中影响后续 expansion | 已覆盖 | `environment.test.ts` |
| invalid stateful builtin 不污染状态 | 已覆盖 | `environment.test.ts` |
| `unset` 修改变量和函数 | 已覆盖 | `environment.test.ts` |
| `read` 消费 stdin 并写入 session | 已覆盖 | `environment.test.ts` |
| list operators 不 fallback 到系统 shell | 已覆盖 | `environment.test.ts` |
| prefix assignments 和 assignment-only commands | 已覆盖 | `environment.test.ts` |
| `$?` 跨命令和 exec 调用保持 | 已覆盖 | `environment.test.ts` |
| `exit` 标记 session exited 并遵循 exit status rules | 已覆盖 | `environment.test.ts` |
| parameter expansion、arithmetic expansion、arithmetic commands | 已覆盖 | `environment.test.ts` |
| command substitution 在当前 shell context 执行 | 已覆盖 | `environment.test.ts` |
| `pushd/popd` directory stack | 已覆盖 | `environment.test.ts` |
| shell functions 跨 exec 保持 | 已覆盖 | `environment.test.ts` |
| `local` / `return` 在函数和 sourced script 中工作 | 已覆盖 | `environment.test.ts` |
| function definition redirection 在调用时应用 | 已覆盖 | `environment.test.ts` |
| background jobs 跨 exec 保持，spawn failure 可 wait | 已覆盖 | `environment.test.ts` |
| `source` 修改当前 session、按 PATH 解析、继承 input redirection | 已覆盖 | `environment.test.ts` |
| `set --`、`shift`、`errexit`、`noglob` | 已覆盖 | `environment.test.ts` |
| `eval` 在当前 session 执行 | 已覆盖 | `environment.test.ts` |
| registered command / builtin / function / system command 调度顺序 | 已覆盖 | `environment.test.ts` |
| `command` builtin lookup 和 system execution | 已覆盖 | `environment.test.ts` |
| `type` builtin 报告真实 resolution | 已覆盖 | `environment.test.ts` |
| registered command storage 按 agent session scope | 已覆盖 | `environment.test.ts` |
| heredoc 到 registered command stdinField | 已覆盖 | `environment.test.ts` |
| heredoc 到 system command 并展开 unquoted variables | 已覆盖 | `environment.test.ts` |
| glob expansion 通过 Host | 已覆盖 | `environment.test.ts` |
| file redirection 不交给系统 shell | 已覆盖 | `environment.test.ts` |
| simple pipelines、compound commands in pipelines、negated pipelines | 已覆盖 | `environment.test.ts` |
| `if`、`[[ ]]`、`case`、for/C-style for/while/until/group/subshell | 已覆盖 | `environment.test.ts` |
| explicit `exit` 不被 `!`、if、loop、group 错误改写 | 已覆盖 | `environment.test.ts` |
| `break` / `continue` 语义与非法用法校验 | 已覆盖 | `environment.test.ts` |
| unsupported parser constructs 明确拒绝，不整段交给系统 shell | 已覆盖 | `environment.test.ts` |
| system command audit events | 已覆盖 | `environment.test.ts` |
| system command spawn failure 不挂死 | 已覆盖 | `environment.test.ts` |
| 更完整的 bash 兼容性 spec | 部分覆盖 | 主仓库覆盖关键 agent 语义；更完整 spec 在 just-bash 子模块 |

### 3.10 Shell 控制面

Owner：`packages/shell`

| 测试点 | 状态 | 现有覆盖 / 待补 |
|---|---|---|
| `shell_exec` running/yield 后可 `shell_wait` | 已覆盖 | `environment.test.ts`、`packages/shell/src/__tests__/tools.test.ts` |
| output limit 触发 running output_limit | 已覆盖 | `environment.test.ts` |
| `shell_input` 可向 foreground process 写 stdin | 已覆盖 | `environment.test.ts`、`tools.test.ts` |
| idle foreground process 默认保持 running，不误报 needs_input | 已覆盖 | `environment.test.ts` |
| `shell_wait` 的 `yieldAfterMs` 从每次调用重新计时 | 已覆盖 | `environment.test.ts` |
| `shell_abort` 终止 foreground process | 已覆盖 | `environment.test.ts`、`tools.test.ts` |
| `shell_abort` 是 control action，不是 tool failure | 已覆盖 | `tools.test.ts` |
| abort/timeout flush redirected foreground output | 已覆盖 | `environment.test.ts` |
| dispose shell kill foreground/background 并移除 session | 已覆盖 | `environment.test.ts` |
| dispose all shells | 已覆盖 | `environment.test.ts` |
| timeout kill foreground process | 已覆盖 | `environment.test.ts` |
| timeout/abort 后 shell 可复用，不泄漏 abort state | 已覆盖 | `environment.test.ts` |
| shell tool result 格式保留 metadata 且模型可读 | 已覆盖 | `tools.test.ts` |
| `shell_input` 拒绝空 stdin，不承担 polling | 已覆盖 | `tools.test.ts` |
| AgentSession abort signal 传播到 `shell_exec` / `shell_wait` / `shell_input` | 已覆盖 | `tools.test.ts` |
| 模型在真实长进程场景中稳定选择 wait/input/abort | 手动/Gated | 需要真实 provider/TUI smoke 多次验证 |

### 3.11 Host、FS 与 Store

Owner：`packages/shell`

| 测试点 | 状态 | 现有覆盖 / 待补 |
|---|---|---|
| `HostBackedFileSystem.readFile` 通过 `Host.spawn cat` | 已覆盖 | `packages/shell/src/__tests__/host-fs.test.ts` |
| exists/stat/writeFile/appendFile/readdir/resolvePath | 已覆盖 | `host-fs.test.ts` |
| readFileBuffer 返回 raw bytes | 已覆盖 | `host-fs.test.ts` |
| `LocalHost` spawn capture stdout | 已覆盖 | `packages/shell/src/__tests__/local-host.test.ts` |
| `LocalHost` 写 stdin 给 process | 已覆盖 | `local-host.test.ts` |
| `LocalHost` terminate foreground process | 已覆盖 | `local-host.test.ts` |
| `AgentSessionCommandStorage` 按 agent session id prefix 隔离 keys | 已覆盖 | `packages/shell/src/__tests__/store.test.ts` |
| storage 拒绝逃逸 session prefix 的 key/session id | 已覆盖 | `store.test.ts` |
| `LocalDemiStore` 拒绝非相对 store path | 已覆盖 | `store.test.ts` |
| 远程 Host / 容器 Host | 未覆盖 | 尚未实现 |

### 3.12 Coding Agent Definition

Owner：`packages/agent-coding`

| 测试点 | 状态 | 现有覆盖 / 待补 |
|---|---|---|
| coding definition 暴露 shell session tools | 已覆盖 | `packages/agent-coding/src/__tests__/coding-definition.test.ts` |
| registered command prompt 注入 system prompt | 已覆盖 | `coding-definition.test.ts` |
| file reference 通过 workspace host 读取 | 已覆盖 | `coding-definition.test.ts` |
| file reference 拒绝 workspace root 外路径 | 已覆盖 | `coding-definition.test.ts` |
| definition dispose 清理 environment shell sessions | 已覆盖 | `coding-definition.test.ts` |
| reference resolution 与 AgentSession send 顺序集成 | 部分覆盖 | base-agent 有通用覆盖，coding 只覆盖 definition 层 |

### 3.13 Editor Command

Owner：`packages/agent-coding`

| 测试点 | 状态 | 现有覆盖 / 待补 |
|---|---|---|
| `editor create` 用 heredoc 创建文件 | 已覆盖 | `packages/agent-coding/src/__tests__/editor-command.test.ts` |
| editor 拒绝 workspace root 外路径 | 已覆盖 | `editor-command.test.ts` |
| patch escaped path 时写入前拒绝 | 已覆盖 | `editor-command.test.ts` |
| `editor edit` exact replace | 已覆盖 | `editor-command.test.ts` |
| ambiguous matches 失败 | 已覆盖 | `editor-command.test.ts` |
| context disambiguation 只在唯一最近匹配时生效 | 已覆盖 | `editor-command.test.ts` |
| empty old text 拒绝且不修改文件 | 已覆盖 | `editor-command.test.ts` |
| unified diff patch | 已覆盖 | `editor-command.test.ts` |
| unified diff headers with timestamps | 已覆盖 | `editor-command.test.ts` |
| patch 多文件并创建新文件 | 已覆盖 | `editor-command.test.ts` |
| patch 删除 `/dev/null` target 文件 | 已覆盖 | `editor-command.test.ts` |
| patch 全量校验后再写入，保证跨文件事务 | 已覆盖 | `editor-command.test.ts` |

### 3.14 Todo Command

Owner：`packages/agent-coding`

| 测试点 | 状态 | 现有覆盖 / 待补 |
|---|---|---|
| `todo add/list/update/done` raw output | 已覆盖 | `packages/agent-coding/src/__tests__/todo-command.test.ts` |
| `todo add/list/update/done` JSON output | 已覆盖 | `todo-command.test.ts` |
| todo 状态按 agent session id 隔离 | 已覆盖 | `todo-command.test.ts` |
| todo 与 shell id 不混淆 | 部分覆盖 | 已通过 session scoped storage 间接覆盖，缺少端到端多 shell 场景 |

### 3.15 Coding Agent 工作流

Owner：`packages/agent-coding`

| 测试点 | 状态 | 现有覆盖 / 待补 |
|---|---|---|
| StubProvider 通过 shell tools 驱动 editor/todo，真实写文件 | 已覆盖 | `packages/agent-coding/src/__tests__/coding-marathon.test.ts` |
| workflow 中复用同一个 shell session | 已覆盖 | `coding-marathon.test.ts` |
| workflow 后文件内容正确 | 已覆盖 | `coding-marathon.test.ts` |
| workflow 后 todo 状态在 agent session 下可读 | 已覆盖 | `coding-marathon.test.ts` |
| 创建文件 -> 测试失败 -> 读取错误 -> 修复 -> 测试通过 | 未覆盖 | 需要更真实的 agent scenario test |
| 长命令 running/yield -> wait -> input/abort 的 agent 级流程 | 未覆盖 | shell 层已测，AgentSession + coding workflow 未测 |
| tool error 后模型恢复继续执行任务 | 部分覆盖 | base-agent tool error 已测，coding workflow 场景未测 |
| 多轮 user message 对 coding workflow 的影响 | 未覆盖 | 需要 multi-turn scenario |
| 多 shell + 同 agent session 的 todo/storage 一致性 | 未覆盖 | 需要 agent scenario |

### 3.16 RPC 协议

Owner：`packages/rpc`

| 测试点 | 状态 | 现有覆盖 / 待补 |
|---|---|---|
| JSON codec 保留 BigInt metadata 和 Uint8Array | 已覆盖 | `packages/rpc/src/__tests__/json-codec.test.ts` |
| transcript patch 更新 in-place tool_call metadata/status | 已覆盖 | `packages/rpc/src/__tests__/patch.test.ts` |
| transcript diff 处理非 JSON / cyclic metadata | 已覆盖 | `patch.test.ts` |
| root entry 不导出 node-only stdio transports | 已覆盖 | `packages/rpc/src/__tests__/root-entry.test.ts` |
| RpcClient open/send 经 InProcessTransport 发 transcript/phase | 已覆盖 | `packages/rpc/src/__tests__/rpc.test.ts` |
| client close 清空本地 transcript view | 已覆盖 | `rpc.test.ts` |
| provider error code 只 forward 一次，并保留 transcript error block | 已覆盖 | `rpc.test.ts` |
| shell tool progress 映射为 shell_output/audit frames | 已覆盖 | `rpc.test.ts` |
| generic tool progress 转 valid tool_progress text output | 已覆盖 | `rpc.test.ts` |
| 只转发 well-formed bash audit progress | 已覆盖 | `rpc.test.ts` |
| shell_input frames 桥接到 active shell session tool | 已覆盖 | `rpc.test.ts` |
| client `shellInput` 等待 result，未 open 时 reject | 已覆盖 | `rpc.test.ts` |
| retry 产生 transcript patch removals | 已覆盖 | `rpc.test.ts` |
| host queued send while busy 并按序 drain | 已覆盖 | `rpc.test.ts` |
| busy 时 host 拒绝 retry/resume/compact | 已覆盖 | `rpc.test.ts` |
| client queued send promise 按各自 phase cycle resolve | 已覆盖 | `rpc.test.ts` |
| error 后只 reject active action，queued send 继续 | 已覆盖 | `rpc.test.ts` |
| abort idle 返回 false，active 返回 true | 已覆盖 | `rpc.test.ts` |
| close frame abort active session | 已覆盖 | `rpc.test.ts` |
| close/dispose definition resources | 已覆盖 | `rpc.test.ts` |
| stdio transport 保留 Uint8Array 并端到端传 RpcClient/RpcHost | 已覆盖 | `packages/rpc/src/__tests__/stdio-transport.test.ts` |
| stdio child-process RpcHost | 已覆盖 | `stdio-transport.test.ts` |
| WebSocket transport JSON text frames 和 binary fields | 已覆盖 | `packages/rpc/src/__tests__/websocket-transport.test.ts` |
| WebSocket RpcClient/RpcHost 端到端 | 已覆盖 | `websocket-transport.test.ts` |
| stdio/WebSocket 下 queued send、abort、retry、resume、compact 与 in-process 一致 | 部分覆盖 | transport e2e 基础已测，复杂 action 组合未覆盖 |
| close 时 active foreground process 通过真实 transport 被终止 | 未覆盖 | 需要 RPC + shell long process 场景 |

### 3.17 TUI

Owner：`packages/tui`

| 测试点 | 状态 | 现有覆盖 / 待补 |
|---|---|---|
| 基本渲染、输入、scroll | 未覆盖 | 需要 TUI 自动化或 snapshot/integration 测试 |
| thinking/text/tool output 显示 | 手动/Gated | 目前靠真实 TUI smoke 验收 |
| 通过 RPC client open/send/receive phase/transcript/shell frames | 部分覆盖 | RPC 层已覆盖；TUI 壳子自身未覆盖 |
| 真实 Claude Code provider 输出真实模型回复 | 手动/Gated | 需要指定真实模型和 thinking 等级 smoke |
| 交互式 shell 操作在 TUI 中顺畅 | 手动/Gated | 需要多次 smoke，因为模型行为有随机性 |

### 3.18 just-bash 子模块

Owner：`packages/just-bash`

| 测试点 | 状态 | 现有覆盖 / 待补 |
|---|---|---|
| demi 依赖的 parser protection | 已覆盖 | `bun run test:just-bash-core` |
| parser edge cases | 已覆盖 | `bun run test:just-bash-core` |
| parse errors | 已覆盖 | `bun run test:just-bash-core` |
| upstream bash/awk/sed/grep/jq 等 spec/comparison 测试 | 部分覆盖 | 存在于子模块，不属于主仓库默认入口 |

## 4. 当前优先补测顺序

1. `base-agent` compaction invariants：tool boundary、exact replay、多次 compact、failure atomicity、persistence roundtrip。
2. context cache contract：先决定是只记录 provider usage，还是主动保障 stable prompt prefix；再补对应测试。
3. agent scenario tests：覆盖创建文件、测试失败、修复、测试通过、长命令和 tool error recovery。
4. persistence/recovery：从 store snapshot 重建后继续 send/retry/resume/compact。
5. RPC real transport complex actions：stdio/WebSocket 下 queued send、abort、retry、resume、compact 与 long shell process。
6. TUI 自动化或 gated smoke SOP：覆盖真实 provider、thinking、tool use、交互式 shell 输出。

## 5. 新增测试放置规则

- 单模块行为放在 owner package 的 `src/__tests__/`。
- 跨模块行为放在最接近不变量 owner 的 package：session/runtime 放 `base-agent`，coding workflow 放 `agent-coding`，协议收敛放 `rpc`。
- 涉及真实 CLI、真实模型、网络、本机登录状态或交互 UI 的测试必须 gated，不进入默认 `bun run test`。
- 测试应断言真实 artifact、provider request、transcript blocks、events、session state、RPC frames 或文件内容。
- 新增 architecture/workflow 约束时，先更新 `docs/agent-rewrite-plan.md` 或本文件，再补测试。

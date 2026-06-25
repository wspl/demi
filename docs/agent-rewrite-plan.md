# Agent 架构重写方案

| | |
|---|---|
| 日期 | 2026-06-19 |
| 状态 | 草案 |
| 范围 | 通用 agent 壳 + coding agent |

## 1. 背景与目标

在 TypeScript 里重写整套 agent 架构，覆盖通用 agent 壳与 coding agent。以 Rust `agent-session` / `coding-agent` 为实现蓝本：已验证的 session lifecycle、turn queue、retry/resume、compaction、mutation guard 等逻辑直接照搬，只在语言、包边界和 Shell/Bash 核心机制上调整。Active turn steer 的详细设计见 `docs/agent-steer-plan.md`；agent 性能评估体系见 `docs/agent-evaluation-plan.md`；Shell + yield 控制面的最终设计见 `docs/shell-yield-control-plan.md`；标准工具展示规范见 `docs/tool-rendering-spec.md`。

参考实现：

- Rust `agent-session`：session runtime、transcript、生命周期、compaction、queue、retry/resume、mutation guard。
- Codex active turn steer：queue 与 steer 是 busy session 的两种不同输入策略，不能互相伪装或自动降级。Codex core 的 steer 是 session-level same-turn pending input：接受后进入当前 turn，并在当前 provider stream / tool boundary 后的同一 turn continuation 中送达；不是 Responses WebSocket 专属控制事件。
- Agent evaluation：性能评估必须模拟真实监督验收循环。Evaluator 同时负责裁判和监督，基于 oracle evidence 判断是否完成；未完成时给 Worker 发送结构化 intervention，并把 intervention 类型、通道和 assistance score 计入评分。
- Provider public API：provider 由用户在 agent / app 创建时显式传入 `providers: [createClaudeCodeProvider(...), createCodexProvider(...)]`；`ProviderDefinition`、`ProviderRegistry` 和 secret-bearing config roundtrip 不应成为用户面对的装配概念。详细设计见 `docs/provider-public-api-plan.md`。
- Rust `coding-agent`：todo、ref expansion、shell。
- `vercel-labs/just-bash`：Bash Engine 实现基线（维护完整 fork，见 §7）。

相对 Rust 蓝本的有意取舍：

- 删除 `replay_from`。transcript 因此变为纯追加（retry 截断最后一轮除外），简化 mutation 面与审计面。
- 不引入 MCP / Skills / 子 agent。能力全部通过 Bash Environment 的命令表达。
- 不做权限 / 审批系统。第一版追求最小但长期稳定的 agent runtime，而不是照搬完整 IDE/GUI agent 产品面。
- 先做 `Host` 抽象与 `LocalHost`，远程 / 容器留待以后。

产品目标是让一个最小 agent 能稳定完成长任务：会话状态、模型可见上下文、tool call/result、shell 控制面、compaction 与 context cache 都必须可靠。参考 agent-gui、Codex、pi agent、opencode 时，只吸收这条核心稳定性链路上的设计与测试要求；权限、分享、revert、审批、项目管理等产品能力不作为当前缺口。

## 2. 术语

| 术语 | 含义 |
|---|---|
| Agent Loop | 通用 session runtime，不感知 coding / fs / git / 业务命令；只处理固定 shell tools 的调用结果和 transcript。 |
| Agent Harness | 描述某类 agent 的 prompt、状态、命令、Host、引用解析和生命周期；不替换 shell runtime。 |
| Bash Environment | Demi 固定的可审计 bash-like 执行环境，承载所有 agent 的 shell session 工具。 |
| Host | Bash Environment 之下的系统后端抽象，提供 `defaultCwd`、`fs`、`process` 和 `store`。 |
| shell session | 一个长期存活的 shell 状态（cwd/env/进程表），跨多次 `shell_exec` 复用。 |
| 注册命令 | agent 专属命令，TS 实现，经 command registry 调度。 |
| portable command | fork 提供的常见 Unix 命令 TS 实现，经 command registry 调度并读写 `Host.fs`。 |
| host external command | 真正依赖 host runtime 的外部命令，经 `Host.process.spawn` 执行。 |

## 3. 架构

demi 是纯 library，不含任何 UI 实现。最外层是**协议层**：一组稳定的事件流与命令接口，供任何壳子（REPL / Electron / Web / 服务进程）接入。壳子不是 demi 的一部分。

```text
壳子（REPL / Electron / Web / ...）   ← 不属于 demi
  ↕  协议层（事件流 + 命令接口）
Agent Loop
  ↕
Agent Harness
  ↕
Bash Environment
  ↕
Host
```

核心原则：

- 协议层是 demi 的对外边界：壳子只通过协议层定义的事件和命令与 agent 交互，不接触内部实现。协议层不假设壳子是什么。
- Agent Loop 是通用的，不感知 coding、文件系统、git 或具体业务命令；它只看到固定 shell tools 的工具调用结果，不解释 shell 进程细节。
- Agent Harness 定义某类 agent 的 prompt、状态、Host、注册命令、引用解析和策略。
- 每个 agent 对模型只暴露标准基础工具组：`shell_exec` / `shell_status` / `shell_write` / `shell_abort` / `yield`。
- Bash Environment 是所有 agent 固定使用的核心机制，不是 coding agent 的可替换依赖；它执行注册命令和 just-bash portable commands，真实外部命令才 spawn 到 Host。
- Host 提供 `defaultCwd`、`fs`、`process` 与 `store`：文件系统访问走 `Host.fs`，真实外部进程走 `Host.process.spawn`，运行状态走 `Host.store`（见 §8）。`defaultCwd` 只是默认 cwd / path resolve helper，不是访问边界。

### 3.1 不可回退的架构守则

这些原则是后续实现的硬约束，不能为了短期通过测试绕开：

1. **默认入口先天多平台**：`@demi/shell` 和 `@demi/coding-agent` 的默认入口必须保持 browser-safe / runtime-neutral。不得用"另开 browser 入口"来掩盖根入口静态依赖 `node:*`、`Buffer`、`process.env` 或 Node-only adapter 的问题。
2. **Node 能力显式隔离**：`LocalHost`、stdio transport、真实 Claude CLI provider 等 Node-only 能力只能从明确的 adapter / provider 子路径或包内导入；测试可以使用 Node API，但运行时根入口不能被测试便利性污染。
3. **Host 是唯一系统访问边界**：bash runner、coding editor、reference resolver、agent runtime storage 都只能通过 `Host.fs` / `Host.process` / `Host.store` 访问系统能力。除显式 Host adapter 外，不直接读写本机 fs，不直接 spawn 本机进程，不把 `node:path` 当成运行时依赖。
4. **defaultCwd 不是权限边界**：`Host.defaultCwd` 只表示默认 cwd 和相对路径解析基准，不表示 workspace root、sandbox root 或可访问路径集合。文件访问能否越过 defaultCwd 由 Host backend policy 决定；项目级路径限制必须作为显式 policy 建模，不能偷塞进 Host 基础语义。
5. **just-bash 是实现基线，不是零散复制来源**：`packages/just-bash` 是 `wspl/just-bash` fork 的 git submodule；demi 消费其中的 `just-bash` workspace package。fork 的改造边界是 bash engine 的可扩展性，不承载 Demi 的 AgentServer/provider 逻辑；demi 仓库内不得再维护第二套 just-bash 源码副本。
6. **不整段回退系统 shell**：parser 或 runner 不支持的语法要明确拒绝，不能把整段 script 交给系统 shell 执行。否则 registered command、audit、session state、Host 抽象都会失效。
7. **状态类 builtin 必须在 runner 内维护**：`cd` / `export` / `unset` / `read` / `local` / `return` / `source` / `shift` / `pushd` / `popd` / `dirs` / `jobs` / `wait` / `exit` / loop control 等会改变 shell session 的行为不能交给 host external command。
8. **portable command 复用 fork，不在 Demi 重造**：`cat` / `ls` / `grep` / `sed` / `awk` / `jq` / `find` / `tee` / `cp` / `mv` / `touch` 等 fork 已实现的命令应通过 just-bash command registry 运行，并读写 `Host.fs`。Demi 不再复制实现，也不把这些命令退回本机 coreutils。只有真正依赖 host runtime 的命令才走 `Host.process.spawn`。
9. **注册命令不可被 shell function 遮蔽**：`editor` / `todo` 这类注册命令是 agent 能力边界，普通函数定义不能覆盖它们；`command` builtin 也不能把状态 builtin 强行改成外部命令。`command` 执行路径应跳过 shell function，但仍保留状态 builtin、注册命令、portable command 和 host external command 的正常边界。
10. **Provider config 不进协议**：AgentClient / Web browser-visible frame 只携带 `providerId` 和 `ModelSelection`；provider 的 `baseUrl`、API key、headers、envPrefix、transport 注入和测试 fake transport 都只能留在创建 public provider 的用户侧闭包里，由 AgentServer 私有解析 provider id 后创建 live runtime。
11. **Manifest 不虚增层依赖**：package manifest 必须反映真实源码依赖，不能让平台无关包通过声明依赖暗中耦合 provider、Node-only adapter 或真实 CLI provider。
12. **命令说明单一来源**：注册命令的 prompt、`<command> prompt` 输出、system prompt 中的命令说明必须由同一份 `CommandSpec` 渲染，不手写多份说明。
13. **AgentClient action Promise 按 action 收敛**：`phase` 是 session 级广播，不是 per-command ack。非 send action 必须按本地发出的 action 顺序把每个 active→idle phase 周期分配给一个已接受 action；`send` 帧必须携带 client-generated `messageId`，queued item mutation 用同一个 id 解析、重排或清空对应的 pending send。多个 queued `send` 不能因为监听到同一个 phase 周期而一起 resolve。
14. **测试与文档跟着约束走**：新增架构约束时先写进方案文档，再用测试、扫描或门禁覆盖；不能只靠聊天上下文或临时记忆。
15. **观测边界不改变 shell 可见语义**：长命令跨 `running` / `shell_status` / `shell_abort` / `yield` 收敛时，重定向、fd duplicate、fd close、`!` 等 shell 语义必须保持一致；本该写入文件的 stdout/stderr 不能因为中途 yield 或中断泄漏到 tool output。
16. **AgentSession 只被 AgentServer 消费**：非测试运行时代码只有 `AgentServer` 可以直接实例化 `AgentSession`；壳子、provider 和业务 harness 只能经 `AgentClient` 或类型契约交互。
17. **新增原则先入文档再实现**：新增架构原则先记录到本方案文档，再继续实现；实现后用测试、扫描或门禁验证。
18. **Shell 控制面显式区分 status、内容和 write**：模型面对的基础工具必须顺滑且不含隐式状态魔法：`shell_exec` 启动命令并返回 `commandId`；`shell_status` 只读取命令状态、计时和 artifact 路径，不返回 stdout/stderr 正文；输出内容通过 `/@/commands/<commandId>/...` 只读虚拟文件，用 `tail` / `grep` / `sed` / `awk` 等 shell 文本命令读取；`shell_write` 只写入非空 stdin；`yield` 是 agent-level 单次 delayed wakeup，终止当前 turn，并在 turn 完成后等待指定时间再唤醒：session idle 时启动内部 wakeup turn，session active 时作为内部 steer 插入当前 turn，绝不进入 queue；进程安静不等于需要输入；主动终止前台命令只能通过 `shell_abort` 显式表达，不应默认当作任务失败污染模型上下文。
19. **长进程优先走受控前台**：需要观测和停止的长进程（如 dev server、watch、preview）应作为 foreground command 运行，由 `shell_exec` / `yield wakeup` / `shell_status` 观测、由 `shell_abort` 停止；不要用 `cmd &` 后再 `pkill` / `killall` 按进程名清理。后台 job 只用于明确需要在 shell session 生命周期内持续保留、且后续通过 jobs/wait 或 session dispose 管理的进程。
20. **Bash Environment 不是 Harness 注入项**：Agent Harness 只能定义 `Host`、注册命令、prompt、引用解析和生命周期；不得让用户传入自定义 `BashEnvironment` 或替换标准 agent 工具面。Demi 的差异化是统一、可审计、可长程运行的 shell session 机制，所有 agent 都走同一套 exec/status/write/abort/yield、audit、tool result 和 compaction 语义。
21. **模型目录属于 provider 能力**：REPL / AgentClient 不硬编码 provider 模型、默认模型、context window 或别名映射；上层只消费 provider catalog 暴露的 full model id 与能力元数据。Codex catalog 复用官方 Codex auth 直接请求 backend；Claude catalog 使用 `models.dev` 并按最低模型版本阈值过滤。详细设计见 `docs/provider-model-catalog-design.md`。
22. **Provider 装配属于用户创建边界**：用户创建 agent / app runtime 时传入 public provider 对象数组，例如 `providers: [createClaudeCodeProvider(...), createCodexProvider(...)]`。`ProviderDefinition`、`ProviderRegistry`、可序列化 provider config 解析器和 live provider runtime factory 都是内部机制；API key 和 secret-bearing provider options 只能留在创建 provider 的用户侧闭包里，不能经浏览器或 AgentClient frame 往返。详细设计见 `docs/provider-public-api-plan.md`。
23. **Abort 是可重复的分层收敛动作**：`abort()` 每次只取消当前最高优先级的可取消层，并返回本次 target 与 `canAbortAgain`。active provider/tool/turn、queued action 都优先于 pending `yield` wakeup；pending wakeup 是最低优先级，不能因为普通 active turn abort 被顺手清理。
24. **工具展示按具体标准工具分发**：`tool_call` 只是 transcript envelope；Web / REPL / 未来壳子遇到标准工具时必须按 `toolName` 分发到 `shell_exec` / `shell_status` / `shell_write` / `shell_abort` / `yield` 的具体展示，未知工具才使用 generic fallback。不得为展示引入新的 model 包；共享边界继续是 `@demi/core` 的 `Block` 与 `@demi/agent` 的 `ClientSessionEvent`。五个标准工具 input schema 都必须允许 `description`，作为具体用户可见状态/结果标题。`shell_exec` 的展开内容必须渲染命令和模型/用户可见的有序 terminal output stream，也就是 stdout/stderr 按到达顺序交错后的内容；不得只显示 stderr，也不得显示 status、shellId、commandId、offset 等协议字段。详细规范见 `docs/tool-rendering-spec.md`。

### 3.2 Shell + yield 控制面

传统做法给模型暴露 Read / Write / Edit / Grep / Shell / ... 等大量工具，会导致 Agent Loop 被工具协议污染，每个工具都要 provider glue、schema、协议层事件、测试，且业务工具会持续膨胀。demi 仍然避免业务工具膨胀，但 shell 控制面必须清晰可预测：模型应当轻松执行命令、接收输出、处理长任务和交互式 stdin，而不是被迫操作一套容易误用的低层状态机。

模型可见的五个工具是 `@demi/agent` 拥有的标准基础工具面。`@demi/shell` 提供 BashEnvironment、shell session、command record、artifact 和 Host-backed stream sink 等 runtime primitives；它不拥有模型可见 AgentTool，也不决定 `yield` 语义。

目标语义：

```text
shell_exec(script, description?, shellId?, yieldAfterMs)
  → 命令结束：返回 exited + exitCode + artifact 路径 + 自动预算 preview
  → 命令仍在跑：返回 running + shellId + commandId + artifact 路径 + 自动预算 preview
  → yieldAfterMs 必填，最大 10 分钟；不接受 timeoutMs，不隐式终止进程

shell_status(commandId, description?)
  → 只读取命令状态、runningMs、idleMs、bytes、artifact 路径；不等待、不写 stdin、不返回输出正文、不改变生命周期
  → running / exited / aborted 都可读取；已完成命令在 AgentSession 生命周期内可继续读取 artifact

shell_write(commandId, stdin, description?)
  → stdin 必须是非空字符串；写入前台进程
  → 轮询必须使用 shell_status，不允许空 input 兼容路径
  → 写入后立即返回一次 status snapshot

shell_abort(commandId, description?)
  → 主动终止前台进程；这是控制动作，不默认表示 agent 任务失败

yield(durationMs, description?)
  → 写入 terminal tool result，结束当前 provider continuation；不读写 shell
  → 当前 turn 完成后开始计时
  → 到点时 session idle：启动一个内部 wakeup turn
  → 到点时 session active：作为内部 steer 投递到当前 active turn，不能进入 queue
  → pending wakeup 是 abort 最低优先级；session close 直接清理
  → 单次 delayed wakeup，不支持 repeat / start_yield / stop_yield
```

业务能力通过命令表达：

```bash
cat src/main.ts
rg "createSession" packages/
editor edit src/main.ts --old "foo" --new "bar"
git status --short
```

`shell_status` 是唯一读取命令状态的工具，但不是读取输出内容的工具；模型不应因为重复看到 `running` 就盲目连续 status。长命令等待节奏由 `yield` 表达：`shell_exec → yield → wakeup turn 或 wakeup steer → shell_status → yield → wakeup turn 或 wakeup steer → shell_status`。需要看输出时，模型使用普通 shell 命令读取 `/@/commands/<commandId>/output.txt`、`stdout.txt` 或 `stderr.txt`，例如 `tail -n 80`、`grep -n ERROR`、`sed -n '200,260p'`。工具结果必须直接可读，包含 status、shellId、commandId、artifact 路径、exitCode 或 next action，不要求模型从一坨 JSON 里猜状态。

runner 不提供由 idle timeout 触发的 shell 输入需求状态。`shell_exec` 的 `yieldAfterMs` 到期时，如果进程还活着，就返回 `running + shellId + commandId + artifact paths`；模型用 `yield` 结束当前 turn 并安排稍后唤醒，在 wakeup turn 或 wakeup steer 中用 `shell_status` 读取状态，必要时再用 `tail` / `grep` / `awk` / `sed` 读取 `/@` artifact，或在明确知道命令正在等待具体输入时用 `shell_write` 写入非空 stdin。慢输出和超长输出都走这套机制：输出 chunk 到达不会直接唤醒模型，超长输出不会塞进 `shell_status`。demi 不保留空 input 轮询入口，避免把一个无效果的 stdin 写入伪装成控制动作，也避免把 `npm install --silent` 这类安静长命令误判成交互输入。

这组工具不是业务能力工具，只是 shell session 的控制面。agent core 保持极简，业务能力通过命令环境扩展。

## 4. Bash Environment

### 4.1 执行模型

```text
解析 script
  → 逐条执行命令
    → 注册命令：走 command registry 的 TS 实现
    → 状态类 builtin：由 engine 内部实现维护 session 状态（见 §7）
    → just-bash portable commands：走 fork command registry + Host.fs
    → 真实外部命令：走 Host.process.spawn
  → parser 不支持的语法：直接拒绝，不整段交给系统 shell
```

调度优先级：

1. Bash Engine 解析语法。
2. 注册命令走 command registry（如 `editor` / `todo`）。
3. 状态类 builtin（`cd` / `export` / `unset` / `read` / `local` / `return` / `source` / `shift` / `exit` 等）由 engine 内部实现，维护 shell session 的 cwd / env / 函数——这些不能交给 host external command，否则状态不连续（见 §4.7.1、§7）。
4. just-bash portable commands（如 `cat` / `ls` / `grep` / `sed` / `awk` / `jq` / `find` / `tee` / `mkdir` / `rm` 等）走 fork 自己的 TS command 实现，文件读写通过 `Host.fs` 暴露的虚拟/远端/本机文件系统。
5. 真实外部命令（如 `git` / `npm` / `node` / `bun` / `cargo` / `docker` 等）才走 `Host.process.spawn(command, args)`。
6. parser 不支持时返回错误。

非注册、非状态类命令必须区分"portable command"和"host external command"两层。portable command 不应依赖本机 `/bin` 或 GNU/BSD coreutils 差异；它们由 fork 实现并读写 `Host.fs`。host external command 才进入 Host policy，由本机、远端、容器或未来 virtual host 决定是否支持、如何执行。

host external command 第一版完全自由，不做 allowlist 或安全护栏。

实现证据与边界：fork 的 `src/commands/registry.ts` 已把 `cat` / `ls` / `mkdir` / `rm` / `grep` / `sed` / `awk` / `jq` / `find` / `tee` 等列为 portable command，具体命令通过 `CommandContext.fs` 读写文件。Demi 不重新实现 `cat file`，也不把它降级成 `Host.process.spawn("cat", ...)`；Demi 的职责是注册 fork portable command，并把 just-bash `IFileSystem` 适配到 `Host.fs`。

`source` / `.` 是状态类 builtin，不走 host external command。它在当前 shell session 中执行脚本，能修改 cwd/env/positional parameters；slashless 文件名先按当前 cwd、再按 `PATH` 经 `Host.fs` 查找，source 脚本内部的 `read` 等命令继承外层 input redirection / heredoc / here-string。

`set` 也是状态类 builtin，不允许落到 host external command。第一版至少维护 positional parameters（`set -- ...`）、`errexit`（`set -e/+e`、`set -o/+o errexit`）、`noglob`（`set -f/+f`、`set -o/+o noglob`）和 `noclobber`（`set -C/+C`、`set -o/+o noclobber`）；其他 shell option 不能伪装支持，必须明确报错。

`eval` 也是状态类 builtin，不允许落到 host external command。它把参数拼接后在当前 shell session 中重新解析执行，能保留 cwd/env/function 变化，并继承外层 input redirection；eval 内解析错误或 unsupported syntax 应返回非零结果，不能把内部异常冒泡成 agent runtime 错误。

`type` 是 introspection builtin，不允许落到 host external command。它必须按 runner 的真实调度顺序报告命令来源：shell builtin、注册命令、shell function、portable command、PATH 文件；`command type` 仍然走 builtin。

### 4.2 命名规则

- Unix 命令名保留给 portable command 或 host external command。
- agent 专属能力使用 command + subcommand，如 `editor create` / `editor edit` / `editor patch`。
- 注册命令不能复用 shell builtin、portable command 或已知 host external command 名。
- 注册命令优先于 host external fallback；命名要克制，只注册少数明确属于 agent 的命令。
- 需要 agent 状态、审计、事务或协议层语义的能力才注册成专属命令。

已有 Unix 命令能做的事不要注册成专属命令，直接用 shell command；由 Bash Environment 决定它走 fork portable command 还是 host external command：

| 用途 | 命令 |
|---|---|
| 读文件 | `cat` / `head` / `tail` / `sed -n` / `nl` |
| 列目录 | `ls` / `tree` / `find` |
| 搜索 | `rg` / `grep` / `find` |
| 文件信息 | `stat` / `file` / `wc` / `du` |
| 文件操作 | `mkdir` / `rm` / `cp` / `mv` / `touch` / `chmod` |
| 文本处理 | `sed` / `awk` / `cut` / `sort` / `uniq` / `tr` / `xargs` |
| 数据处理 | `jq` / `yq` / `sqlite3` / `xan` |

`git` 不做 wrapper，与 `npm` / `cargo` / `docker` 一样是 host external command。

### 4.3 注册命令规格

注册命令必须自带 prompt/help 生成能力，作用类似 tool schema：模型只看 system prompt 里的命令说明即可正确调用。

```ts
interface CommandSpec {
  name: string
  summary: string
  subcommands: CommandSubcommandSpec[]
}

interface CommandSubcommandSpec {
  name: string
  summary: string
  input?: CommandInputSpec
  positionals?: string[]
  stdinField?: string
  output?: CommandOutputSpec
  examples: string[]
  run(ctx: CommandRunContext): Promise<CommandRunResult>
}

type CommandInputSpec = Record<string, ZodType>

type ParsedCommandInput = {
  subcommand: string
  values: Record<string, unknown>
}

interface CommandOutputSpec {
  json?: ZodType  // JSON 模式下的 stdout schema；raw 模式不约束
}

interface CommandRunResult {
  exitCode: number
}

interface CommandRunContext {
  argv: string[]
  parsed: ParsedCommandInput
  stdin: CommandStdin
  env: Record<string, string>
  cwd: string
  io: CommandIO
  storage: CommandStorage
}
```

**input 用扁平字段表，不包 `z.object`。** shell 命令本质是扁平 argv 序列：

```ts
// 正确
input: {
  path: z.string().describe('Target file path'),
  old: z.string().describe('Exact text to replace'),
  new: z.string().describe('Replacement text'),
}

// 错误
input: z.object({ path: z.string() })
```

Bash Engine 根据 `CommandSpec` 把扁平 argv 解析成 `parsed.values`，handler 不自己解析字符串。

argv 映射规则：

- `positionals` 声明字段顺序，如 `['path']`。
- 不在 `positionals` 里的字段映射为长参数，如 `--old` / `--new`。
- `stdinField` 声明哪个字段从 stdin / heredoc 读取。
- 字段类型与描述来自对应 zod field。
- `z.array(...)` 表示可重复参数。
- `z.boolean()` 表示 boolean flag。

示例：

```ts
const editor: CommandSpec = {
  name: 'editor',
  summary: 'Create, edit, and patch files.',
  subcommands: [
    {
      name: 'create',
      summary: 'Create a new file. Fails if the file exists.',
      input: {
        path: z.string().describe('Target file path'),
        content: z.string().describe('File content, usually passed via heredoc.'),
      },
      positionals: ['path'],
      stdinField: 'content',
      examples: [
        "editor create src/foo.ts <<'EOF'\nexport const foo = 1\nEOF",
      ],
      run: async ({ parsed, stdin, env, storage }) => {
        // parsed.values.path
        // parsed.values.content
      },
    },
  ],
}
```

### 4.4 输出格式

注册命令子命令接受标准 boolean 参数 `--json`：

- 传 `--json`：输出 machine-readable JSON，结构由命令定义，写入 stdout，按 `output.json` 校验。
- 不传 `--json`（默认）：输出 human-readable raw text。
- 两种模式互斥，不允许混用。

`--json` 是注册命令协议的一部分，由 Bash Engine 从 argv 识别并在 prompt 中说明。普通 shell command 不强制遵守（它们有自己的 `--json`，如 `rg --json`、`git --json`）。

### 4.5 命令 prompt 渲染

prompt 文本由 `CommandSpec` 自动生成：

```ts
renderCommandPrompt(editor)
```

`<command> prompt` 子命令调用同一 renderer。约定：

- 每个注册命令组必须实现 `<command> prompt` 子命令，只输出帮助文本，不改文件或状态。
- `<command> prompt` 输出与 `renderCommandPrompt(spec)` 同源。
- Agent Harness 构造 system prompt 时，从 command registry 收集 `CommandSpec` 渲染后注入。
- 模型无需先调用 `<command> prompt` 即可使用命令；该子命令主要用于调试与人工查看。
- 不允许 prompt 里手写一份说明、实现里再维护另一份。
- renderer 只依赖命令配置与 agent 类型，不依赖 session state / transcript / provider turn。

命令 prompt 至少包含：用途、子命令列表、参数与 stdin/heredoc 约定、成功/失败输出格式、是否修改文件或状态、2-3 个可复制示例。

```bash
editor prompt
editor create src/foo.ts <<'EOF'
...
EOF
editor edit src/foo.ts --old "foo" --new "bar"
editor patch <<'PATCH'
--- a/src/foo.ts
+++ b/src/foo.ts
...
PATCH
```

### 4.6 命令执行隔离

注册命令不直接感知 AgentSession / transcript / provider turn / agent state，只通过 shell 执行上下文工作：argv / stdin / env / cwd / io / storage。

Bash Environment 在每个 shell session 注入：

```text
DEMI_SESSION_ID=<agent session id>
DEMI_SHELL_ID=<shell id>
```

命令需要隔离状态时，用 `DEMI_SESSION_ID` 作为隔离 key 的一部分；它指的是 agent session，不是 shell 控制句柄。如 `todo`：

```text
todo storage key = todos/${DEMI_SESSION_ID}.json
```

`CommandStorage` 是 `Host.store`（见 §8）的 agent-session-scoped 视图：Bash Environment 创建 `CommandRunContext` 时用 `DEMI_SESSION_ID` 包一层，命令看到的 key 自动带 agent session 前缀。命令作者只写 `storage.writeJson('todos.json', ...)`，不拼前缀。需要跨 agent session 共享的状态第一版不开放。

约束：

- 不把 session object / transcript / agent state 传给 `CommandRunContext`。
- agent session 隔离信息和 shell 控制句柄通过 `DEMI_*` env 注入。
- 持久化状态通过 `CommandStorage`，key 自带 agent session 隔离。
- 第一版强制注入 `DEMI_SESSION_ID` 与 `DEMI_SHELL_ID`。

### 4.7 shell session 与输出流

#### 4.7.1 状态连续性

一个 agent 内的多次 `shell_exec` 默认复用该 agent session 的长期 shell session：

- `cd packages/foo` 之后下一条 `cat package.json` 仍在 `packages/foo`。
- `export` / shell 函数 / `pushd`/`popd` / 后台 job 句柄跨 `shell_exec` 保留。
- session 持有自己的 cwd / env / 进程表。

`shellId` 指长期 shell session。`commandId` 指一次 foreground command。session 同时只跑一条前台命令；前台命令 running 时 `shell_exec` 返回 `running + shellId + commandId`，后续 `shell_status` / `shell_write` / `shell_abort` 操作同一 command，直到其退出或被中止。

shell session 通常对一个 AgentSession 长期存活（跨多个 user turn），销毁时其 cwd / env / 后台进程一并释放。默认 `shell_exec` 不传 `shellId` 时使用该 AgentSession 的默认 shell；如果默认 shell 正在跑 foreground command，则创建辅助 shell，方便模型在 dev server 运行时执行一次性检查命令。显式传入忙碌 `shellId` 时不自动创建辅助 shell，避免模型误以为命令跑在指定 shell 状态里。

#### 4.7.2 长命令

`shell_exec` 不是同步的一次性请求。长命令需可观测，否则测试 / 构建 / 安装依赖等任务会变成黑盒。

```text
shell_exec(script, yieldAfterMs)
  → 在当前 shell session 执行
  → runner 实时收集模型可见 stdout/stderr 到 command artifact 和 interleaved output artifact
  → yieldAfterMs 到点且命令未结束：返回 running + shellId + commandId
  → 后续 shell_status / shell_write / shell_abort 操作同一 command
```

Agent Loop 只在明确边界点恢复：command exit、`shell_exec` 的 `yieldAfterMs` 到点、`shell_status` / `shell_write` / `shell_abort` tool result 返回、`yield` terminal result 返回、pending yield wakeup 到点、provider stream 自身完成或出错。每次恢复给模型的是程序化 command snapshot 或内部 wakeup 输入，不是模型生成的摘要。模型不应每个输出 chunk 都被唤醒；输出内容不通过 `shell_status` 分页读取，而是通过 `/@/commands/<commandId>/...` artifact 文件读取。

`yieldAfterMs` 到期且进程仍在运行时，默认状态是 `running`，即使 stdout/stderr 没有新增内容。安静长命令由 `runningMs` / `idleMs` 暴露观测信息，是否通过 `yield` 安排后续 wakeup 再继续 `shell_status`、用 `tail` / `grep` / `awk` 读取 artifact、改用非静默命令、或主动 abort 由模型决定；runner 不能把 idle timeout 自动升级成输入需求状态。pending yield wakeup 到点时，若 session idle 就启动内部 wakeup turn；若 session 已经 active，就复用 steer delivery 插入当前 turn，不能进入普通 queue。

dev server / watch / preview 这类命令如果只是为了冒烟验证，应作为 foreground command 启动，并设置较短 `yieldAfterMs` 观察启动日志；验证完成后调用 `shell_abort(commandId)`。不要把这类命令放到后台再用 `pkill -f`、`killall` 或类似进程名匹配方式回收，因为这绕过 shell 控制面，可能误杀同名进程，也让审计链路变差。

Demi Web 的浏览器 GUI 只能通过动态 dev server 验收。`@demi/web` 的 Bun server 是
WebSocket/API 后端，不得服务 Vite `dist`、静态 bundle、preview 页面或 production fallback；
日常开发和验收必须打开 Vite dev server 页面，并让它连接后端 `/control` 与 `/agent`。

`running`、`exited`、`aborted` 都是观测边界，不是 shell 语义边界。若前台命令带输出重定向，runner 必须按 sink 映射过滤可见 stdout/stderr，并在后续 `shell_status` 或 `shell_abort` 收敛时通过 Host 完成文件 sink 写入；不能把被重定向的内容暴露给模型，也不能绕过 Host 直接使用本机 fs。最终实现应支持 file redirection sink 流式写入，让长命令运行中的目标文件也可通过 Host.fs 可见。

```ts
type StreamArtifact = {
  path: string
  bytes: number
}

type ShellCommandSnapshot = {
  status: 'running' | 'exited' | 'aborted'
  shellId: string
  commandId: string
  stdout: StreamArtifact
  stderr: StreamArtifact
  output: StreamArtifact
  runningMs: number
  idleMs: number
  exitCode?: number
  preview?: {
    text: string
    budgetTokens: number
    truncated: boolean
  }
  audit?: BashAuditEvent[]
}
```

artifact 路径挂载在只读虚拟文件系统：

```text
/@/commands/<commandId>/stdout.txt
/@/commands/<commandId>/stderr.txt
/@/commands/<commandId>/output.txt
/@/commands/<commandId>/meta.json
```

`output.txt` 是 stdout/stderr 按到达顺序交错后的终端 transcript。`/@` 路径由 just-bash `IFileSystem`
overlay 提供，只允许 fork portable commands 读取；真实 host external process 不能 fallback 读取内存态
`/@` 路径。`/@` 命名空间只读、生命周期跟随 AgentSession，不写入任务 workspace。

`@demi/agent` 根据当前 `Model.contextWindow` 自动决定 tool result preview 预算：未知或 `<= 300k`
tokens 使用约 `1k` tokens，`<= 1M` 使用约 `10k` tokens，更大窗口 hard cap 到约 `20k`
tokens。模型不能通过 `maxOutputBytes` 控制预算；需要更多内容时必须用 shell 文本命令读取 artifact。

卡死感知字段：`runningMs`（已跑时长）、`idleMs`（距上次输出时长）、`stdout.bytes` / `stderr.bytes` / `output.bytes`（是否有新增或异常大输出）。

观测节奏（参考 Codex）：

- 长命令的首次 `shell_exec` 通常使用 `yieldAfterMs = 10_000`。
- `idleMs` 仅作观测，不自动 kill。
- `yield` 是 agent-level 单次 delayed wakeup；不做 repeat / heartbeat。
- 连续多次 `running` 且无输出时，由 agent 决定继续等 / abort / 询问用户。

## 5. Agent Loop

Agent Loop 是纯通用 runtime。

**职责：** transcript、queue、steer、send / retry / resume、abort / cancellation、provider turn、tool invocation、tool continuation 状态、compaction、extension state snapshot、mutation guard、事件产出（供 AgentServer 消费）。

**不负责：** fs 语义、git 语义、业务领域能力、terminal、process session 内部状态、file explorer、project/worktree 管理。Shell/Bash 是统一执行基座，但进程、cwd/env、stdin/stdout、audit 和 command lifecycle 的细节由 Bash Environment 负责，Agent Loop 只处理工具调用和 transcript。

Snapshot restore 是显式 API，不靠外部手工拼 constructor 参数。`AgentSession.fromSnapshot` 必须校验 `harnessName`，并从 snapshot 恢复 transcript、agent state、cwd 和 model；恢复后的 runtime 从 `idle` 开始。snapshot 中持久化的 active `phase` 和 `queue` 只作为最后观测状态保存，不在进程重启后自动重放，因为原 action promise、provider stream 和 tool invocation 已经不存在。

Demi 第一版只暴露标准基础工具组。Agent Loop 对 shell command continuation 只保存 opaque token，不理解其背后是不是本地进程或远端进程：

```ts
type ToolContinuation = {
  toolCallId: string
  // 对 Agent Loop 是 opaque token。Bash Environment 内部用 shellId + commandId 定位。
  // Agent Loop 不解释，只透传给标准工具实现。
  shellId: string
  commandId: string
  status: 'running'
}
```

真正的 pid、stdout/stderr artifact、idleMs、stdin、abort 都留在 Bash Environment 内部。术语统一：shell 工具入参/出参的 `shellId` / `commandId` 与 `ToolContinuation` 里的字段是同一个值，不另起 `continuationId`。

### 5.1 接口

```ts
interface AgentSessionParams<State> {
  provider: AgentProvider
  model: ModelSelection
  cwd: string
  transcript?: Transcript
  runtime: AgentHarnessRuntime<State> // AgentServer 从 AgentHarness 解析出的内部 hooks
}

interface AgentHarnessRuntime<State> {
  harnessName: string
  initialState(): State
  systemPrompt(ctx: AgentPromptContext<State>): string
  preamble?(ctx: AgentPromptContext<State>): string | null
  tools(ctx: AgentToolContext<State>): AgentTool[] // 固定 shell tools，由 AgentServer 创建
  resolveReferences?(ctx: AgentReferenceResolveContext<State>, content: UserContentBlock[]): Promise<UserContentBlock[]>
  dispose?(): Promise<void>
}

type AbortTarget =
  | 'active_provider_stream'
  | 'active_tool_invocation'
  | 'active_turn'
  | 'queued_action'
  | 'pending_yield_wakeup'

type AbortResult =
  | { aborted: true; target: AbortTarget; canAbortAgain: boolean }
  | { aborted: false; target: null; canAbortAgain: false }

interface AgentSession<State> {
  send(content: UserContentBlock[], options?: { id?: string }): Promise<void>
  dequeueMessage(id: string): boolean
  sendQueuedMessage(id: string): boolean
  steerQueuedMessage(id: string): Promise<boolean>
  clearMessageQueue(): number
  steer(content: UserContentBlock[]): Promise<void>
  retry(): Promise<void>
  resume(): Promise<void>
  compact(): Promise<void>
  abort(): Promise<AbortResult>
  waitUntilDone(): Promise<void>

  transcript(): Transcript
  state(): State
  phase(): SessionPhase
  queuedMessages(): QueuedMessage[]
}

```

相对 Rust 蓝本删除 `replayFrom`。mutation guard 保留，用于外部 transcript 编辑（`reserve_mutation`）与 compact 期间的并发保护。

`send` 在 busy 时仍表示下一 turn 队列；`steer` 表示追加到当前 active turn。两者是显式调用方选择，runtime 不允许把 rejected steer 自动转成 queued send，也不允许用 abort/resume 模拟 steer。

`abort` 是可重复执行的分层收敛动作，不是 session close。每次调用按优先级取消一层：active provider stream / active tool invocation / active turn 收敛状态、queued action、pending `yield` wakeup。返回的 `AbortResult.canAbortAgain` 告诉调用方是否还存在下一层可取消目标。pending `yield` wakeup 是最后优先级：普通 active turn abort 不清理它；只有当前面所有 active 和 queued 状态都收敛后，再次 abort 才取消 pending wakeup。关闭 session / dispose 则直接清理全部 pending wakeup。

Queue item management 是最终 runtime 契约的一部分，不是 UI 本地状态。
`dequeueMessage` 只删除仍在队列里的 send，并 resolve 对应 pending action，但不执行；
`sendQueuedMessage` 把仍在队列里的 send 移到下一 turn 队首；`steerQueuedMessage` 原子地把
仍在队列里的 send 取出并转成当前 active turn 的 steer，失败时恢复原队列位置，不能在 UI 里用
“先 dequeue 再 steer”拼出来；`clearMessageQueue` 删除所有仍在队列里的 send，且不影响当前
active turn。`AgentClient` 以 `dequeueMessage`、`sendQueuedMessage`、`steerQueuedMessage`、
`clearMessageQueue` 暴露同名能力；浏览器 UI 必须直接调用这些方法，不能自己发明本地 queue
状态。浏览器 UI 的空输入提交是 queue 快捷操作：当 composer 为空且 queue 非空时，running
状态把最后一个可见 queued message 转成 active steer；非 running 状态提交该 queued message。

### 5.2 Transcript 与 Compaction

Transcript 是全量无损的 append-only 日志。

- block 数组：`User` / `Resume` / `Thinking` / `Text` / `ToolCall` / `Response` / `Error` / `Abort` / `CompactionBoundary` / `CompactionMarker` / `ExtensionStateSnapshot`。
- block 只追加、不修改、不删除（retry 截断最后一轮是 Rust 蓝本既有行为，保留）。
- 工具调用的完整 output（含 shell 的 ShellCommandSnapshot 原文）落在 `ToolCall.output`，不丢。
- transcript 持久化、可重放、可作审计依据。

Compaction 是长任务能力的核心路径，不是附属优化。它必须保证 agent 在上下文接近上限时仍能继续工作，同时不破坏审计历史、不切坏 tool pair、不重复执行已完成工具、不留下失败中间态。

Compaction **不删 transcript**，只控制"发给模型的部分"：

- token 用量接近模型上限时（runtime 自动判断，非模型决定），runtime 在 transcript 上找 cut point，让模型对 cut point 之前的内容生成 summary。
- 在 cut point 处插入 `CompactionBoundary`（带 summary），追加 `CompactionMarker`（记录被压缩 token 数）。
- 老 block 原样保留在 transcript。
- 下次构造 `InferenceRequest` 时从最后一个 `CompactionBoundary` 起取 blocks（`effective_transcript`），boundary 之前的内容被 summary 替代，不再整段发给模型。
- 协议层暴露的永远是全量 transcript；只有发给 provider 的 items 被压缩。

触发时机（照搬 Rust 蓝本）：

- **preflight**：`send` 前若 effective transcript 估算 token 接近上限，先 compact 再跑。
- **auto-recover**：provider 返回 usage 接近上限时，取消当前请求、跑完 pending tools、compact、push resume turn 继续。
- **手动**：调用方经协议层主动调 `compact()`。

cut point 选择：从末尾往前累加 token，优先落在完整 turn 边界；不能切断 `ToolCall -> output`，不能让 provider 看到孤立 tool_use/tool_result。如果单个 turn 本身超过 `KEEP_RECENT_TOKENS`，允许 split-turn compact，但必须把被切掉的 turn prefix 写进 summary，recent part 继续作为原始 blocks replay。找不到安全 cut point 时不 compact。

## 6. Agent Harness

```ts
interface AgentHarnessContext<State> {
  state: State
  cwd: string
}

interface AgentHarness<State> {
  name: string
  initialState(): State
  host(ctx: AgentHarnessContext<State>): Host
  commands?(ctx: AgentHarnessContext<State>): CommandSpec[]
  systemPrompt(ctx: AgentPromptContext<State>): string
  preamble?(ctx: AgentPromptContext<State>): string | null
  lifecycle?(event: AgentLifecycleEvent<State>): Promise<void>
  resolveReferences?(ctx: AgentReferenceResolveContext<State>, content: UserContentBlock[]): Promise<UserContentBlock[]>
}
```

Agent Harness 是某类 agent 的装配层，不是运行中的会话。它定义 prompt、初始状态、Host、注册命令、引用解析和生命周期；运行中的 transcript、phase、queue、compact、retry/resume 仍属于 `AgentSession`。

`AgentHarness` 依赖 `Host` / `CommandSpec`，因此它属于 agent assembly 边界。`AgentServer` 持有一个已选好的 harness，创建 Bash Environment 和固定 shell tools，再把解析后的内部 `AgentHarnessRuntime` hooks 传给 `AgentSession`。

Harness 的可变面必须克制：

- `host()` 选择命令在哪个系统后端和默认 cwd 上执行，例如本地、远程或容器。
- `commands()` 注册 agent 专属命令（如 `editor` / `todo`）。
- `systemPrompt()` 生成基础行为说明；注册命令的调用说明由 command registry 自动注入，不在 prompt 里手写。
- `resolveReferences()` 只负责把用户引用扩展成模型可见内容，文件读取必须经 `Host.fs`。
- `lifecycle()` 做 harness 自己的清理；Bash Environment 的 shell session 清理由 AgentServer 统一处理。

Harness 不暴露这些可替换点：

- 不提供 `tools()`；标准基础工具组固定由 `@demi/agent` 创建并暴露给模型。
- 不接收 `BashEnvironment`；AgentServer 按 harness 的 `Host + commands` 创建标准 Bash Environment。
- 不允许替换 agent tool result 格式、exec/status/write/abort/yield 语义、audit 事件或 command registry 调度。

AgentServer 打开 session 时的装配顺序：

```text
open(provider, cwd)
  → 使用 AgentServer 绑定的 AgentHarness
  → harness.host(...) + harness.commands(...)
  → 创建 BashEnvironment(host, commands, sessionId)
  → CommandStorage 从 Host.store 派生 agent-session-scoped 视图
  → 创建固定 agent tools: shell_exec / shell_status / shell_write / shell_abort / yield
  → 创建 AgentSession(provider, model, cwd, harness runtime hooks)
```

第一版只内置 coding harness。`editor` / `todo` 都是 coding harness 的注册命令。

## 7. Bash Engine 实现来源

Bash Engine 以 `vercel-labs/just-bash` 为基线。just-bash 是一个完整的 bash 实现（约 13 万行，含 parser、interpreter、几十个 builtin、完整 expansion）。demi 通过 `packages/just-bash` submodule 维护 `wspl/just-bash` fork，并消费 fork 里的 `just-bash` package；不再把单个 helper / builtin / parser 文件复制进 `@demi/shell`。已有 just-bash 语义时，改动必须进入 fork 或 fork 暴露的稳定 API，禁止在 demi runner 里重新推导替代实现。

fork 不是把上游源码当内部路径直接 import，也不是把 Demi runtime 塞进 just-bash。fork 的职责是让 bash engine 更容易被 Demi 使用：稳定 parser/AST 导出、长期 shell state、可插拔命令调度、Host IO 边界、输出 sink、审计和 job hooks。`@demi/shell` 是最终的 Demi shell runtime package，负责 Host contract、registered command、BashEnvironment、command artifact、ShellCommandSnapshot、exec/status/write/abort primitives 和 agent tool 集成所需的 shell 侧能力；模型可见 AgentTool 由 `@demi/agent` 拥有。`@demi/shell` 只依赖 fork package 的稳定 API，不越过 package 边界 import fork 内部文件。

需要明确的一点：just-bash 的状态类 builtin（`cd` / `export` / `unset` / `read` / `local` / `return` / `source` / `shift` / `exit` 等）必须保留在 engine 内部，不能改成 host external command。原因是 shell session 的状态连续性（§4.7.1）依赖它们：系统 `cd` 不会影响后续 `shell_exec`，只有 engine 内的 `cd` 改变 session 的 cwd。即使这些名字不是 demi registered command，也必须由 engine 内部实现维护 session 状态。

just-bash fork 不是只有 parser/interpreter。它的 `src/commands/` 已经包含大量 portable command（`cat` / `ls` / `grep` / `sed` / `awk` / `jq` / `find` / `tee` / `mkdir` / `rm` 等），这些 command 通过 `CommandContext.fs` 读写 `IFileSystem`。Demi 必须复用这部分实现，让 portable command 在本机 fs、remote fs、container fs、memfs/virtual fs 上保持一致。`git` / `npm` / `node` / `bun` / `cargo` / `docker` 这类真正依赖 host runtime 的命令才走 `Host.process.spawn`。

fork 策略：

1. fork `vercel-labs/just-bash`，记录 upstream repo、commit、license，并保留 Apache-2.0 license / NOTICE / 修改说明。
2. 把 `wspl/just-bash` 作为 git submodule 放在 `packages/just-bash`；demi 根 workspace 显式包含 `packages/just-bash/packages/just-bash`，并以包名 `just-bash` 依赖它。
3. fork 包保留 parser / interpreter / builtin / expansion / upstream tests 的完整基线。demi 需要的 parser bugfix、builtin 行为、IFS/read/set 等 bash 语义修正直接改 fork，不在 `@demi/shell` 或其他包复制实现。
4. fork 默认入口必须 browser-safe / runtime-neutral。Node-only filesystem、worker、CLI、真实进程或重型命令能力只能从显式 Node-only 子路径导出；用静态入口扫描和 browser bundle smoke test 兜住。
5. fork 对 `@demi/shell` 暴露稳定 engine API：长期 shell state、portable command registry、Host spawn/filesystem boundary、registered command hook、output sink/redirection hook、audit hook、foreground/background job control、parser/AST exports。
6. `@demi/shell` 保留 CommandRegistry、Host contract、BashEnvironment、ShellCommandSnapshot、command artifact、exec/status/write/abort primitives 和 agent tool 集成所需的 shell 侧能力；模型可见 AgentTool 由 `@demi/agent` 拥有；bash 语义下沉到 fork engine，Demi runtime concerns 不进入 fork。
7. 上游关系在 `wspl/just-bash` fork 仓库内通过 git remote、commit、license 和 patch history 表达。demi 只保留 submodule 指针，不保留 `upstream/`、`vendor/` 或另一份 just-bash 源码。

`just-bash` 是实现来源，不是模型面对的工具名。模型 / 协议层 / audit event 不暴露 `just-bash` 作为概念；架构文档可以区分 portable command 与 host external command，以保证实现边界清楚。本方案不要求完全兼容真实系统 bash，但需具备 just-bash 已支持的能力；超过 parser 能力的语法直接拒绝，不整段交给系统 shell。

验收标准：

- fork package 上游核心测试通过。
- fork 默认入口 browser-safe，不能静态依赖 `node:*`、`Buffer`、`process.env`、Node-only adapter 或 worker-only chunk。
- 已支持的 parser/interpreter/builtin 行为保持 parity，并通过 demi shell session 回归测试。
- registered command 可接入同一执行模型。
- portable command 正确通过 `Host.fs` 读写文件；真实外部命令正确通过 `Host.process.spawn` 运行并收集 stdout/stderr/exit。
- shell session 状态（cwd/env/函数）跨 `shell_exec` 连续。

### 7.1 fork engine 集成执行设计

本小节是把 §7 原则落成代码的具体设计。fork 已有完整可运行的 interpreter（parser/AST/compound command/状态 builtin/expansion/arithmetic/glob/redirection/pipeline/function/local scope/set-shopt 全套），`@demi/shell` 不得再复制这些语义。

#### fork 与 demi 的根本差异（4 个扩展点）

fork 的设计目标是"纯 TS 模拟 bash + 虚拟 fs + 内置命令"，demi 的目标是"驱动 Host 的 agent shell session"。Host 不是只有进程执行能力，也必须暴露系统级文件访问和运行状态存储能力。差异决定 fork 需要扩展的地方：

1. **文件后端是 `Host.fs`**：fork portable commands、redirection、glob、`source`、`$(< file)`、file tests 都应通过 fork `IFileSystem`，而 `@demi/shell` 的 `IFileSystem` 适配器必须委托到 `Host.fs`，不能再用 `cat`/`tee`/`test` 等 host processes 模拟文件系统。
2. **真实外部命令才走 `Host.process.spawn`**：fork 的 command registry 先处理 portable commands 和 registered commands；只有没有 portable/registered/function/builtin 命中的 host runtime command 才进入 `hostSpawn(command, args, { cwd, env, stdin })`。
3. **长期 shell session，不是每次 exec 隔离**：fork 的 `Bash.exec()` 每次 copy state；demi 需要跨 exec 状态连续。**扩展点**：demi 绕过 `Bash` 类，直接用 `Interpreter` 类并自己持有 `InterpreterState`——`Interpreter` 原地修改传入的 state，demi 跨 exec 复用同一个 state 对象。
4. **长命令可观测（exec/status/yield/abort）**：fork 的 `exec` 同步 await 到结束；demi 需要让 `shell_exec` 跑到初始观察边界后返回 `running`，并允许后续 `shell_status` 读取 command record。**不进 fork**——这是 demi runtime concern，通过 `hostSpawn` 的 Promise 挂起实现（见下方执行模型）。portable commands 通常同步完成，不参与 foreground 控制。
5. **ShellCommandSnapshot + audit + DEMI_SESSION_ID / DEMI_SHELL_ID**：fork 的 `ExecResult` 只有 stdout/stderr/exitCode。**不进 fork**——audit、command snapshot 和 `DEMI_*` env 注入是 demi runtime concern。

#### 执行模型

```text
BashEnvironment.exec(script)
  ├── parse(script) → AST                          [fork parser]
  ├── new Interpreter({ hostSpawn, fs, commands, ... }, state) [fork interpreter, state 跨 exec 复用]
  ├── interpreter.executeScript(ast)               [fork 驱动 bash 语义]
  │     ├── builtin (cd/export/...)                [fork builtins/]
  │     ├── compound (if/for/while/case/...)       [fork control-flow.ts]
  │     ├── expansion ($VAR/$(...)/$((...))/glob)  [fork expansion/]
  │     ├── registered command (editor/todo)       [fork CommandRegistry → demi CommandSpec 适配]
  │     ├── portable command (cat/ls/grep/...)     [fork CommandRegistry → Host.fs]
  │     └── host external command (git/npm/node/...)
  │           └── hostSpawn(name, args, opts)      [demi 注入的真实进程钩子]
  │                 ├── Host.process.spawn(name, args)
  │                 ├── 启动 foreground 进程 + pump 模型可见 stdout/stderr 到 command artifact
  │                 └── waitForForeground()        [demi 的边界点逻辑]
  │                       ├── exit → resolve ExecResult
  │                       └── yieldAfterMs → 不 resolve，保留 pendingExec + command record
  └── 收集 ShellCommandSnapshot + audit → agent tool result
```

#### 长命令挂起机制

fork interpreter 是"跑完整个 script 返回 ExecResult"的同步模型。demi 需要长命令跑到初始观察边界点时**暂停 script 执行**，返回 `running + commandId`。之后 `shell_status` 不续接 interpreter，也不等待；它只读取 command record 的状态、计时、字节计数和 artifact 路径，不读取输出正文。command exit 时 pending `executeScript` 自然 settle，并更新 command record。

`BashEnvironment.exec()` 不直接 await `interpreter.executeScript`，而是 race 它和边界点：

```ts
async exec(input: ShellExecInput): Promise<ShellCommandSnapshot> {
  const session = this.resolveTargetSession(input)
  if (session.foreground) throw new BusyShellError(session.foreground.commandId)
  const command = this.createCommandRecord(session, input)

  const interpreter = this.createInterpreter(session, input)
  const ast = parse(input.script)
  const execPromise = interpreter.executeScript(ast)

  const boundary = this.waitForBoundary(session, input)
  const outcome = await Promise.race([
    execPromise.then(r => ({ kind: 'done' as const, r })),
    boundary.promise,
  ])

  if (outcome.kind === 'done') return this.buildExitedSnapshot(command, outcome.r)

  // 边界点触发：foreground 进程已在 session.foreground，execPromise 仍 pending
  command.pendingExec = execPromise
  return this.buildRunningSnapshot(command, outcome)
}
```

`shell_write(commandId)` 写入 foreground stdin 后立即返回一次 `shell_status` snapshot；`shell_abort(commandId)` 终止 foreground process group 并返回最终 snapshot。等待节奏由 agent 工具 `yield(durationMs)` 表达，不由 `@demi/shell` 提供 wait 工具。

#### 注册命令适配

fork 的 `Command` 接口：`{ name, execute(args, ctx) }`，`ctx` 是 `CommandContext`。demi 的 `CommandSpec` 有 `subcommands`/`positionals`/`stdinField`/`--json`/`renderCommandPrompt`/`storage`。

适配器 `commandSpecToForkCommand(shellSession, spec, storage)` 把 demi `CommandSpec` 转成 fork `Command`：把 fork `CommandContext`（`env` Map、`stdin` ByteString）适配成 demi `CommandRunContext`（`env` Record、`stdin.text`），注入 `CommandStorage`/`io`/`DEMI_SESSION_ID`/`DEMI_SHELL_ID`。每次创建 shell 时，fork `CommandRegistry` 必须同时包含：

- fork portable commands：由 fork 稳定 API 创建，覆盖 `cat` / `ls` / `grep` / `sed` / `awk` / `jq` / `find` / `tee` / `mkdir` / `rm` 等 virtual-fs-aware command。
- demi registered commands：由 `CommandSpec` 适配而来，如 `editor` / `todo`。

注册前必须拒绝 shell special builtin、portable command 和明确保留的 host external command 名，避免模型可见命令语义混乱。合并后的调度顺序只需要保证 portable/registered commands 都在 `hostSpawn` 前命中，不靠覆盖已有命令表达 agent 专属能力。

**注册命令调度路径的已知缺口已修正，但语义要保留**：fork 的 `executeExternalCommand` 在有 `hostSpawn` 时先检查 `ctx.commands.get(commandName)`；如果命中，直接调 `cmd.execute()`，不走 PATH。这让 fork portable commands 和 demi registered commands 都能在 `hostSpawn` 之前运行。后续 Host.fs 重构不能回退到“有 hostSpawn 就把 `cat`/`ls` 交给真实系统”的旧路径。

**fork portable commands 必须注册**：fork 的 `src/commands/` 里有 `cat`/`ls`/`grep`/`sed`/`awk`/`jq` 等几十个内置 command 实现，`Bash` 构造时通过 `createLazyCommands` 注册到 `CommandRegistry`。demi 仍然可以不用 `Bash` 类、直接构造 `Interpreter`，但必须通过 fork 的稳定 API 创建 portable command registry，并和 demi registered commands 合并。若当前 fork API 没有公开合适的 portable command factory，先在 fork 暴露稳定导出，再在 `@demi/shell` 使用；不得从 fork 内部路径临时 import registry 文件。

#### CommandContext 字段映射

fork `CommandContext` 与 demi `CommandRunContext` 的字段差异：

| fork `CommandContext` | demi `CommandRunContext` | 适配 |
|---|---|---|
| `env: Map<string, string>` | `env: Record<string, string>` | `mapToRecord(ctx.env)` |
| `stdin: ByteString`（opaque 字节） | `stdin: { text: string }` | `decodeBytesToUtf8(ctx.stdin)` |
| `cwd: string` | `cwd: string` | 直传 |
| `fs: IFileSystem` | `Host.fs` / command-scoped file helper | registered commands needing files must use Host.fs, not shelling out to `cat`/`tee` |
| `execute(args, ctx)` | `run(ctx)`（args 在 `ctx.argv`） | 适配器把 `args` 放进 `ctx.argv` |
| 返回 `ExecResult { stdout, stderr, exitCode }` | 返回 `CommandRunResult { exitCode, metadata? }` | 适配器把 demi 命令的 `io.stdout`/`io.stderr` 输出收集成 `ExecResult.stdout`/`stderr` |

#### ExecutionLimits

fork 有 `ExecutionLimits`（maxCommandCount/maxOutputSize/maxCallDepth/maxLoopIterations/maxHeredocSize）。demi 自己维护 command artifact 和按模型上下文窗口决定的 tool result preview 预算，不需要 fork 的 `maxOutputSize`。构造 `Interpreter` 时传入高限值或禁用 fork 的 output limit，避免 fork 在 demi 的边界点之前就抛 `ExecutionLimitError`。

#### fork 测试基础设施

fork 用 **vitest**（不是 bun test），从 `packages/just-bash/packages/just-bash/` 跑 `npx vitest run`。demi 根 `package.json` 的 `test:just-bash-core` 脚本只用 bun test 跑 3 个 parser 测试文件，不覆盖 fork interpreter 全套。Step C 验证 fork 改动时需手动跑 `npx vitest run src/interpreter/`。

#### 不用 fork 的 Bash 类

demi **直接用 `Interpreter` 类**，不用 `Bash` 类。`Bash.exec()` 每次 copy state（`Bash.ts:634`），是"每次 exec 一个新 shell"模型；demi 需要跨 exec 状态连续，所以自己持有 `InterpreterState` 并传给 `new Interpreter(opts, state)`——`Interpreter` 原地修改传入的 state（`this.ctx.state.lastExitCode = exitCode` 等直接赋值）。

#### Host.fs 与 just-bash IFileSystem

fork 的 redirection、portable command、glob、`source`、`$(< file)` 和 file tests 都走 `IFileSystem`。demi 的 `IFileSystem` 适配器必须委托到 `Host.fs`。

**不手写 redirection，不用 shell 命令模拟文件系统**。`@demi/shell` 实现 `HostBackedFileSystem implements IFileSystem`，把 fork 的 fs 操作（readFile/readFileBuffer/writeFile/appendFile/exists/stat/lstat/readdir/mkdir/rm/realpath 等）路由到 `Host.fs`。这样 fork 的完整 redirection 逻辑（noclobber、fd duplicate、fd close、process substitution、here-doc、here-string）和 portable command 逻辑全部复用，demi 不需要重新实现。`HostBackedFileSystem` 是 Demi runtime concern，放在 `@demi/shell`，不进 fork。

`Host.fs` 是 Host 的核心能力，不是 `Host.process.spawn` 的语法糖。LocalHost 用 Node `fs/promises` 实现；RemoteHost 用远端文件 API 实现；ContainerHost 用容器 fs API 实现；MemHost 用内存 fs 实现。`Host.process.spawn` 对 virtual host 是另一项能力：可以不支持，可以只支持 fork portable command 以外的受控进程，也可以把真实进程和 Host.fs 挂载/同步后执行。

#### audit 收集

- **system-command audit**：在 `hostSpawn` 实现里，每次 spawn 后记录 `{ kind: 'system-command', name, args, cwd, exitCode }`。
- **registered-command audit**：在 `commandSpecToForkCommand` 的 `execute` 里，记录 `{ kind: 'registered-command', name, args, exitCode }`。

audit 收集到 per-command accumulator，附在 `ShellCommandSnapshot` 的完成结果上。

#### DEMI_SESSION_ID / DEMI_SHELL_ID 注入

`createShell()` 时把 `DEMI_SESSION_ID`（agent session id）和 `DEMI_SHELL_ID`（shell 控制句柄）写入 `InterpreterState.env`（Map）并加入 `exportedVars` 集合，fork interpreter 通过 `buildExportedEnv` 把它传给所有命令。

#### 文件结构

```text
packages/shell/src/
  index.ts              re-export
  command.ts            demi CommandSpec 协议（不变）
  host.ts               Host contract：defaultCwd + fs + process + store
  tools.ts              shell runtime primitives（exec/status/write/abort），不拥有 AgentTool
  storage.ts            CommandStorage（不变）
  bytes.ts              UTF-8 helpers（不变）
  environment.ts        重写：BashEnvironment + ShellSession + hostSpawn + 边界点
  host-fs.ts            HostBackedFileSystem，IFileSystem 路由到 Host.fs
  script-parser.ts      删除
packages/host-local/src/
  local-host.ts         Node LocalHost adapter：defaultCwd + fs + process + store
```

#### 接口兼容

`BashEnvironment`/`ShellCommandSnapshot`/`BashAuditEvent`/`CommandMetadataRecord` 是 shell runtime 公开接口。模型可见 AgentTool schema/result 由 `@demi/agent` 拥有；`tools.ts`/`command.ts`/`coding-harness.ts`/`agent/server.ts`/测试随 harness API 和标准工具面迁移。

### 7.2 前置事实

- demi 目录最初只有方案文档；当前已按 §14 的 Step 0-6 落地 monorepo 和本地实现。
- bun 1.3.11 可用。
- just-bash（vercel-labs/just-bash）约 13 万行，含完整 parser/interpreter/builtin/expansion，**以 `wspl/just-bash` fork submodule 为实现基线；demi 只消费 fork 里的 `just-bash` package，不再新增零散复制片段**。
- 方案原本漏了 provider 层——agent 强依赖 `AgentProvider.run`，必须先有 provider 包（哪怕只是 stub）。
- Rust 蓝本的 `InferenceRequest` 是**纯 items 数组模型**（`items: InferenceItem[]` + systemPrompt + cwd + tools + thinking + cancel）。claude-code provider 内部把 items 转成 Claude CLI 的 stream-json stdin messages，并用 MCP control_request bridge 驱动工具；这些都是 provider 实现细节，不进 `InferenceRequest` 接口。
- shell session 状态连续性要求状态类 builtin（cd/export/unset/read/local/return/source/shift/pushd/popd/dirs/jobs/wait/exit）、shell function 定义、`$?` 上一条命令状态和后台 job 句柄在 engine/session 内维护，不能交给 host external command。
- AgentServer/AgentClient 是唯一运行入口：本地 JS 调用方使用 `server.client()`，跨进程或网络调用方使用 transport；不存在绕过 AgentServer 直连 AgentSession 的另一套 API。

### 7.3 Fork 可行性调研结论

- 可行。上游 just-bash 是公开 TypeScript monorepo，`packages/just-bash` 是发布包；上游根仓库已有 build / typecheck / unit / comparison / dist smoke test 脚本，适合作为 workspace fork 接入。
- license 可行。上游包为 Apache-2.0；fork 时需要保留 LICENSE / NOTICE / upstream commit，并给修改文件保留修改说明。
- 技术上需要 fork，而不是直接依赖 npm 包。上游 `Bash.exec()` 默认每次隔离 shell state；demi 需要长期 shell session、Host 边界、registered command、audit、job 和 output hooks。fork 的改造应最小化并聚焦这些 engine 扩展点；`@demi/shell` 负责 shell runtime primitives、ShellCommandSnapshot 和 agent tool 集成所需的 shell 侧能力，模型可见 AgentTool 由 `@demi/agent` 拥有。
- browser-safe 不能直接信任上游。上游有 `just-bash/browser` 包含 `process` 的公开 issue，demi 的 fork 默认入口必须自己用静态闭包扫描和 browser bundle smoke test 验证。
- 风险主要在维护成本和依赖体量。fork 会带来上游同步成本，但比继续复制 parser/helper/builtin 片段更可控；后续实现策略是稳定 fork package `just-bash` 的 API，再把 `@demi/shell` 里仍手写的 bash 语义逐步下沉到 fork engine。

## 8. Host

Host 是 agent 运行所依赖的系统级后端，不是 workspace sandbox。它把默认 cwd、文件系统、进程执行和运行状态存储放在同一抽象下，让 LocalHost / RemoteHost / ContainerHost / MemHost 都能用一致的能力面描述。

```ts
interface Host {
  defaultCwd: string
  fs: HostFileSystem
  process: HostProcess
  store: HostStore
}

interface HostFileSystem {
  readFile(path: string, options?: { cwd?: string }): Promise<Uint8Array>
  writeFile(path: string, data: Uint8Array, options?: { cwd?: string; createParents?: boolean }): Promise<void>
  appendFile(path: string, data: Uint8Array, options?: { cwd?: string; createParents?: boolean }): Promise<void>
  exists(path: string, options?: { cwd?: string }): Promise<boolean>
  stat(path: string, options?: { cwd?: string }): Promise<HostFileStat>
  lstat(path: string, options?: { cwd?: string }): Promise<HostFileStat>
  readdir(path: string, options?: { cwd?: string }): Promise<HostDirEntry[]>
  mkdir(path: string, options?: { cwd?: string; recursive?: boolean }): Promise<void>
  rm(path: string, options?: { cwd?: string; recursive?: boolean; force?: boolean }): Promise<void>
  realpath(path: string, options?: { cwd?: string }): Promise<string>
}

interface HostProcess {
  spawn(params: HostSpawnParams): Promise<HostSpawnHandle>
}

interface HostSpawnHandle {
  stdout: AsyncIterable<Uint8Array>
  stderr: AsyncIterable<Uint8Array>
  writeStdin(data: Uint8Array): Promise<void>
  closeStdin(): Promise<void>
  kill(): Promise<void>
  wait(): Promise<{ exitCode: number | null; signal?: string }>
}

interface HostStore {
  readJson<T>(key: string): Promise<T | null>
  writeJson<T>(key: string, value: T): Promise<void>
  delete(key: string): Promise<void>
  list(prefix: string): Promise<string[]>
}
```

核心语义：

- `Host.defaultCwd`：默认工作目录和相对路径解析 helper。它不是 workspace root、sandbox root、权限边界或可访问路径集合。
- `Host.fs`：系统级文件访问 facet。editor、reference resolver、just-bash portable commands、redirection、glob、`source`、`$(< file)`、file tests 都必须走这里。它可以是真实 fs、remote fs、container fs、memfs 或 policy-restricted fs。
- `Host.process`：系统级进程执行 facet。`git` / `npm` / `node` / `bun` / `cargo` / `docker` 等 host runtime command 走这里，并受 Host backend policy 控制。
- `Host.store`：系统级 agent runtime state facet。todo、command JSON state、session snapshot、session metadata 等状态都走这里。它和 `fs/process` 属于同一个 Host 后端，不再保留旁路 store 边界。

文件操作不能再通过 `cat` / `tee` / `test` / `ls` 这类 shell command 间接模拟。那会把文件系统能力错误绑定到系统 coreutils，实现 memfs/virtual fs 时无法成立，也会引入 GNU/BSD 差异、`--` 参数安全、二进制编码等问题。

`LocalHost` 是 `@demi/host-local` 的 Node adapter：`defaultCwd` 是启动默认目录，`fs` 走 Node `fs/promises`，`process` 走 Node `child_process`，`store` 走 Node fs 下的本地状态目录。远程 / 容器 / memfs 以后换 `Host` 实现，`BashEnvironment` 和 `AgentSession` 不变。Agent Loop 不感知这些差异。

默认运行时模块必须只依赖 `Host` 接口，不依赖 `LocalHost`。使用方需要本地系统能力时显式依赖 `@demi/host-local`。这个隔离同样适用于 stdio transport 和真实 Claude CLI provider。

访问控制不是 `defaultCwd` 的职责。如果 coding agent 需要限制某些操作只改项目目录，必须作为显式 policy 或 command-level guard 设计；不能把 Host 基础语义写成“defaultCwd 外不可访问”。Host backend 可以按自己的策略拒绝某些 fs/process/store 操作，但这个策略不是由 `defaultCwd` 自动推导。

Bash Environment 内部自由决定：portable command 如何经 `Host.fs` 执行、真实外部 command 如何经 `Host.process.spawn` 执行、cwd/defaultCwd 处理、`DEMI_SESSION_ID` 注入、shell session 保存、stdout/stderr buffer 截断、audit 记录。这些都不进 Agent Loop 公共接口。

shell session 内部结构：

```ts
type ShellSession = {
  id: string
  cwd: string
  env: Record<string, string>
  startedAt: number
  lastOutputAt: number
  stdoutBuffer: RingBuffer
  stderrBuffer: RingBuffer
  totalStdoutBytes: number
  totalStderrBytes: number
  foreground?: { startedAt: number; spawn: HostSpawnHandle }  // 当前前台命令
  exitState?: { exitCode: number | null; signal?: string }
}
```

本地实现里 `foreground.spawn` 是 `LocalHost.process` 产生的真实子进程句柄；换 `RemoteHost` 后即远端句柄；换 `MemHost` 时可以选择不支持真实外部进程，或只支持受控 shim。Agent Loop 不关心。

`DEMI_SESSION_ID` 在 AgentSession 创建时分配并记入 session metadata，Bash Environment 创建 shell session 时注入 env；`DEMI_SHELL_ID` 仅表示当前 shell 控制句柄。注册命令经 `CommandStorage`（`Host.store` 的 agent-session-scoped 视图，自动按 `DEMI_SESSION_ID` 隔离 key，见 §4.6）访问 HostStore。

## 9. 审计

shell session 工具调用必须产出 audit events：

```ts
type BashAuditEvent =
  | { kind: 'registered-command'; name: string; args: string[]; exitCode: number }
  | { kind: 'system-command'; name: string; args: string[]; cwd: string; exitCode: number }
```

| kind | 质量 | 说明 |
|---|---|---|
| registered-command | 最高 | 可解释 agent 专属语义与状态变化。 |
| system-command | 中 | 可审计 argv / cwd / stdout / stderr / exit code。 |

shell session 工具不记录 fs diff。普通 shell command 修改文件不提供通用自动撤销。第一版不设计通用编辑日志；如以后 `editor` 需要类似能力，只能作为 `editor` 自己的实现细节。

## 10. Coding Agent

Coding Agent 是一个 Agent Harness，由 `@demi/coding-agent` 导出。它不是新的 runtime，也不拥有可替换 shell；它只是给统一 Agent Loop + Bash Environment 装上 coding 任务需要的 prompt、Host、注册命令、引用解析和生命周期。

- todos 不放进 CodingState，是 `todo` 注册命令自己的状态，按 `DEMI_SESSION_ID` 隔离持久化；状态为 `pending` / `in_progress` / `done`。
- Coding harness 注册少量 agent 专属命令，并让 AgentServer 用这些命令创建标准 Bash Environment。
- Coding harness 的主入口是 `createCodingAgentHarness({ host, commands?, referenceHost? })`。调用方只能选择 Host 和额外命令；不能传入自定义 `BashEnvironment`。

`@demi/coding-agent` 的公开导出保持克制：

- `createCodingAgentHarness`
- `createCodingCommandRegistry`
- `createEditorCommand`
- `createTodoCommand`
- `createFileReferenceResolver`
- `CodingAgentHarnessOptions` / `CodingState` 等必要类型

P0 命令：

```text
editor prompt
editor create
editor edit
editor patch
todo prompt
todo list
todo add
todo update
todo done
```

`prompt` 子命令只输出帮助文本，不改文件或状态。

`editor` 子命令语义：

- `editor create`：创建新文件，已存在则失败；内容从 stdin / heredoc 传入。
- `editor edit`：对已有文件做精确替换，匹配不到或匹配多处时失败。匹配多处时可用 `--occurrence <n>`（1-based）或 `--context <line>`（锚定行号附近最近匹配）消歧；仍无法消歧则失败，返回所有匹配位置供模型调整。
- `editor patch`：应用多文件 patch。格式用 unified diff（`diff -u` / `git apply` 风格）——模型训练数据多见、工具链成熟、可校验；不用 OpenAI `*** Begin Patch` 私有格式。

直接使用 shell 命令时仍然只通过 Bash Environment 暴露给模型，不新增模型工具。命令按执行后端分两类：

- just-bash portable commands：fork command registry 已实现的 `cat` / `head` / `tail` / `ls` / `tree` / `find` / `rg` / `grep` / `sed` / `awk` / `jq` / `stat` / `file` / `wc` / `mkdir` / `rm` / `cp` / `mv` / `touch` / `tee` / `diff` 等，必须走 fork command 实现和 `Host.fs`，保证本机、远端、容器、memfs/virtual fs 语义一致。
- host external commands：`git` / `bun` / `npm` / `pnpm` / `yarn` / `node` / `python` / `cargo` / `docker` 等真正依赖 host runtime 的命令，才走 `Host.process.spawn`。这类能力由具体 Host adapter 决定是否支持。

## 11. 协议层

协议层是 demi 的对外边界，也是唯一通信方式。壳子（REPL / Electron / Web / 服务进程 / 本地 JS 调用方）只通过协议层与 agent 交互，不接触 Agent Loop / Bash Environment / Host 等内部实现。协议层不假设壳子是什么，也不含任何渲染逻辑。

**AgentServer 是唯一运行入口。** 不存在“本地直连绕过 server”的另一套 API。本地 JS 调用方使用 `server.client()` 拿到本地 `AgentClient`，跨进程或网络场景使用 transport 连接同一套 frame handler。transport 仍然存在，但它是 AgentServer/AgentClient 的通信适配层，不是独立包或独立公共分层。

协议层 = `ClientFrame` / `ServerFrame` 帧协议 + transport 抽象。客户端侧由 `AgentClient` 把帧还原成可编程视图；服务端侧由 `AgentServer` 把帧转成 `AgentSession` 调用。`AgentSessionHandle` 不作为对外 API 暴露，它是 `AgentServer` 内部持有 runtime 的句柄。

### 11.1 AgentServer 与 AgentClient

`AgentServer` 绑定一个已选好的 `AgentHarness`。app/REPL 在创建 server 前选择 coding agent 或其他 agent；`open` 帧只携带 `ProviderSelection` 和 cwd，不携带 harness 名，也没有 host 侧 harness registry。

```ts
const server = new AgentServer({
  agent: createCodingAgentHarness({ host }),
  providers: [createClaudeCodeProvider(...), createCodexProvider(...)],
  shell: { initialEnv: { PATH: process.env.PATH ?? '' } },
})

const client = server.client()
await client.open(providerSelection, cwd)
```

`server.client()` 表示同进程本地 client 视图，不是建立网络连接。需要跨进程或网络时，用 transport：

```ts
server.attachTransport(createStdioServerTransport(process.stdin, process.stdout))
const client = new AgentClient(createStdioClientTransport(stdout, stdin))
```

### 11.2 客户端视图

`AgentClient` 把帧还原成可编程视图，供壳子调用：

```ts
interface AgentClient {
  open(provider: ProviderSelection, cwd: string): Promise<void>
  send(content: UserContentBlock[]): Promise<void>
  steer(content: UserContentBlock[]): Promise<void>
  retry(): Promise<void>
  resume(): Promise<void>
  compact(): Promise<void>
  abort(): Promise<AbortResult>

  subscribe(listener: (event: SessionEvent) => void): () => void
  close(): Promise<void>
}

type SessionEvent =
  | { type: 'transcript_snapshot'; blocks: Block[] }
  | { type: 'transcript_patch'; patches: Patch[] }
  | { type: 'phase'; phase: SessionPhase }
  | { type: 'queue'; queue: QueuedMessage[] }
  | { type: 'steer_result'; steerId: string; status: 'accepted' | 'rejected'; reason?: string }
  | { type: 'tool_progress'; toolUseId: string; output: ToolResultContentBlock[] }
  | { type: 'shell_output'; shellId: string; commandId: string; snapshot: ShellCommandSnapshot }
  | { type: 'shell_write_result'; commandId: string; snapshot: ShellCommandSnapshot }
  | { type: 'audit'; events: BashAuditEvent[] }
  | { type: 'error'; message: string; code?: string }
```

事件携带程序化数据（block 数组、command snapshot、phase），不是渲染指令。壳子自己决定怎么显示。`transcript_snapshot` 首次/重连发全量，后续 `transcript_patch` 发增量，客户端无需自己合并。`shell_output` 透传 `ShellCommandSnapshot` 原文。协议层不生成摘要、不替壳子做展示决策。

### 11.3 壳子职责边界

壳子可以：展示 transcript、展示 shell tool call 与流式输出、展示 running command session、消费 command snapshot / delta、展示 audit events、展示 file diff。

壳子不应：直接读写 Host.fs / Host.store；感知 Bash Environment 是 local / remote；感知 host external command 如何 spawn；直接操作 Agent Loop 内部状态；绕过 AgentClient 直接持有 AgentSession。

## 12. AgentServer Transport

Transport 是 AgentServer 的通信适配层：定义 `ClientFrame` / `ServerFrame` 帧协议和 `AgentTransport` 抽象。所有消费者——本地 JS 或远程 Web / Electron——都走同一套帧协议，只是 transport 不同。

### 12.1 设计原则

**动作帧 + 状态帧，普通 turn 动作不做 per-command id 编排。** demi 是单 worker 串行：同一时刻只有一轮工作在跑，客户端发 `send` 时如果已有 run 正在执行，则进入 AgentSession queue；`queue` 帧公开等待中的 user turn。`retry` / `resume` / `compact` 这类会重写 transcript 或改变当前控制流的动作在 busy 时回 `rejected` 帧带命令名，客户端据此确认。

由于没有 per-command id，AgentClient 不能让每个 `send` / `retry` / `resume` / `compact` Promise 各自独立监听同一组 `phase` 广播后自行判断完成。client 必须维护本地 action FIFO：每次从 idle 进入 active phase 只认领一个等待中的 action，回到 idle 时只 resolve 这个 action；`rejected` 只 reject 尚未进入 active phase 的同名动作；`error` 优先 reject 当前 active action，无法关联到 active action 时再 reject 全部等待动作；`closed` 让全部等待动作收敛。

`steer` 是例外：它不是启动下一轮 phase 的普通 turn 动作，而是在 active turn 内即时接受或拒绝的控制动作。客户端必须给 `steer` 帧携带 `steerId`，服务端用 `steer_result` 按 id ack；多个 steer 不能共用 phase FIFO，也不能通过 `queue` 帧表示。
`steer_result: accepted` 只表示 session 已接收该 same-turn input；如果当前 delivery 点是下一次 provider continuation，真实 `steer` block 会等到最近的 provider/tool 边界后写入 transcript：provider stream 结束后立即 materialize，tool 执行中则在当前 tool result 写入后立即 materialize。GUI 壳子在 ack 和 transcript materialization 之间应保留本地 pending steer 呈现，直到收到对应 `steer` block 或当前 turn 结束。

**shellId 和 commandId 分别关联不同对象。** `shellId` 关联长期 shell session；`commandId` 关联一次 foreground command。输出、写 stdin 和 abort 都应优先按 `commandId` 关联，避免 dev server 运行时的辅助 shell 和原 shell 状态混淆。

**一连接一 session。** 一个 transport 连接对应一个 agent session。`open` 在握手时完成（传 provider 配置 + cwd），之后普通 agent 动作都作用在这个 session 上，帧里不带 agent session id；shell 输出帧同时带 `shellId` 和 `commandId`，shell 写入/中止动作按 `commandId` 指向具体命令。多 agent session 场景由多连接解决。

**transcript 用 snapshot + patch。** 首次发 `transcript_snapshot`（全量），后续发 `transcript_patch`（patch 增量），避免长 transcript 全量重发。无论 transport 是 in-process 还是跨网络，都用同一套 snapshot/patch 策略。

**shell 输出独立成帧。** `shell_output` 高频大体积，和 transcript 控制流分开，客户端按 shellId 订阅/取消订阅，背压可控。

### 12.2 帧类型

```ts
// client → server：对 session 做动作，不带 id
type ClientFrame =
  | { type: 'open'; provider: ProviderSelection; cwd: string }
  | { type: 'send'; content: UserContentBlock[] }
  | { type: 'abort' }
  | { type: 'retry' }
  | { type: 'resume' }
  | { type: 'compact' }
  | { type: 'shell_write'; commandId: string; stdin: string }
  | { type: 'close' }

// server → client：session 状态变化
type ServerFrame =
  | { type: 'opened' }
  | { type: 'abort_result'; result: AbortResult }
  | { type: 'rejected'; command: string; reason: string }
  | { type: 'transcript_snapshot'; blocks: Block[] }
  | { type: 'transcript_patch'; patches: Patch[] }
  | { type: 'phase'; phase: SessionPhase }
  | { type: 'queue'; queue: QueuedMessage[] }
  | { type: 'tool_progress'; toolUseId: string; output: ToolResultContentBlock[] }
  | { type: 'shell_output'; shellId: string; commandId: string; snapshot: ShellCommandSnapshot }
  | { type: 'shell_write_result'; commandId: string; snapshot: ShellCommandSnapshot }
  | { type: 'audit'; events: BashAuditEvent[] }
  | { type: 'error'; message: string; code?: string }
  | { type: 'closed' }
```

### 12.3 Transport

```ts
interface AgentTransport<SendFrame, ReceiveFrame> {
  send(frame: SendFrame): void
  onFrame(handler: (frame: ReceiveFrame) => void): () => void
  close(): void
}
```

Transport 只负责收发帧，不解释帧语义。第一版实现：

- **in-process**：`server.client()` 内部使用，帧直接在内存里传递（不经序列化），是零成本直通。
- **stdio**：CLI 壳子场景，帧以换行分隔的 JSON（NDJSON），从 `@demi/agent/stdio` 显式子路径导入。
- **WebSocket**：Web / Electron 场景，帧为 JSON 文本消息。

in-process transport 让“本地 client”和“跨进程 client”走完全相同的代码路径（同一套帧协议、同一个 AgentClient/AgentServer handler），只是 transport 实现不同。序列化第一版用 JSON（可读、调试友好、Web 原生）；in-process 不序列化。msgpack 等二进制编码以后按需加。

### 12.4 关联模型小结

| 关联需求 | 靠什么 |
|---|---|
| 命令是否被接受 | `send` 可通过 `queue` 帧进入等待队列；其他动作被拒时发 `rejected` 帧（带命令名），无 rejected 即被接受 |
| 这轮工作的事件边界 | transcript 的 User/Resume block（turn 边界，天然在 block 里）+ phase 事件（running→idle） |
| shell 输入/输出配对 | shellId |
| retry 截断了哪些 | transcript_patch 里包含删除的 block |
| 这轮是否跨了多个 turn | transcript 里 CompactionBoundary + Resume block，仍属同一次 send |

协议层不暴露 turn id 作为独立字段——它在 transcript 的 block 里，客户端需要时自己读。

### 12.5 Server handler 与 client

**AgentServer**：持有一个 `AgentHarness` 和 provider registry。每个 transport 连接打开一个 session，server 用 harness 提供的 `Host + commands` 创建标准 Bash Environment，再创建 `AgentSession`。server 把 `ClientFrame` 转成 session 调用，把 session 的 `SessionEvent` 转成 `ServerFrame` 发出。`open` 帧创建 session，`close` 帧销毁。transcript 变更产出 snapshot/patch。AgentServer 是 `AgentSession` 的唯一运行时消费者。

**AgentClient**：持有 `AgentClientTransport`，把方法调用转成 `ClientFrame` 发出，把收到的 `ServerFrame` 还原成 `SessionEvent` 推给 `subscribe` 监听器。客户端维护本地 transcript 视图（apply snapshot/patch），壳子基于此视图渲染。

```text
壳子 → AgentClient → ClientFrame → [transport] → ServerFrame → AgentServer → AgentSession
                                          ↑
AgentSession 事件 → AgentServer → ServerFrame → [transport] → ClientFrame → AgentClient → 壳子 subscribe
```

两端对称：server 和 client 都只懂帧协议，都不直接接触对方的内部对象。本地 client 时 transport 是 in-process，两端在同一进程但仍是这套帧路径。

## 13. 包结构

demi 是纯 agent 库，不含 frontend 实现 / module 层。Agent runtime 和 server/client/transport 都由 `packages/agent` 提供；transport 是 AgentServer 的通信适配层，不作为独立领域包存在。

包职责、依赖方向、公共入口和 adapter 归属是核心架构约束，权威规则见 `docs/package-boundaries.md`。本节只保留包职责概览；如果职责概览和包边界文档不一致，先修正边界文档或代码，再继续实现。

```text
packages/core/            基础类型：Block、Transcript、UserContentBlock、ModelSelection、
                          TokenUsage 等跨包共享类型（agent 与 provider 都依赖）
packages/provider/        AgentProvider 接口、InferenceRequest/InferenceItem、
                          ProviderEvent、public Provider、auth 能力
packages/agent/           通用 AgentSession（Agent Loop）、AgentServer、AgentClient、
                          ClientFrame/ServerFrame、AgentTransport、in-process + WebSocket
                          transport、显式 stdio adapter、transcript snapshot/patch
packages/just-bash/       forked Bash Engine：parser/interpreter/builtin/expansion 基线、
                          browser-safe 默认入口、Host/command/output/audit/job 扩展 API、
                          fork core tests
packages/shell/            Host contract、BashEnvironment、shell runtime primitives、
                          ShellSession、ShellCommandSnapshot、command registry、command prompt、
                          audit、portable command routing、host external spawn、
                          HostStore-scoped CommandStorage
packages/host-local/       本机 Node adapter：LocalHost(defaultCwd/fs/process/store)
packages/coding-agent/    Coding agent harness、prompt、coding commands、todo
packages/provider-claude-code/  Claude Code provider：驱动系统 claude code CLI
packages/provider-codex/  Codex provider：复用官方 Codex auth，驱动 Responses transport
packages/provider-openai-api/  OpenAI API provider：官方 Responses endpoint/env 映射；
                          显式 wireApi 选项支持 Chat Completions 兼容 endpoint
packages/provider-anthropic-api/  Anthropic API provider：Messages endpoint/env 映射
packages/repl/             本地验收壳子和 composition root
```

`packages/core` 与 `packages/provider` 是底层依赖：core 放跨包共享类型，provider 定义 AgentSession 调用模型的标准接口。`InferenceRequest` 是纯 items 数组模型（`items: InferenceItem[]` + systemPrompt + cwd + tools + thinking + cancel），与 Rust 蓝本一致；provider 实现内部如何把 items 喂给模型（直连 API、stdin stream-json、或 provider 自己支持的 resume 机制）是 provider 自己的事，不进接口。

`packages/agent` 依赖 core、provider 和 shell。AgentSession 是内部运行时；AgentServer 是唯一运行时消费者，负责把 AgentHarness + `Provider[]` + shell options 组装成 session。AgentClient 和 transport frame handler 也在同一包内，避免额外的公共通信分层。

`packages/provider-claude-code` 的实现机制：直接 spawn 系统 `claude` CLI（`--print --output-format stream-json --input-format stream-json`），stdin/stdout JSON 行通信，手写 MCP JSON-RPC bridge 处理 tool 调用，`DISABLE_AUTO_COMPACT: 1`（用 demi 自己的 compaction）。**不依赖 `@anthropic-ai/claude-agent-sdk`**——Rust 蓝本完全自实现，demi 照搬。CLI 的 stream event 映射成 `ProviderEvent`。这套机制照搬 Rust `provider-claude-code` crate。

没有 provider 包，AgentSession 无法跑（§5 的 send/retry/resume/compact 都依赖 `AgentProvider.run`）。provider 包先于 agent 实现，测试使用 stub provider（返回脚本化事件流）让 agent 行为规格能独立验证；真实 provider 以 leaf package 方式落在 concrete provider 包中，产品入口通过 `providers: [...]` 直接装配。

`AgentHarness` 类型属于 agent assembly 边界。对外只传入一个已选好的 harness 给 AgentServer，不暴露 harness registry，不暴露可替换 Bash Environment。

## 14. 重写计划

本文是把方案落成代码的执行路径。demi 是全新 bun monorepo，纯 agent 库（不含 UI 实现，通过协议层供壳子接入），从零开始，不搬 agent-gui 代码。每个 Step 都有明确的产物和验收，做完一个再做下一个。

### 14.1 拆包顺序

按依赖自底向上建，先建被依赖的：

```text
Step 0  core                    ← Block/Transcript/ModelSelection 等基础类型
Step 1  provider                ← AgentProvider 接口 + InferenceRequest/ProviderEvent + stub
Step 2  agent                   ← AgentSession（依赖 core + provider）
Step 3  just-bash + shell       ← forked just-bash engine + Host + shell session 工具
Step 4  coding-agent            ← coding agent harness + editor/todo
Step 5  agent server            ← AgentServer + AgentClient + transport（同包能力）
Step 6  provider-claude-code    ← 驱动 claude code CLI 的真实 provider
```

`AgentHarness` 的类型放在 agent runtime 边界。demi 是纯库，不做 module-agent / frontend 实现，但提供 AgentServer/AgentClient/transport 供壳子接入。

### 14.2 测试执行方式

测试不是单独最后补，而是跟每个 Step 的验收绑定：Step 2 用 StubProvider 复刻 AgentSession 行为规格，Step 3 覆盖 bash runtime / Host / shell tools，Step 4 覆盖 editor / todo / coding marathon，Step 5 覆盖 AgentClient/AgentServer/transport，Step 6 覆盖 claude-code provider 的 CLI 参数、stream-json 输入转换、事件映射和 MCP control_request。完整测试模块、缺口和优先级以 `docs/testing.md` 为准，本小节只记录执行入口。

日常门禁：

```bash
bun run typecheck
bun run test
bun run test:just-bash-core
```

分层测试入口：

```bash
# agent session/transcript 行为
bun test packages/agent/src/__tests__

# bash runtime、注册命令、Host、shell tools
bun test packages/shell/src/__tests__

# coding agent、editor/todo、stub provider 端到端 marathon
bun test packages/coding-agent/src/__tests__

# AgentServer/AgentClient 帧协议、in-process/stdio/websocket transport
bun test packages/agent/src/__tests__

# claude-code provider 的本地适配层测试，不依赖真实 Claude 请求
bun test packages/provider-claude-code/src/__tests__

# 平台默认入口静态闭包扫描
bun test packages/core/src/__tests__/platform-entrypoints.test.ts
```

真实 Claude CLI e2e 是外部环境相关测试，默认跳过；只有本机安装并完成可用认证后再手动开启：

```bash
DEMI_CLAUDE_CODE_E2E=1 bun test packages/provider-claude-code/src/__tests__/real-cli.e2e.test.ts
```

fork interpreter 改动用 vitest 验证（fork 用 vitest，不是 bun test）：

```bash
cd packages/just-bash/packages/just-bash && npx vitest run src/interpreter/
```

### 14.3 Step 0 — 初始化 monorepo 与 core 包

**目标**：建起 bun workspace 骨架，落地跨包共享的基础类型。

**做什么**：

1. `bun init` 建 monorepo 根，配 `workspaces` 指向需要发布/消费的 workspace package。
2. 根 `package.json`、`tsconfig.json`（base）。
3. 建 `packages/core`，对照 Rust `alloy-generated` 定义方案里反复引用的类型：
   - `Block` 及其所有变体（User/Resume/Thinking/Text/ToolCall/Response/Error/Abort/CompactionBoundary/CompactionMarker/ExtensionStateSnapshot）。
   - `Transcript`（blocks 数组 + 方法签名）。
   - `UserContentBlock`、`ToolResultContentBlock`、`ToolCallStatus`。
   - `ModelSelection`、`Model`、`TokenUsage`、`SessionPhase`、`QueuedMessage`、`ThinkingConfig`。
4. 类型只定义、不实现逻辑。逻辑归各包。

**验收**：`tsc --noEmit` 通过；core 包可被其他包 import 类型；类型与 Rust 蓝本 `alloy-generated` 对齐。

### 14.4 Step 1 — provider 包（接口 + stub）

**目标**：定义 agent 调用模型的标准接口，并提供 stub provider 让后续 agent 测试能跑。接口对照 Rust `provider` crate。

**做什么**：

1. `packages/provider`，定义 `InferenceRequest`（**纯 items 数组模型**，与 Rust 蓝本一致）：
   ```ts
   interface InferenceRequest {
     modelId: string
     systemPrompt: string
     cwd: string
     items: InferenceItem[]
     tools: ToolDefinition[]
     thinking?: ThinkingConfig
     cancel: AbortSignal
   }
   ```
   - `InferenceItem`：UserMessage / AssistantText / AssistantThinking / AssistantRedactedThinking / ToolUse / ToolResult。
2. `ProviderEvent`（对照 Rust `runtime.rs`）：ThinkingStart / ThinkingDelta / ThinkingSignature / RedactedThinking / TextDelta / ToolCallRequested / Response(usage) / Error / Abort。
3. `AgentProvider` 接口 + public `Provider` / `ProviderSelection`（按 provider id 选择、server-held runtime factory、state/model catalog 观察）。
4. `StubProvider`：可脚本化的事件流——测试时传入一组 `ProviderEvent` 序列，`run()` 按序 yield。支持 tool call 请求 + 第二轮续接。
5. auth 能力先留接口空壳，不实现。

**关键点**：`InferenceRequest` 是纯 items，不含 JSONL / resume / MCP 任何 claude-code 专属概念。这些是 Step 6 provider 实现内部的事。agent 永远只构造 items、消费 ProviderEvent。

**验收**：StubProvider 能模拟"返回文本 → 请求 tool call → 收到 tool result 后续接文本"的完整两轮；provider 包只依赖 core，不依赖 agent。

**为什么放这么早**：Step 2 的 agent 测试（send/retry/resume/compaction）每一条都需要 provider 驱动。

### 14.5 Step 2 — agent 包

**目标**：实现通用 AgentSession，对照 Rust `agent-session` 蓝本，复刻已验证的 session lifecycle。不接 coding、不接 bash。

**2.1 Transcript**：`Transcript` 实现 blocks 数组、append-only、所有 push/apply 方法（对照 `transcript.rs`）。单测覆盖 block 追加、tool call 完成、dangling 清理、compaction boundary 插入后 effective_transcript 切片正确。

**2.2 Session runtime 骨架**：`AgentSession` + 内部 worker（单 worker 串行）；状态机 Idle/Running/Compacting；queue；mutation guard。**不实现 replay_from**。

**2.3 动作实现**（对照 `worker.rs`）：send（BeforeRoundStart → resolve refs → push user turn → preflight compact → provider turn）、retry（truncate + AfterTranscriptRewrite + 重发）、resume（mark abort resumed + push resume turn + provider turn）、compact。

**2.4 Provider turn + compaction**（对照 `turn.rs`）：execute_provider_turn（build InferenceRequest → stream events → 应用 transcript → 执行 pending tools → auto-recover）、execute_compaction（find cut point → provider 生成 summary → insert boundary + marker）、execute_pending_tools。

**2.5 测试规格复刻**（11 条）：(1) send 先写 user turn 再跑 provider；(2) provider 请求含 system prompt/preamble/transcript；(3) tool 调用后继续 provider roundtrip；(4) abort 不被 ref resolution 或 tool execution 阻塞；(5) queue 在当前 run 结束后继续处理；(6) retry 截断最后一轮并重放 user message；(7) resume 添加 continue user turn；(8) transcript 全量无损 append-only，compaction 只插 boundary/marker；(9) compaction 插入 boundary 和 marker，下次 inference 从最后一个 boundary 起取；(10) extension state snapshot 持久化；(11) mutation guard 阻止 running/pending/reserved 下的并发外部编辑。

**2.6 事件产出**：`SessionEvent` 类型（transcript 变更/phase/queue/tool_progress/error）。AgentSession 不实现 transport 帧协议，`shell_output` 事件在 Step 3 接入 bash 后补。

**验收**：11 条测试全过；事件流测试通过；agent 只依赖 core + provider，无 fs/bash/child_process 依赖（持久化通过 Host 的 store facet，测试用内存 HostStore）；没有任何 Bash/editor/todo 字样。

**关键注意**：Rust 蓝本里 transcript 持久化和 extension state 落盘用了 fs。demi 的 agent 要保持纯净——持久化通过 Host 的 `store` facet，不在 AgentSession 或 BashEnvironment 中直接依赖本机 fs。

### 14.6 Step 3 — just-bash + shell 包

最大的一步，分多个小步。策略是先把 `wspl/just-bash` 作为 `packages/just-bash` submodule 接入，再接 `@demi/shell` 的 shell session、Host、registered commands 和 audit；不继续复制零散 parser/interpreter 片段。

**3.1 Fork just-bash**：fork `vercel-labs/just-bash`，记录 upstream repo/commit/license，保留 Apache-2.0；git submodule 接入 `packages/just-bash`；fork 包保留完整 parser/interpreter/builtin/expansion/upstream tests，demi 的 bash 语义改动直接在 fork 内维护；fork 默认入口 browser-safe；fork 暴露稳定 engine API（§7.1）；demi 只保留 submodule 指针。**验收**：fork core tests green；fork 默认入口 browser-safe；仓库内没有第二套 just-bash 源码树。

**3.2 接入 command registry + 注册命令调度**：在 forked engine 的命令调度处插入 registered command hook；实现 `CommandSpec` 解析（扁平 argv → parsed.values）；`renderCommandPrompt(spec)` + `<command> prompt` 同源；`--json`/raw 双模式。**验收**：注册命令拦截 argv[0] 不落进 host external command；注册命令名不能复用 shell builtin、portable command 或已知 host external command 名；prompt 与 renderer 一致。

**3.3 Host + LocalHost**：定义 `Host`（`defaultCwd` + `fs` + `process` + `store`）；`LocalHost.process.spawn` 走 Node `child_process.spawn`，`LocalHost.fs` 走 Node `fs/promises`，`LocalHost.store` 走 Node fs 下的状态目录。**验收**：LocalHost 跑通基本进程生命周期、文件系统 read/write/append/list/stat/rm/mkdir、状态 read/write/list/delete。

**3.4 Shell session + 状态连续性**：`ShellSession` 结构；engine 的 cwd/env/last status 读写指向 session；状态类 builtin 改 session 状态；fork portable commands 通过 Host.fs 执行；真实外部命令通过 Host.process.spawn 执行；list operators + pipeline 由 engine 解释；文件重定向经 Host.fs；prefix assignment；parameter/command/glob expansion；根入口平台无关。**验收**：§4.7.1 的连续性场景全部通过，且 `cat`/`ls`/`grep`/redirection 可在 memfs/virtual Host 上不依赖系统 coreutils。

**3.5 ShellCommandSnapshot + command artifact**：runner 实时收集模型可见 stdout/stderr 到 command artifact，并维护 stdout/stderr 交错后的 output artifact；artifact 通过只读 `/@/commands/<commandId>/...` 虚拟文件暴露给 just-bash portable commands；`yieldAfterMs` 必填且最大 10 分钟；tool result 只返回状态、artifact 路径和按当前模型 `contextWindow` 自动预算的 preview；安静长命令默认仍返回 `running`，不由 idle timeout 推断输入需求；`ShellCommandSnapshot` 只包含 `running` / `exited` / `aborted`。**验收**：长命令可观测，`shell_exec` 超过 `yieldAfterMs` 返回 `running + commandId` 且不杀进程，模型可用 `tail` / `grep` / `sed` / `awk` 读取 `/@` artifact，§4.7.2 行为通过。

**3.6 status / write / abort / dispose**：`shell_status`/`shell_write`/`shell_abort`/`disposeShell`/`disposeAllShells`；`shell_status` 负责非阻塞读取 command 状态、计时、字节计数和 artifact 路径，不返回 stdout/stderr 正文；`shell_write` 要求非空 stdin；主动 `shell_abort` 是控制动作；AgentSession abort 时终止前台进程；AgentServer 关闭 session 时统一 dispose Bash Environment，并调用 harness lifecycle 清理。**验收**：各路径单测通过，且 `shell_status` 不再承担输出分页。

**3.7 DEMI_SESSION_ID / DEMI_SHELL_ID + Host.store + CommandStorage**：创建 shell 时注入 `DEMI_SESSION_ID`（agent session id）和 `DEMI_SHELL_ID`（shell 控制句柄）；`HostStore` 接口挂在 Host 上；`CommandStorage` 是 Host.store 的 agent-session-scoped 视图，拒绝绝对路径/`..`穿越/非法 agent session id。**验收**：同一 agent session 的多次 shell_exec 共享 todo 状态，不同 agent session 读写互不干扰。

**3.8 Audit**：`BashAuditEvent` 区分 registered command、portable command 和 host external command；每次 command 收集 audit events 附在 `ShellCommandSnapshot` 的完成结果上。**验收**：三类命令各产生对应 kind 的 event。

**3.9 标准基础工具组封装**：`@demi/agent` 把 BashEnvironment 的 exec/status/write/abort primitives 和 agent-level `yield` 包装成 §3.2 的五个 AgentTool；`yield` 是 terminal tool result，并登记 delayed wakeup 调度器；到点时 idle session 开内部 wakeup turn，active session 走内部 steer，不进 queue；zod schema；接入 agent 的 ToolContinuation；shell 命令实时输出经协议层 `shell_output` 事件流出。**验收**：主路径手测通过（cat/editor/git status）；长命令手测通过；shell session 状态跨 exec 连续。

**3.10 fork engine 集成与 @demi/shell 重写**：把 `@demi/shell` 手写的 bash 语义（`environment.ts` 3340 行 + `script-parser.ts` 1444 行）替换成 fork `Interpreter`，对齐 §7.1。这是 Step 3 的收尾重构——3.1-3.9 的能力全部保留，实现从"手写 interpreter"切换到"消费 fork engine"。

前置：fork 已暴露 `./interpreter` 子路径和 `hostSpawn` 钩子（fork commit `5e925b7`）。

做什么：
0. **fork 第二个改动**：`executeExternalCommand` 在 `hostSpawn` 检查之后、`resolveCommand` 之前，增加直接 `ctx.commands.get` dispatch（§7.1"注册命令调度路径的已知缺口"）。
1. **重写 `host-fs.ts`**：`HostBackedFileSystem implements IFileSystem`，把 fork fs 操作路由到 Host.fs，不再路由到 Host.process.spawn 的 cat/tee/test/find/stat。
2. **重写 `environment.ts`**（目标 < 800 行）：`ShellSession` 持有 fork `InterpreterState`；**不用 fork 的 `Bash` 类**，直接用 `Interpreter`；fork `CommandRegistry` 合并 fork portable commands 和 demi registered commands（§7.1"fork portable commands 必须注册"）；`BashEnvironment.exec` 用 `parse` + `new Interpreter` + race `executeScript` 和边界点（§7.1 执行模型）；`ExecutionLimits` 传高限值（§7.1）；`hostSpawn` 仅实现真实外部 Host.process.spawn + foreground + 边界点 + audit；`commandSpecToForkCommand` 适配器（§7.1 字段映射）；`status`/`write`/`abort`/`dispose` 操作 command record 与 foreground process。
3. **删除 `script-parser.ts`**，AST 类型用 fork `./ast/types`。
4. **更新 `@demi/shell` package.json**：从 fork `./interpreter`/`./ast/types`/`./parser` 导入；移除 `re2js`。
5. **跑全部测试** + fork interpreter 用 vitest 验证（§7.1"fork 测试基础设施"）。
6. **更新 git submodule pointer**：根仓库 `git add packages/just-bash` + 首次 commit。

验收：`environment.ts` < 800 行；`script-parser.ts` 删除；`@demi/shell` 不再手写任何 bash 语义；所有现有测试绿；`platform-entrypoints.test.ts` 绿；`@demi/shell` 不再依赖 `re2js`。

### 14.7 Step 4 — coding-agent 包

**4.1 Coding agent harness**：`CodingState`（初版不放 todos）；`systemPrompt`（注册命令说明由 command registry 自动注入）；`host()` 返回执行后端；`commands()` 注册 editor/todo 和调用方扩展命令；`lifecycle`；`resolveReferences` 经 Host.fs 展开文件引用。公开入口为 `createCodingAgentHarness({ host, commands?, referenceHost? })`；不得接收 `BashEnvironment`。

**4.2 editor 命令**：`editor prompt`（renderCommandPrompt 同源）；`editor create`（Host.fs exists/mkdir/write）；`editor edit`（Host.fs read → 精确替换 → Host.fs write，`--occurrence`/`--context` 消歧）；`editor patch`（unified diff 解析 → 多文件应用）；文件访问全走 Host.fs；editor/reference resolver 运行时不静态依赖 `node:*`/`Buffer`/`process.env`。

**4.3 todo 命令**：`todo prompt`/`list`/`add`/`update`/`done`；状态经 CommandStorage 按 `DEMI_SESSION_ID`（agent session id）隔离存 `todos.json`，支持 `pending` / `in_progress` / `done`；`--json` 模式。

**4.4 集成验收**：system prompt 自动包含 editor/todo 说明；`editor prompt` 与 system prompt 中的 editor 说明一致；coding marathon 测试通过。

### 14.8 Step 5 — AgentServer / AgentClient / transport

**目标**：在 `@demi/agent` 内实现 AgentServer/AgentClient/transport（§11-§12）。本地 JS 调用方用 `server.client()`，远端调用方用 transport；两者共享同一套 frame handler。依赖 AgentSession 和 bash（shell_output 事件），不依赖具体 provider。

**5.1 帧协议**：`ClientFrame`/`ServerFrame`（§12.2）；`AgentTransport` 接口；跨进程帧序列化用 JSON，in-process 不序列化；默认 `@demi/agent` 根入口不静态依赖 Node-only transport。

**5.2 AgentServer**：`AgentServer` 绑定一个 `AgentHarness`，每个 transport 连接持有一个 `AgentSession`，把 ClientFrame 转成 session 调用，把 SessionEvent 转成 ServerFrame；一连接一 session；transcript 变更产出 snapshot/patch；shell_output 透传 `ShellCommandSnapshot`；`send` busy 时进 queue，`retry`/`resume`/`compact` busy 时回 `rejected`。

**5.3 AgentClient**：`AgentClient` 持有 transport，把方法调用转成 ClientFrame；客户端维护本地 transcript 视图（apply snapshot/patch）；`send`/`retry`/`resume`/`compact` 的 Promise 按本地 action FIFO 收敛（§3.1 守则 12）。

**5.4 Transport 实现**：`InProcessTransport`（本地直连，零成本）；`StdioTransport`（NDJSON，Node-only adapter，显式子路径）；`WebSocketTransport`（JSON 文本帧）。

**5.5 验收**：`server.client()` 端到端；shell_output 帧正确带 shellId；busy 时 send 触发 queue 帧并按序 resolve；retry 的 transcript_patch 包含删除的 block；stdio transport 端到端；本地 client 和跨进程 client 走完全相同的 AgentClient/帧协议代码。

### 14.9 Step 6 — provider-claude-code 包

**目标**：实现第一个真实 provider，驱动系统装的 claude code CLI。以 Rust `provider-claude-code` crate 为蓝本。**不依赖 `@anthropic-ai/claude-agent-sdk`**。

**机制**（零 SDK 依赖）：spawn `claude` CLI（`--print --output-format stream-json --verbose --input-format stream-json --tools ''`）；stdin/stdout JSON 行通信；history 由 `InferenceItem[]` 转成 stream-json stdin messages；MCP bridge 手写 JSON-RPC（initialize/tools/list/tools/call/ping）；env `DISABLE_AUTO_COMPACT=1`/`MAX_MCP_OUTPUT_TOKENS=1000000`/清除 `CLAUDECODE`；stream event → `ProviderEvent` 映射。

**做什么**：`Transport`（spawn CLI/stdin 写/stdout 读/kill/wait）；`CliStatus`（which claude/claude --version）；stream-json input message 转换；MCP bridge；`StdoutMessage` 解析 + event 映射；auth/状态检测。

**验收**：agent + claude-code provider 能完成多轮带 tool call 的对话；tool 执行经 bash 包的四个工具；compaction 由 agent 接管；abort 能 kill CLI 进程；provider 包 `package.json` 无 SDK 依赖。

**为什么放最后**：需要系统装 claude code CLI + 登录。前面所有逻辑用 stub 验证清楚后，接真实 provider 只是适配层。

### 14.10 当前进展

Step 0-6 的本地包和集成测试已落地：core / provider / agent / just-bash / shell / host-local / coding-agent / provider-claude-code / provider-codex / provider-openai-api / provider-anthropic-api。公开运行入口已调整为 `AgentServer` + `AgentClient`：本地调用使用 `server.client()`，跨进程/网络使用 `attachTransport()`；coding harness 只定义 Host、commands、prompt、reference resolver 和 lifecycle。当前门禁已通过：`bun run typecheck`、`bun run test`（345 pass / 10 skip）、`bun run test:just-bash-core`、`packages/just-bash/packages/just-bash` 下的 `pnpm typecheck`、`./node_modules/.bin/vitest run src/interpreter/`。

**已完成的关键实现**（按 §3.1 架构守则覆盖）：

- bash engine 边界已拆成 `just-bash` fork package 与 `@demi/shell`：`packages/just-bash` 是 submodule，暴露 parser/IFS 稳定导出。
- `@demi/shell` 默认入口 browser-safe：根入口只导出平台无关实现；本机系统能力由 `@demi/host-local` 的 `LocalHost` 提供。
- `@demi/agent` 默认入口去除 Node-only 依赖：stdio transport 从根入口移出，JSON codec 和 transcript patch diff 改为纯 TS/Web 标准。
- `@demi/coding-agent` 运行时入口去除 Node-only 依赖：editor/reference resolver 使用 Host contract、Web UTF-8、平台无关路径归一化。包名和主入口已迁移为 `@demi/coding-agent` + `createCodingAgentHarness`。
- 平台默认入口静态闭包测试覆盖所有平台无关包；just-bash 边界静态扫描；AgentServer/transport 边界静态扫描；package manifest 分层扫描。
- shell session 生命周期、状态连续性、长命令观测、DEMI_SESSION_ID/DEMI_SHELL_ID、CommandStorage、audit、注册命令 prompt/`--json`/commandMetadata、shell 控制接入 AgentSession abort signal——均已具备基础实现；最终 shell + yield 控制面将收敛为 `shell_exec` / `shell_status` / `shell_write` / `shell_abort` / `yield`，见 `docs/shell-yield-control-plan.md`。
- agent 的 transcript/queue/retry/resume/compaction/mutation guard/abort 收敛/tool error 局部化/extension state snapshot——均已落地。
- AgentClient 的 action FIFO 收敛/abort Promise/shellWrite 等待/transcript snapshot+patch/stdio+websocket transport/Uint8Array+bigint 安全编解码——均已落地。
- claude-code provider 的 CLI spawn/stream-json/MCP bridge/event 映射/tool_use continuation/abort 清理/binary media 转换/config 白名单——均已落地。真实 CLI e2e 默认跳过，`DEMI_CLAUDE_CODE_E2E=1` 手动跑。
- provider public API 已收敛为用户侧 direct provider creation：`createClaudeCodeProvider` / `createCodexProvider` / `createOpenAIApiProvider` / `createAnthropicApiProvider` 返回 public `Provider`；AgentServer 接收 `providers: Provider[]`，协议只携带 `ProviderSelection`，不再让 secret-bearing config 往返浏览器。
- OpenAI API provider 已落地：默认 `OPENAI_BASE_URL ?? https://api.openai.com/v1` 和 `OPENAI_API_KEY`，支持 `envPrefix`、显式 `baseUrl`/`apiKey` 优先；默认 model catalog 镜像 Codex 当前可见模型集合；传入 `models` 时全量替换默认 catalog；默认 `wireApi: "responses"` 走 Responses request/body/tool/result 映射和 SSE text/tool/usage 映射，兼容 endpoint 可显式传 `wireApi: "chat-completions"` 走 Chat Completions 映射；兼容 endpoint 若在 `choices[].delta.reasoning_content` 暴露非标准 reasoning delta，则 best-effort 映射为 Demi `thinking_*` 事件，不伪造 Responses encrypted reasoning signature。
- Anthropic API provider 已落地：默认 `ANTHROPIC_BASE_URL ?? https://api.anthropic.com/v1` 和 `ANTHROPIC_API_KEY`，支持 `envPrefix`、显式 `baseUrl`/`apiKey` 优先；默认 model catalog 镜像 Claude Code 当前模型集合；传入 `models` 时全量替换默认 catalog；Messages request/body/tool/result 映射和 event-stream thinking/text/tool/usage 映射。
- coding editor/todo/reference resolver 已改为使用 Host contract；editor patch 兼容 `diff -u`/git-style unified diff。editor/reference resolver 不再把 default cwd 当作 workspace/sandbox/权限边界；如果以后需要项目级路径限制，只能作为显式 policy 或 command-level guard 建模。

**已修正 Step 3 的重大偏离**：

`@demi/shell` 的 `environment.ts`（3340 行）+ `script-parser.ts`（1444 行）手写了一整套 bash interpreter（compound command / 状态 builtin / expansion / arithmetic / glob / pipeline / redirection），而 fork 只被当成 tokenizer 用（只导入 `parse` 和 IFS helpers）。这违背 §7"不在 `@demi/shell` 或其他包复制实现"、"不得重新创建内部 just-bash 副本"的核心约束。fork 实际已有完整可运行的 interpreter（7600+ 行，覆盖度远超 demi 手写版本）。

**Step A 已完成**（fork commits `5e925b7`、`4e2ab29`、`c7f1be5`、`cabfc0f`）：fork 暴露 `./interpreter` 子路径（`Interpreter`/`InterpreterState`/`InterpreterContext`）；`InterpreterContext` 增加可选 `hostSpawn` 钩子，`executeExternalCommand` 在有 `hostSpawn` 且命令非注册命令时走钩子而不是 IFileSystem + PATH；`executeExternalCommand` 在 `hostSpawn` 检查之后增加直接 `ctx.commands.get` dispatch，让注册命令不走 PATH 查找；后续补齐 host-backed session hooks、bash 语义修正和 Node 25 fetch mock type compatibility。fork interpreter 完整 vitest 当前为 617 pass / 1 skip，demi 根测试当前为 345 pass / 10 skip。

**git 状态**：demi 根仓库已完成首次提交，`packages/just-bash` 已通过 `git submodule add` 正式登记为 submodule（`.gitmodules` 指向 `https://github.com/wspl/just-bash.git`）。fork 子模块 worktree 已干净，当前 HEAD 是 `cabfc0f`；根仓库 submodule pointer 已指向 `cabfc0f`。

**Step 3.11 Host.fs / portable command 路由与 Host facet 收敛已完成**：`Host` contract 已收敛为 `defaultCwd + fs + process + store`。`HostBackedFileSystem` 路由到 `Host.fs`，不再通过 process spawn 执行 `cat`/`tee`/`test`/`ls` 等命令；真实外部命令统一经 `Host.process.spawn`；agent/session 命令状态统一经 `Host.store`。`BashEnvironment` 每个 shell session 都注册 fork portable commands，并在名称冲突时让 demi registered commands 覆盖；portable/registered commands 在 host external command 前命中。对应测试已覆盖 fake `Host.fs` 记录、process spawn 禁用、portable `cat | tee` 读写再读、reference read、patch rollback、`LocalHost.fs` 基础操作和 `LocalHost.store` 基础操作。editor/reference resolver 不再使用 default cwd 派生访问边界。

**本轮已修正的其他偏离**：

1. 旧 agent definition API 已移除。公开装配入口统一为 `AgentHarness`；coding harness 通过 `commands()` 暴露 editor/todo，命令 prompt 仍由 `CommandRegistry.renderPrompt()` 同源渲染；public `BashEnvironment` 注入和 public `tools()` 替换点已移除。
2. AgentServer 不再用 shell 输入伪造 `toolCallId`；最终协议使用 `shell_write_result` 帧，用 `commandId` 作为完成确认与客户端 Promise 收敛的关联键。
3. fork interpreter 的若干 bash 语义已按真实 bash 对齐：`break`/`continue` 外层与非法参数、`shift` 非数字/负数、`set` invalid option、`eval` 解析错误退出码；同时修正 group stdin consumption，避免非 stdin-consuming 注册命令误消费外层输入。
4. fork 测试 mock 已兼容 Node 25 的 `fetch.preconnect` 类型变化，`pnpm typecheck` 当前通过；真实 DNS/network 集成测试仍受本机 DNS 策略影响，不作为 Step 3.11 验收证据。

### 14.11 Codex Provider 最终态入口

Codex provider 的调研过程、最终态设计和落地记录见 `docs/codex-provider-research.md`。核心结论：Demi 不实现自己的登录流程，而是兼容官方 Codex auth storage，默认复用 `$CODEX_HOME/auth.json` / `~/.codex/auth.json`，并按官方 refresh/route/header 规则请求 Responses。

已补齐 provider contract 的稳定 `sessionId` / `turnId` / `requestId`：AgentSession 负责生成并传入 provider，Codex provider 用它们设置 `session-id`、`thread-id`、`x-client-request-id` 和 `prompt_cache_key`。WebSocket/SSE、auth refresh、reasoning/tool replay 仍保持为 provider 内部机制，不进入 Agent Loop。

当前实现包含 `packages/provider-codex`、REPL `--provider codex` 入口、platform boundary 检查、默认 deterministic provider tests，以及默认跳过的真实 Codex e2e。真实 e2e 已用本机官方 Codex auth 验证 text、medium thinking、cache usage 和 shell tool roundtrip；测试模块和 gated 验收入口见 `docs/testing.md#531-codex-provider`。

### 14.12 OpenAI API / Anthropic API Provider 最终态入口

`packages/provider-openai-api` 和 `packages/provider-anthropic-api` 是 concrete provider leaf packages。它们不复用 Codex Responses 或 Claude Code JSONL/CLI mapper，而是分别实现官方 HTTP API wire contract：

- OpenAI API：默认 `wireApi: "responses"`，`POST {baseUrl}/responses`，Responses stream 的 `response.output_text.delta` 映射为 `text_delta`，`response.function_call_arguments.*` 与最终 `function_call` item 聚合为 `tool_call_requested`，usage 映射为 Demi `TokenUsage`。兼容 endpoint 可显式传 `wireApi: "chat-completions"`，走 `POST {baseUrl}/chat/completions` 和 `choices[].delta.*` 映射。
- Anthropic API：`POST {baseUrl}/messages`，event stream 的 thinking/text/tool_use/message_delta/message_stop 映射为 Demi `ProviderEvent`，tool result 以 Anthropic user content block replay。
- Endpoint/env 规则一致：显式 `baseUrl` 优先，其次 `${envPrefix}_BASE_URL`，最后官方默认 endpoint；显式 `apiKey` 优先，其次 `${envPrefix}_API_KEY`。默认 prefix 分别为 `OPENAI` 和 `ANTHROPIC`。
- Secret boundary 一致：API key、自定义 headers、raw baseUrl、envPrefix 和 raw provider options 只留在 provider creator closure，Web `listProviders` / `listModels` / `prepareSession` 和 AgentClient frames 不携带这些值。

REPL/Web 当前 composition root 默认装配 `createClaudeCodeProvider()`、`createCodexProvider()`、`createOpenAIApiProvider()`、`createAnthropicApiProvider()`；`--provider openai|anthropic` 选择对应 provider，`--base-url` 只覆盖当前选中的 HTTP provider；`--openai-wire-api responses|chat-completions` 只控制 OpenAI provider 的 wire API。Web 启动时传入的 `--model` / `--model-context-window` / `--model-display-name` / `--model-thinking-efforts` / `--model-can-disable-thinking` / `--thinking` 必须在 composition root 转成选中 API provider creator 的 `models` / `defaultModelId` 配置，由 provider catalog 暴露 browser-safe 模型能力元数据；`--model` 全量替换选中 provider 的 control catalog，保证 DeepSeek、OpenRouter 等 compatible endpoint 不会因为默认 catalog 的任何模型而请求错误模型；`--model-context-window` 是 caller-declared compatible model 的必填正整数，供 context usage、compaction 阈值和模型切换决策使用，不能默认为 unknown；`--model-display-name` 只覆盖 UI 展示名，不改变 provider request 使用的真实 model id；`--model-thinking-efforts` / `--model-can-disable-thinking` / `--thinking` 描述该模型是否支持 reasoning、有哪些 effort、默认 effort 和是否允许关闭，UI 据此显示 reasoning selector，provider request 仍只接收 `ProviderSelection.model.thinking`；这个过程仍只暴露 `providerId`、`modelId` 和便携模型元数据，不把 raw endpoint 或 secret-bearing options 发给浏览器。

## 15. 优先级

**P0**

- Agent Loop 测试规格复刻
- Agent Loop 实现
- AgentSession 事件产出（供 AgentServer 消费，§11.1）
- Host 抽象 + LocalHost
- Bash Engine 接入 just-bash parser/interpreter 能力
- just-bash fork core tests
- 标准基础工具形状（exec/status/write/abort/yield，shellId + commandId）
- shell running result + commandId
- ShellCommandSnapshot
- BashEnvironment（基于 Host contract）+ 显式 LocalHost adapter
- host external command spawn 不做 allowlist

**P1**

- shell session 状态连续性（跨 exec 复用 cwd/env，§4.7.1）
- stdout/stderr 经协议层 `shell_output` 事件流出
- status / write / abort / yield
- `DEMI_SESSION_ID` / `DEMI_SHELL_ID` env 注入与命令状态隔离
- coding 注册命令：`editor` / `todo`
- 注册命令 `--json` / raw 双模式输出
- 注册命令 prompt 自动注入 system prompt
- `<command> prompt` 与 `renderCommandPrompt(spec)` 同源
- audit metadata

**P2**

- AgentServer transport：帧协议 + AgentClient + in-process / stdio / WebSocket transport（§12）
- transcript snapshot/patch 增量同步
- 超出 just-bash 基线的 shell compatibility
- command-level diff

**P3**

- RemoteHost / ContainerHost（换 Host 实现，BashEnvironment 不变）
- 更完整 bash compatibility

> 注：本方案不设计安全护栏 / 权限模型，第一版 host external command spawn 完全自由。这是有意取舍，不在优先级列表里。

### 15.1 风险与应对

| 风险 | 应对 |
|---|---|
| just-bash 体量大（13 万行），复制片段会无穷无尽 | 改为 fork-first：完整 fork 为实现基线，在 fork 内补 engine 扩展 API；`@demi/shell` 调稳定 fork API，不再新增零散复制，也不把 AgentServer/provider 逻辑塞进 fork。 |
| 状态类 builtin、portable command 与 host external command 的边界模糊 | §4.1 调度优先级已明确：状态类 builtin engine 内实现；fork portable command 经 command registry + `Host.fs` 执行；真实外部命令才 `Host.process.spawn`。Step 3.4 专门验证连续性和 portable command 路由。 |
| `defaultCwd` 被误用成 workspace/sandbox/权限边界 | §8 已明确 `defaultCwd` 只是默认 cwd 和相对路径解析 helper。coding editor/reference resolver 不能用它派生路径访问权限；如果需要项目级限制，必须显式建模 policy 或 command-level guard。 |
| Step 2 持久化若硬编码 fs 会破坏纯净性 | agent 持久化走 `Host.store`，测试可用内存 HostStore；本机落盘只是 `LocalHost.store` 的实现细节。 |
| claude-code provider 的 stream-json/MCP 机制复杂 | 对照 Rust `provider-claude-code` crate 的结构实现；Step 6 专门拆成 CLI 输入转换 / MCP / event-mapping 三块。 |
| `InferenceRequest` 接口设计不当导致 Step 6 重做 | Step 1 接口对照 Rust `provider` crate，纯 items 模型；stream-json/MCP 不进接口，是 Step 6 内部细节。 |
| `hostSpawn` 挂起时 `executeScript` 的 Promise 一直 pending | 保留在 command record 的 pending exec 中；`shell_status` 只读 command record，不续接 race；`shell_abort` 或 dispose 时 kill 进程后 Promise 自然 settle。 |
| `HostBackedFileSystem` 缺失完整 Host.fs 后端 | 已完成：`HostBackedFileSystem implements IFileSystem` 委托 `Host.fs` 覆盖 fork redirection、glob、source、file tests 和 portable commands 所需的 read/write/append/exists/stat/lstat/readdir/mkdir/rm/realpath 等操作；测试断言这些操作不调用 `Host.process.spawn`。 |
| fork portable commands 未注册导致退回系统 coreutils | 已完成：每个 shell session 合并 fork portable commands 和 demi registered commands，调度路径在 `hostSpawn` 前命中 portable/registered commands；`NoSpawnLocalHost` 测试验证 `cat input.txt | tee copied.txt` 和后续 `cat copied.txt` 不依赖本机 `/bin/cat` / `/usr/bin/tee`。 |
| fork 的注册命令调度顺序和 demi 预期不同 | fork 默认顺序不能让 shell function 遮蔽 demi registered command。后续 Host.fs 重构时同时验证 builtin、registered command、portable command、host external command 的优先级；必要时在 fork 稳定 API 中调整，而不是在 `@demi/shell` 复制 dispatch。 |
| 输出重定向跨 yield/abort 的文件 sink 写入 | fork 的 redirection 语义必须通过 IFileSystem + output sink 保持；长命令 yield 时不能把本该写文件的 stdout/stderr 泄漏到 tool output。Host.fs 重构后用长命令 + redirection + abort 场景验证。 |

### 15.2 不做的事

- 不搬 agent-gui 任何代码，只参考 Rust 蓝本结构。
- 不做 module-agent / frontend 实现 / 额外 facade（demi 是纯库，协议层供壳子接入）。
- 不做安全护栏 / 权限模型（方案已定）。
- 不做 MCP / Skills / 子 agent（方案已定）。
- 不做 RemoteHost / ContainerHost（P3）。
- 不做 replay（方案已删）。
- 不预先裁剪 just-bash；先维护完整 fork，再按明确 package 边界接入 demi。

# Tool Rendering Specification

| | |
|---|---|
| 日期 | 2026-06-25 |
| 状态 | 最终规范 |
| 范围 | 标准 agent tools 在 Web / REPL / 其他壳子里的展示规则 |

## 1. 目标

Demi 的底层 transcript 和事件协议已经足够表达工具调用：`@demi/core` 定义
`Block`，`@demi/agent` 定义 `ClientSessionEvent`。工具展示不需要也不应该引入新的
model/render-model 包。

标准工具展示的原则是：

1. `tool_call` 是持久 transcript 的 envelope，不是展示类型。
2. 壳子先按 `block.type` 分发；遇到 `type === "tool_call"` 后必须按具体
   `block.toolName` 分发。
3. Demi 标准工具 `shell_exec` / `shell_status` / `shell_write` / `shell_abort` /
   `yield` 都必须有一等展示，不允许落到普通 generic tool card。
4. Generic tool 展示只用于未知外部工具或未来扩展工具，不用于标准工具。
5. Web 和 REPL 可以各自实现 DOM / terminal UI，但必须消费同一套 `Block` 与
   `ClientSessionEvent` 数据，不复制协议或引入平行数据模型。

## 2. 共享协议边界

持久历史以 `Block` 为准：

- `Block.type === "tool_call"` 表示模型发起了一次工具调用。
- `toolName` 是标准工具展示分发键。
- `input` 是 provider 传入的 JSON 字符串；渲染层负责解析。
- `status` 是 `executing | completed | error`。
- `streamingOutput` / `output` 是工具输出文本或媒体块。
- `metadata` 可携带 `ShellCommandSnapshot` 等结构化运行时状态，渲染层可以用它增强展示，
  但不能把它当作唯一来源。

实时事件以 `ClientSessionEvent` 为准：

- `transcript_snapshot` / `transcript_patch` 是持久 UI 的主输入。
- `shell_output` / `tool_progress` 可以给正在执行的标准工具补实时 stdout/stderr 或状态。
- `shell_write_result` / `abort_result` 是用户控制动作 ack，不替代 transcript 里的
  `tool_call` 展示。
- `audit` 可以在 `shell_exec` 卡片内展示注册命令或系统命令明细，但不能改变标准工具分发键。

因此，Web、REPL 和未来壳子可以共享协议和事件结构，但不共享一个抽象 UI model 包。

## 3. Description 约定

所有标准工具的 input schema 都必须允许可选 `description?: string`。

`description` 是短的用户可见意图标题，要让用户看懂这一步想完成什么。

渲染规则：

1. 非空 `description` 是工具 block 的首选标题。
2. 没有 `description` 时，渲染层使用各工具的确定性 fallback。
3. `description` 只影响展示，不改变 shell runtime、tool result 或模型 replay 语义。
4. `description` 不应该是单纯事物名，也不应该塞入长脚本、完整 stdout/stderr、协议状态、
   step 编号、toolName、commandId、内部标签或原因说明。

## 4. 标准工具展示

| 工具 | 展示形态 | 标题 fallback | 关键内容 | 执行中状态 |
|---|---|---|---|---|
| `shell_exec` | 终端命令 block | `input.script` | script、stdout/stderr、exit/status、commandId、shellId | 扫光 loading，支持展开输出 |
| `shell_status` | 命令状态 inline block | `Check <commandId>` | 标题说明状态检查动作；不提供展开面板 | 扫光 loading，不能伪装成 shell_exec |
| `shell_write` | stdin 写入 inline block | `Send input to <commandId>` | 标题说明写入动作；不提供展开面板 | 扫光 loading，成功不等于命令完成 |
| `shell_abort` | 停止命令 inline block | `Stop <commandId>` | 标题说明停止动作；不提供展开面板 | 扫光 loading，completed/aborted 都不是 UI 错误 |
| `yield` | 等待唤醒 inline block | `Wait <durationMs>ms` | 标题说明等待时长；不提供展开面板 | 等待中使用和 thinking 一致的扫光 |

这些工具可以共用一个基础 `ToolCard` 外壳，但内容区域、标题 fallback、图标和状态文案必须按
工具名区分。图标可以不同：`shell_exec` 用 terminal，`shell_status` 用 activity/search，
`shell_write` 用 keyboard/input，`shell_abort` 用 stop，`yield` 用 clock/timer。
标准工具执行中状态统一使用和 thinking 一致的扫光 loading，不使用独立 spinner。

Web 中只有 `shell_exec` 工具块和 `thinking` block 可展开。`shell_status` / `shell_write` /
`shell_abort` / `yield` 以及未知 generic tool 都必须保持不可展开的 inline 呈现；错误信息如需
展示，只能作为 badge 或行内摘要出现，不能通过 disclosure 展开。

## 5. Web 规范

Web 的 `ToolCallBlock` 必须显式分发：

```text
shell_exec    -> shell exec renderer
shell_status  -> shell status renderer
shell_write   -> shell write renderer
shell_abort   -> shell abort renderer
yield         -> yield renderer
unknown       -> generic renderer
```

`AgentMessageVirtualBlock` 继续按 `block.type` 做第一层分发；`ToolCallBlock` 承担第二层
`toolName` 分发。虚拟列表、sticky user block、自动滚动、tail loading 等仍然是 Web 私有 UI
实现细节，不进入共享协议层。

Web 需要避免两类错误：

- 不要因为 `type === "tool_call"` 就把标准工具统一渲染成 generic card。
- 不要把 `shell_status` / `shell_write` / `shell_abort` / `yield` 伪装成
  `shell_exec` 的命令输出；它们是不同控制动作。

## 6. REPL 规范

REPL 继续消费同样的 `Block` 和 `ClientSessionEvent`，但输出是 terminal 行。

最低要求：

- `shell_exec` 输出 `tool> shell_exec ...`，并展示 script fallback。
- `shell_status` 输出 commandId 和状态检查含义。
- `shell_write` 输出 commandId 和 stdin 写入含义；stdin 内容可截断。
- `shell_abort` 输出 commandId 和停止含义。
- `yield` 输出等待时长和唤醒含义。
- `description` 存在时优先用于摘要；没有时使用第 4 节 fallback。

REPL 不需要复用 Web 组件，也不应该引入 DOM-oriented render model。它只需要和 Web 遵循同一份
工具名到展示语义的映射。

## 7. 验收与测试

必须覆盖：

1. 标准工具 schema：五个工具都允许 `description`，且标准工具集合仍然精确为
   `shell_exec/shell_status/shell_write/shell_abort/yield`。
2. Web 分发：`ToolCallBlock` 对五个标准工具都有专门 renderer，未知工具才进入 generic。
3. Web 展示：五个标准工具都优先显示 `description`，没有时使用第 4 节 fallback。
4. REPL 展示：五个标准工具的 terminal 摘要都优先显示 `description`，没有时使用第 4 节
   fallback，并避免 patch replay 重复输出。
5. 协议稳定性：`transcript_patch` 更新同一个 `tool_call` 的 status/output/metadata 时，Web 和
   REPL 都能更新同一展示块，而不是新增一条错误的重复块。

真实模型验收需要至少覆盖一次长命令流程：

```text
shell_exec -> yield -> shell_status -> shell_write 或 shell_abort
```

验收时应确认 Web 和 REPL 都能看清每个控制动作分别在做什么，而不是只看到一组泛化的 tool call。

# Demi Next: 同构会话、状态归属与执行恢复

| | |
|---|---|
| Date | 2026-09-05 |
| Status | Proposal — 待设计评审，尚未实现；现行包边界仍以 `docs/package-boundaries.md` 为准 |
| Scope | 会话节点同构、持久化原子性、Host 与状态解耦、外部操作的恢复语义 |

## 设计原则

Agent 是可独立运行的会话节点。父子关系描述组织方式，不定义另一种运行时。
根节点是没有父节点的普通节点；subagent 是产品和命令界面对父子关系的称呼。

所有节点使用同一个装配入口、同一种生命周期、同一个存储契约、同一种事件、
同一个执行恢复机制。定义、模型、初始上下文和完成策略可以不同；机制保持一致。

复用以复用事实和行为为准：一个状态只有一个权威写入点，一个机制只有一种实现。
不为追求形式统一，把文件系统、会话存储、进程和产品权限合并成万能接口。

持久化提交、消息接受、执行派发和完成通知都具有明确的语义边界。
本地事务保证本地状态一致；外部效果的不确定性必须保留，不以错误码掩盖。

本提案不扩展任务预算、hostless 接受范围或多节点拓扑，不讨论具体 Sandbox 实现。

## 一种节点，一套运行时

以 conversation `c1` 中的三个 agent 为例：

```text
Backend product scope: c1
  产品所有权、文件/媒体命名空间、执行目标策略
  │
  └── AgentServer                         唯一运行时入口
        │
        ├── SessionRegistry               所有已装载节点的唯一登记处
        │     a1  parentId=null
        │     a2  parentId=a1
        │     a3  parentId=a2
        │
        ├── assembleSession               三个节点都经过这个装配入口
        │     └── SessionNode
        │           AgentSession          推理、队列、turn、compaction
        │           provider runtime      每节点独立
        │           command registry      每节点有效命令集合
        │           shell environments    每节点按执行目标管理
        │           persistence binding   每节点相同的提交保证
        │           event subscription    每节点相同的事件类型
        │
        └── 关系与完成策略                使用节点 API，不构造另一套节点
              spawn / send / steer / close / resume / list

Browser binding ── sessionId + action ──> AgentServer
demi agent CLI ── actorId + action ──────> AgentServer
```

`SessionNode` 是运行时资源的所有者，`AgentSession` 是会话状态机。
节点创建、恢复和 fork 都进入 `assembleSession`，没有子代理专用的工具构造、
provider 绑定、Host 环境缓存、订阅或销毁实现。快照复制只产生数据；创建可运行副本
仍经过同一个装配入口，新副本不会继承源节点的执行句柄或执行所有权。

`AgentServer` 保留现有公共运行时入口，不在外面增加一套职责相同的 Manager/Runtime。
其内部模块按装配、节点资源、登记、关系、协议绑定分别承担职责。
关系模块调用统一节点操作；命令和协议都是这些操作的适配器。

节点配置分为两类：

| 类别 | 归属和行为 |
|---|---|
| 定义与策略 | harness、有效命令集合、模型、提示配置、完成策略；创建时解析继承，显式更新后才变化 |
| 运行时资源 | provider、shell、订阅、取消信号；逐节点构造，关闭时逐节点释放 |

继承是“创建参数的求值”，不是对父节点可变对象的长期引用。命名 profile 和
未命名继承都先解析为同一种有效配置，再装配节点。回调函数由 harness 提供；
存储保存可序列化配置、定义标识和契约版本，不保存函数闭包。恢复时必须能解析
同一配置，缺失定义或不相容的契约应明确报错，不静默换用另一套权限或命令。

可变的共享执行目标是另一种明确的引用：节点指向产品 scope 的目标策略，
不通过继承父节点对象实现。每次新执行都解析该策略。

## 身份与数据归属

```text
scopeId = c1                 产品的 conversation 范围；不是一个 agent
  │
  ├── session a1             parentId = null
  ├── session a2             parentId = a1
  └── session a3             parentId = a2

执行 e7：scope=c1, actor=a3, target=device-d2
  a3 是实际调用者；c1 决定产品授权和默认执行目标
```

`scopeId`、`sessionId`、`parentId` 分别表达产品范围、节点身份和关系。
Backend 将 scope 映射到 conversation；agent 包不解释用户、workspace 或 device 表。
根 sessionId 不承担 conversationId 的隐含职责，也不替代子节点的调用者身份。

持久化按稳定的节点 ID 组织。Backend 继续使用每 conversation 一份 SQLite，
所有节点在同一数据库中使用带 `session_id` 的同形数据表：

```text
conversations/c1.sqlite
  sessions      (session_id, parent_id, definition, lifecycle, version, ...)
  blocks        (session_id, block_index, block_json)
  messages      (message_id, sender_id, recipient_id, delivery_state, content, ...)
  executions    (operation_id, session_id, invocation_id, target, state, evidence, ...)
  command_state (session_id, namespace, key, value, ...)

  a1、a2、a3 使用完全相同的行结构和读写入口
```

`parent_id` 是关系数据，不参与存储路径拼接。节点完成后仍能被正常读取，
树查询从关系数据获得完整结构，不依赖每层父节点是否在内存中。
session 状态不在单独的 ChildJob 元数据里再保存一份完成状态。

消息队列以 `messages` 为持久化事实来源；checkpoint 的待处理消息视图由它构造，
不同时维护两套互相复制的持久化 inbox。描述、完成结果和关系存于会话记录；
工具摘要等展示信息可以从 transcript 派生，不另建权威状态机。

## 存储与执行目标分离

Host 的职责为执行环境：`fs`、`process`、`identity` 和默认工作目录。
会话存储、消息和命令状态通过明确的存储依赖注入，不挂在 Host 上。

```text
恢复 a3
  AgentStore.load(c1, a3)
       │
       ▼
  读取状态、定义、消息和未决执行
       │
       ▼
  assembleSession(a3)
       │
       └── 有新的执行请求时 ── TargetResolver(c1, a3, action)
                                  │
                                  ▼
                               Host + target binding

切换到 device-d3：改变新执行的目标解析；a3 的存储和身份不变
```

读取、列举或恢复会话不需要先取得 Host，不需要调用携带 initialState 的临时
`harness.host()`。Host 离线不影响读取已提交的会话状态。

`TargetResolver` 是执行目标解析契约，放在现有 harness/server 装配边界中，
不要求新增一个服务。返回值包含 Host 及稳定的目标标识。具体设备、目录和授权
仍由 backend 的 conversation/runner 模块解析。

执行创建时固定 target binding。后续 status、stdin 和取消都从执行记录的
binding 找原目标，再检查当前调用权限；不能重新解析“当前 Host”后把旧操作
发送到新机器。Host 对象缓存是实现细节，持久化身份不能使用 JS 对象引用。

命令的 `storage` 继续使用 shell 层的 `CommandStorage` 契约，由产品按
`scopeId + sessionId + command namespace` 注入。同一节点切换 Host 后使用同一份
命令状态；访问其他节点的状态必须通过明确的节点操作，不通过 key 绕过归属。
不再并存同职责的 `HostStore` 和 `CommandStorage` 接口。

## 原子提交契约

`AgentStore` 是 agent 域的持久化端口。节点级的 store 是它绑定 scope/session
之后的视图，不是另一套实现。Backend SQLite 与测试用 memory adapter 都实现
同一套契约；需要持久恢复的部署必须显式配置持久 adapter。

契约必须表达以下领域提交：

| 提交 | 同一个原子单元内的内容 |
|---|---|
| 创建节点 | 节点身份、父关系、有效配置、初始状态、第一条 inbox 消息；由操作触发时还包括操作回执 |
| 保存 checkpoint | 变更的 block 行、截断、会话状态及持久化版本 |
| 接受/消费消息 | 唯一 messageId、队列状态；消费时与其进入 transcript/state 的变化共同提交 |
| 准备执行 | 已完整记录的工具调用、operationId、固定目标和输入摘要、派发状态 |
| 记录工具结果 | 该次工具调用结果、相关执行证据和同步的会话状态 |
| 完成节点 | 最终 checkpoint、完成状态和结果，以及策略要求发给接收者的完成消息 |

这些是有限的领域操作，可共用内部声明式 write batch。端口不暴露 SQL，
也不提供能在事务中 await 任意业务代码的通用回调。
异步 blob 写入在数据库事务前完成，事务只提交已存在的引用。

每节点的状态写入经过同一串行提交队列。提交带预期持久化版本，以检测迟到
checkpoint 或异步执行回调对新状态的覆盖。版本冲突必须重新读取并重新判断，
不能原样重放一个旧快照。这是单进程的状态一致性约束，不涉及多节点调度。

`load` 必须读到同一已提交版本：session state、block 行和队列之间不能混合
不同提交。关系操作涉及多个节点时，在同一 scope 的本地事务中处理必要的行；
不为整个 agent 树的每次 token 更新做全树 checkpoint。

流式文本允许按时间窗口合并落盘。以下边界必须等待持久化成功：接受消息的
确认、启动有副作用的操作前、将工具结果交给下一次推理前、发布完成结果前。
存储失败时停止推进该节点的相关动作并报告错误，不能吞掉写入错误后宣告完成。
浏览器可先显示流式临时内容，重连以已提交版本为准；该体验不代表执行已获确认。

## 同构生命周期与关系行为

节点的 turn phase、持久化生命周期和资源是否已装载是三个不同事实。
`idle` 仅表示当前没有推理 turn，不能自动等同于任务完成；卸载资源也不是任务完成。

所有节点支持同一组 create/open/send/steer/abort/close/resume 操作。
完成策略是显式配置，例如保持可接收消息，或在没有 turn、消息、唤醒和所等待
子任务时完成。产品默认让交互节点保持开放，让一次性委派任务在静止时完成。
同一策略可以用于任意节点，不在执行器中判断 `isSubagent`。

父子关系只决定操作授权、继承参数、等待关系、取消传播及完成结果接收者。
关系模块不拥有子节点的 provider、shell 或第二套 session 构造代码。

创建/恢复顺序：

```text
读取 scope 的节点与关系
        │
        ▼
按同一入口装载需要运行的节点，登记全部关系
        │
        ▼
恢复未决执行和已接受消息
        │
        ▼
开放节点调度，统一评估完成策略
```

加载关系的屏障防止某个父节点在其持久化子任务尚未登记前被判定完成。
不必为查询关系而实例化已经完成的全部节点。

完成与消息接受在同一串行化规则下竞争：先接受的消息必须参与完成判断；
先完成的节点拒绝普通 send，并通过显式 resume 重新开放。
每次完成有单调的 completion sequence，结果消息使用确定的唯一标识。
完成记录和消息写入同一 scope 的事务，进程退出不能产生“完成了但父节点永远
收不到结果”的间隙；消费重试也不会重复生成同一个完成消息。

恢复一个已完成节点保留身份和历史，进入新的活动周期。旧周期的迟到回调不能
关闭或推进新周期。Browser 断开仅移除订阅，不改变任意节点的生命周期。

所有节点发出统一的 `{ sessionId, event }` 事件。树关系单独传输。
transcript revision 按节点维护，持久化版本不与它混用；子节点使用同一个
transcript reset/patch 和进度渲染路径。产品可以把节点显示成主对话或子任务。

## 执行记录与结果未知

执行记录追踪实际操作，transcript 追踪模型已经收到的内容。两者通过内部
operationId 和 invocationId 关联，不从显示文本或 provider 的 toolUseId
推断执行身份；provider ID 仅保留为一次工具调用的协议关联字段。

执行记录保存恢复所需的参数或持久引用，不能只依赖可能被 compaction 移除的
transcript block。输入摘要用于拒绝同一个 operationId 对应不同参数，不代替
实际输入。凭证不进入记录；恢复观察或派发时仍经过产品的当前授权检查。
回执保存终态、必要返回值或引用和有限输出视图，不复制完整命令输出。
“已确认执行结束，但输出文件不可读”和“执行结果未知”分别表达，不能混为一谈。

工具调用与长任务有不同生命周期：

```text
a3 的 shell_exec 工具调用 t9
  创建执行 e7 ── 等待观察窗口 ── 返回 running(e7) ── 工具调用 t9 完成
       │
       └── e7 仍在 device-d2 上运行 ──────────────── 执行结束

后续 shell_status(t10) 查询 e7；不会创建第二个 shell 作业
后续 shell_write(t11) 是对 e7 的一次新输入操作，需要自己的操作身份
```

运行状态、传输状态和取消意图不能互相冒充。断线不能伪造成一个进程 signal
或证明命令没有执行；已确认进程退出也不能证明它没有留下部分效果。

建议执行状态契约：

| 状态 | 含义与恢复行为 |
|---|---|
| prepared | 派发意图已提交，尚未跨过派发标记；恢复可执行第一次派发 |
| dispatching | 派发标记已提交，可能已送达；恢复先查询证据，不能直接再执行 |
| running | 执行方确认已启动；状态查询仍使用原 target binding |
| finished | 有确认的最终结果；保存 exitCode/signal/result 等事实，非零退出不代表没有副作用 |
| not_started | 执行方明确保证未开始，或本地在调用执行方之前拒绝 |
| unknown | 无足够证据确定最终结果；保留原因、目标、最后确认状态和查询标识 |

`cancelRequested` 是独立意图。确认发送了取消请求不等于已经取消；取消也不代表回滚。
`unknown` 可以被后续证据解析为 running/finished/not_started，不是简单的终止错误。

派发流程：

```text
a3 准备执行 e7
  1. 原子提交 t9 的完整输入与 e7(prepared)
  2. 持久化 e7(dispatching)
  3. 调用执行方，携带 e7 和已固定的目标
  4. 持久化确认/结果，再推进相应工具调用与后续推理

在 2 与 3 之间退出：虽然可能尚未发送，恢复仍保守地查询；无证据则 unknown
在执行完成与结果提交之间退出：查询回执；查询不到则 unknown
```

执行方提供统一的观察结果：已启动、已结束、明确未启动、未知。
适配器没有可靠证据时必须返回未知。仅仅在内存 job 表里查不到不能表示未执行。
runner 协议携带稳定操作标识及观察结果；这里不要求引入任意 shell 的事务执行，
也不依赖具体机器生命周期实现。若回执只在执行方进程存活期间可查，恢复保证就
明确限于那个范围，执行方重启后缺失证据仍是 unknown。

重复执行的处理分两种有实质区别的能力：

| 能力 | 条件与保证 |
|---|---|
| 领域内原子去重 | 受控操作的状态变更、operationId 去重和结果回执处在同一存储事务；重复请求返回相同回执 |
| 外部操作观察 | 任意 shell、流式 stdin、外部进程等依赖执行证据；无证据时不自动重放 |

例如创建 a2 时，节点、初始消息和创建回执共同提交；同一个创建操作不会生成
第二个 a2。命令状态写入只有在命令的存储实现能把变更和回执共同提交时才可声明
原子去重。操作 ID 存在、一个 `idempotent: true` 标志或“先查再执行”都不构成保证。
这类事务由拥有相关数据的领域存储实现，不能横跨控制库、会话库和外部文件系统。

stdin 写入可能已经被远端进程消费。没有端到端去重协议时，不能在重连后自动
重发同一段输入。status 是观察，不重新启动执行；abort 记录取消意图并查询确认。

恢复 pending 工具调用时，先读取执行记录，使用已有结果、恢复观察，或生成明确
的 unknown 工具结果。未知状态通过框架提供的结构化视图及模型可读文本表达，
保留“可能已产生部分效果”的语义；不将所有中断统一成可重试失败。
框架不自动重放未知操作，模型的继续请求也不会自动重跑整段历史脚本。
模型主动提出的新命令仍是新操作；框架不能据此声称任意业务效果只发生一次。

生命周期 hook 的职责也受同一边界约束。会话内状态计算可以参与提交；需要写
外部系统的 hook 必须形成显式操作或有回执的消息处理，不能藏在可重试的
`after_tool_call` 回调里。结果提交后触发的普通观察者不参与成功与否的判定。

## 模块责任与复用边界

| 模块 | 单一职责 |
|---|---|
| agent/session | 单节点推理和 turn 状态机；不解释产品 conversation 或父子角色 |
| agent/server | 统一装配、节点资源、登记及动作入口；协议 binding 不拥有节点 |
| agent 的关系模块 | 关系查询、委派、完成策略和结果交付；复用节点动作 |
| agent/store | AgentStore 契约、媒体引用处理及持久化数据定义 |
| agent 的执行模块 | 派发边界、操作记录、恢复决策；不解释 bash 或具体设备 |
| shell | Host、命令、CommandStorage、执行状态和观察契约；不依赖 agent |
| host-virtual / host-remote | 执行适配及事实映射；不拥有会话持久化 |
| backend/storage | AgentStore/CommandStorage 的 SQLite 实现和本地事务 |
| backend/conversation | scope 的产品所有权和执行目标策略 |
| utils | 复用已有 SerialQueue、错误和异步原语；不放会话或执行领域代码 |

执行模块管理派发和证据，AgentSession 管理模型回合；它们不各自保存第二份
执行状态。transcript 中的工具视图是已经交付给模型的历史观察，不是执行记录
的实时副本，后续状态变化以新的观察表达。

不为这一设计添加工作流引擎、消息代理、通用事务框架或通用事件溯源系统。
领域内 inbox 和执行记录使用现有 scope 数据库；已有存储、工具和命令契约
做直接调整，消除重复路径。

## 可验证的完成条件

验收测试属于机制的 owning package；同一份节点契约测试在 root、child 和
grandchild 上参数化执行。受控 provider 和故障注入即可验证，不调用真实模型。

| 测试模块 | 必须证明的行为 |
|---|---|
| agent session-node conformance | create/restore/send/steer/model switch/事件/close/resume/dispose 对任意节点一致，harness 回调不因节点角色缺失 |
| agent store conformance | 任意提交中断后只能读到完整旧版本或新版本；追加、修改、截断、队列与完成均覆盖 |
| backend SQLite store integration | 根与多层后代使用同一行模型；迟到写入不能覆盖新状态；损坏数据明确失败 |
| agent tree lifecycle | 关系恢复后才调度；消息与完成竞争无丢失；完成通知重复投递无重复消费；归档后仍可查完整关系 |
| agent target routing | Host 离线仍可读会话；切换不改变存储；旧执行的 status/write/abort 使用原目标并重新检查授权 |
| agent execution recovery | 派发前、派发间隙、执行中、结果落盘前后退出；没有证据不重放；已提交结果不重复调用 |
| shell execution conformance | 工具返回 running 不等于作业完成；断线是未知而非虚构退出；取消请求与退出确认分离 |
| command/runner protocol integration | 相同操作标识的查询与回执语义；执行方丢失历史时为 unknown；stdin 不盲目重发 |

实现时先更新最高包边界契约，定义统一身份、节点与提交语义，再改运行时和
存储，最后贯通执行协议及恢复。每个机制只保留一种实现；相关现行文档按最终
行为同步重写，审查历史只记在 progress log。不会增加历史数据探测、迁移、
清理或兼容路径。代码未满足上述验收前，本提案保持未实现状态。

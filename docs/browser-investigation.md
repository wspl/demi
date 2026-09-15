# Browser 行为问题与参考实现核查（临时）

更新：2026-09-15。Demi 基线：`8c9f5f6a`。

这是临时调查记录，不是选定设计，也不表示问题已经修复。权威设计仍是
[Browser](demi-next/browser.md)；包职责见 [Package boundaries](package-boundaries.md)。
本记录集中保存问题、证据、参考实现和后续核查项，完成修复后移除或归档。

修复依据：针对有问题的 API 对照 Codex 和 Playwright 的实现；有冲突时
优先参考对应 Codex API 的已验证行为。修复在现有架构内进行。

阅读顺序：[拟修改清单](#拟修改清单) → [问题清单](#问题清单) → [参考实现核查](#参考实现核查) →
[后续核查与修复](#后续核查与修复)。

## 拟修改清单

本节供审阅，尚未实施。范围是下面逐项列出的 API 修复、必要的契约与帮助同步、
回归验证，以及原有测试程序问题。参考版本见下文“来源与版本”。
Codex 行为已经确认的部分列出具体改法；证据不足的部分单列，不能在实施时
自行补成另一个功能。表中的“验收”是后续检查要求，不代表已经通过。

### 已有依据的 API 改动

| 问题 | 拟修改的具体行为 | 参考依据 | 验收例子 |
| --- | --- | --- | --- |
| B01：fill | 按原生控件类型处理。date/time 等使用对应值设置路径并检查浏览器接受结果；普通文本按 Codex locator fill 的替换输入路径处理；去掉先点击再对所有类型 InsertText 的统一做法 | C `playwright_locator_fill`、其 injected fill 与文本输入路径 | 合法原生日期写入；非法日期报错；文本替换、空值、只读控件；页面阻止 beforeinput 时与 C 对照。不给所有 fill 额外加最终值相等断言 |
| B02：定向 key | 输入前聚焦指定元素，保留已有选择区；不能忽略传入目标而对旧焦点发送 | C locator press、`focusLocator` | 焦点原在文本框，定向 checkbox 按 Space 后只改变该 checkbox |
| B03：组合键 | 支持已设计的 ControlOrMeta+A 等组合键，按 Host 平台解释 ControlOrMeta，正确按下与释放；非法键名在输入前失败 | C locator press 及其按键解析/分发路径 | 全选、组合修饰键、字面加号、未知键；成功、失败、取消后没有遗留按下的键 |
| B04：有目标 type | 按 C pressSequentially：聚焦而不先点击，不主动重置已有选择区；逐字符输入；输入中目标被替换、frame 改变或失焦时停止，不继续打到别处 | C `playwright_locator_press_sequentially` | hello 中选择 ll 后输入 XY 得到 heXYo；未聚焦但保留选择区；输入中脚本转移焦点。不会把整串 InsertText 说成逐字符输入 |
| B05：check | 先读取 checked；已满足则不点击，否则点击后再读；未达到要求报错；radio 取消勾选报错；使用相应原生/ARIA 状态判定 | C locator check、`readCheckedState` | 已勾选不反转；页面阻止勾选会报错；radio 和支持的 ARIA checkbox。不会用 AX setValue 绕过点击来冒充 check |
| B06：已确认动作的前置条件 | click 检查 visible、enabled 并走指针准备；fill/type 检查 visible、enabled、editable；key 检查 visible、enabled 后聚焦；select 检查 enabled。取消这些动作共用鼠标命中检查的做法 | C 各动作入口。move/scroll 的具体条件另列核查项 | 可聚焦但被遮挡的输入不因鼠标命中失败而拒绝 fill；select 不额外添加 P 的 visible 要求；键盘不要求鼠标可点击 |
| B07：click 位置准备 | 滚动后观察位置稳定性，再复查控件状态并按当前位置发送；参考 C 的有上限位置观察及适用的 frame 命中检查，修正当前一次取点后直接输入的路径 | C `resolvePointerActionTarget`、`performPointerAction` | 滚动导致布局变化、移动目标、当前已支持 frame 中的坐标。不会把 P 的整套事件拦截机制直接列为新增要求，也不承诺绝对无竞态 |
| B08：wait 状态 | visible/hidden 使用实际显示状态和尺寸；attached/detached 使用节点连接状态；现有 enabled 等待复用与动作相同的 enabled 判定 | C locator waitFor、elementState/isEnabled | visibility:hidden 不能满足 visible；节点删除满足相应 hidden/detached；fieldset/ARIA 禁用不能仅靠当前节点 disabled 属性判断 |
| B09：select | 按 C 遍历选项、匹配 value/label/index；未出现或未启用时在期限内等待；按原生单选/多选设置并发 input/change。重复选项和单选接收多个候选时也按 C 的顺序匹配规则调整，去掉我们额外的“必须唯一/单选必须只传一个候选”限制 | C locator selectOption、其 injected selectOptions | 延迟出现的选项、禁用 option/optgroup、重复值、单选多个候选、多选；返回实际选中值。现有 value/label/index 三种 CLI 参数仍互斥，不另加新参数形式 |
| B10：label/text/name | label 按控件标签关系匹配，text-match 按页面文本匹配，role/name 按无障碍名称匹配；纠正 schema 中把 label 描述成 accessible name 的说明 | C 对应 selector 引擎 | 原生日期通过标签定位；页面文本和 aria-label 不同；label for、包裹 label、aria-labelledby |
| B10：单目标歧义 | 对单目标动作参考 C 的实际规则：总共一个匹配时使用它；多个匹配但只有一个可见时使用该可见元素；仍有多个可见匹配时报歧义。find/read --all 继续返回集合 | C `querySelectorStrictWithVisibleFallback`；这是与当前“任何多匹配都失败”规则的明确变化 | 同名按钮一个隐藏一个可见；两个可见同名按钮；集合读取不丢隐藏匹配 |
| B11、B12：错误映射 | 分开报告不存在的 tab、浏览器关闭、零匹配、多匹配、历史边界、参数错误、驱动错误、文件错误；已知底层原因不再统一折成 browser_failed 或 invalid_arguments | C 找不到 tab 的错误、locator 诊断；Demi 已有错误契约 | 错误 tab ID 不影响正常 tabs；read 零匹配不报 ambiguous；无上一页不报参数错误；文件已存在/不可写保留原因 |
| B13：动作进展 | 接通现有 not_started/completed/unknown 到实际动作路径及超时错误；补充与本次目标有关的定位和状态诊断。这个三值字段来自 Demi 已有契约，不声称来自 Codex | C `actionError` 的相关诊断；现有 Demi 进展契约 | 输入前失败、输入后等待失败、断连后无法确认，三者可区分；不增加另一套 clicked/submitted 标志，不按错误自动重放 |
| B14：附带 URL 查询 | 普通动作不再仅为补充 URL 而额外查询页面，并让该查询把已完成动作变成失败；有导航等待的调用按对应等待结果报告 | C 普通动作与显式观察分开的返回方式 | 动作执行成功但随后 metadata 查询不可用时，不错误宣称动作没发生；明确的导航等待失败仍正常报告 |
| B16：普通 click 内部等待 | 参考 C 在输入前建立有上限的加载观察；“没等到加载开始/完成”属于内部等待结束，不自动推导业务失败。显式 wait 的超时仍是错误；整个 Demi 调用的截止时间和取消契约仍有效 | C `clickPoint`、`waitForPageLoadEvent` | 不触发导航的点击；导航停滞；输入前失败；显式等待超时。不会把内部等待改成必须等 load 的 P 保证 |
| B24：显式 URL 等待 | 保留现有 wait --url 和动作 --wait-url 参数。按 C waitForURL/expectNavigation：当前 URL 已匹配即可满足 URL 条件；动作 --wait-url 先建立等待再执行动作，即使条件原已满足也仍执行动作。后续不再要求必须出现新 URL/loader | C waitForURL、客户端 expectNavigation | 已在 dashboard 后点击且无导航；立即跳转、同文档跳转、延迟跳转；显式条件一直不满足时超时。这一项会改变旧设计的“必须新导航”规则 |
| B25：URL 通配符 | 修正 globset 默认语义与当前设计/Codex 的差别：* 不跨 /，** 可以跨 /，其他字符按字面匹配 | C `yG`；Demi 当前 `Glob::new` 使用 globset 默认 literal_separator=false，且支持额外模式语法 | /a/* 不匹配 /a/b/c；/a/** 可以；?、方括号、花括号按字面处理。这是本次列清单时确认的同一 URL API 问题 |
| B17：观察文本 | inspect/find 文本输出显示结果里已有的 states，与 JSON 一致；补清实际值是否已由观察采集，缺失值先按下列核查项处理 | C AX 状态输出；Demi 已有 browserNode.states | checked、disabled、expanded 等在文本与 JSON 一致；不自动给每个成功动作追加整页或截图 |
| B18：短引用 | tab/ref 改为便于复制的短句柄；保持现有会话、tab、文档和节点失效边界，旧引用不能误指新节点。具体编号分配方案须先写清，不能从 C 数字索引外观猜其隐藏实现 | C 短数字索引、`targetForElement` 失效检查；Demi 现有身份约束 | 多 tab、再次 inspect、节点替换、整页刷新、浏览器重启后的旧引用；不增加旧数据迁移或第二份 tab 注册表 |
| B19、B20：帮助与示例 | 从现有 schema/manifest 补齐 1000 上限、超时、键名和上述变化；示例使用真实参数（当前 key 为 --key）；逐项标明未实现的无目标输入、wait --load 等，不将其当成本轮自动新增能力 | C/U 当前 API 说明；Demi 现有命令声明机制 | 帮助、schema、Rust 生成绑定、设计示例一致；修复过的行为在帮助中可知，未实现扩展不宣称可用 |

### 尚不足以确定具体改法的部分

这些也是完整清单的一部分，但当前只能安排核查，不能冒充已经确定的修复：

| 问题 | 还缺什么 | 本轮清单明确不作的假设 |
| --- | --- | --- |
| B01：普通文本的空值与事件拦截 | C 普通文本路径先发送可取消的 paste，再按控件处理替换；还需最小实验确认空字符串、页面取消 paste、contenteditable 的结果 | 不自行增加“页面拒绝输入时强制写值”的后备路径；只处理 fill 所需的纯文本，不新增富文本或公共剪贴板 API |
| B06：move/scroll | 确认当前 Codex 对应入口的控件条件、滚动目标和遮挡处理，再补该动作的条件表；不能因 P hover 不检查 enabled 就声称 C 一定相同 | 不套用 click 条件，也不凭 P 表格直接决定 C 行为 |
| B15、B16：open/goto/reload/back/forward | C back/forward/reload 使用尽力加载等待已确认，但 goto 有独立路径；还要对照断网、导航失败、重定向和同文档事件的实际返回，才能把“返回成功意味着什么”写成完整契约 | 不先承诺所有网络错误页都转换为命令失败；也不忽略浏览器已明确报告的导航错误 |
| B17：输入值观察 | 当前 browserNode 只有 role/name/states 等字段，需核对输入值目前是否进入 name/states，及 C 如何区分值与名称、保护密码 | 不在没有核查时新增第二份重复值或泄露受保护值 |
| B18：编号分配 | 写出短句柄的分配与代际失效方案，确认旧引用不会在新浏览器中被重新解释；C 完整 AX 编号分配不可见 | 不凭空指定会复用的 tab-1/e1 规则，不增加持久化迁移 |
| B20：未实现接口 | 无目标 type/key、wait --load 等属于现有设计与当前能力的差距，逐项列明；若要在此次落地，须单独补入具体参数、行为和验收清单 | 不借“帮助同步”偷偷新增命令、参数或完整未来能力集 |
| B21：原生日期内部 AX 子控件 | 复现具体超时阶段，区分节点定位、状态检查、聚焦和输入；B01 修复原生 date 不代表这一项已解决 | 不预测哪些控件“不支持”，不自动降级为文本日期 |
| B22、B23：模型表现 | 工具修好后仍需区分页面确认框说明、观察是否充分与模型遗漏。当前只保留样本及通用用法说明的对照，不添加模型行为规则 | 不添加固定失败次数、浏览器专用纠偏循环或特定业务提示 |

### 测试程序与交付

| 问题 | 修改或执行要求 |
| --- | --- |
| H01 | 测试仅保留一个会话控制客户端；观察使用只读通道，避免接管连接 |
| H02 | 仅在另有授权的真实模型评估中固定并核对每轮实际模型/推理参数；本次修复回归不调用真实模型 |
| H03 | 测试页面服务存活到完整任务结束，成功、失败、取消共用清理；不把 send 返回当成任务结束 |
| H04 | 从配对设备和 Cloud 各自预检测试地址；环境故障与浏览器结果分开记录 |
| H05 | 原生日期样本保留；文本日期对照单列；测试页面写清确认框每次提交需重勾；最终结果查测试服务记录 |

代码影响范围为现有 `crates/demi-commands/src/browser/`，以及确有协议/帮助
变化时的 `packages/browser-protocol`、生成的 Rust 绑定和
`packages/coding-agent/src/commands/browser/group.ts`。设计规则在
`docs/demi-next/browser.md` 同步更新。只有经上述问题确认需要修改时才改相关
驱动依赖；不预先扩大为驱动重构。

验收使用隔离页面、协议/动作回归和真实浏览器，不调用真实模型。涉及 Host 的
最终结果在配对设备与 Cloud 都检查；按项目要求更新承载变化的构建产物和
Cloud 镜像，在整个修复批次完成后重启运行中的 backend/web，再提交并推送。

本清单不包含完整引入 Playwright、增加 Node 浏览器服务、更换原生架构，
也不包含新增浏览器 UI、自动截图、自动重放动作、模型纠偏框架或未来命令全集。
现有默认超时和最大时限不在修改范围内；也不顺带引入新持久化或迁移机制。
若核查发现必须增加未列出的行为，先补到清单中明确说明，不能夹带进代码。

## 证据范围

- **实测**：此前通过真实 Demi backend/provider/agent/browser 调用
  `gpt-5.6-luna`、`max`，操作隔离的表单页面；不是通用网站基准。
- **代码确认**：读取当前实现能确认分支行为，但新增发现尚未全部运行复现。
- **待验证**：有风险或与设计不符，需要参考源码、测试或最小复现才能定论。
- 原始实测资料保留在本机 `.cache/browser-luna-results.json` 及其对应转录中；
  本文不保存凭证、页面私有内容或完整转录。

## 问题清单

| ID | 状态 | 情况与例子 | 当前证据 / 需要查明的事 |
| --- | --- | --- | --- |
| B01 | 实测、代码确认 | 原生日期输入填写后仍为空，工具返回 completed | `tab.rs::fill` 对所有可编辑输入使用选择内容加 InsertText；对照原生输入支持和失败判断 |
| B02 | 实测、代码确认 | 定向复选框按空格，实际发送给原来的焦点 | `actions.rs::Key` 调用 chromiumoxide `Element::press_key`，后者直接转给 Page，不聚焦 |
| B03 | 实测、代码确认 | ControlOrMeta+A 等组合键不能按设计使用；模型猜测多个键名 | 当前使用单键查表；对照组合键语法、键名、释放与错误 |
| B04 | 代码确认 | type 先点击，可能改变原有光标；整串 InsertText 缺少逐字符键盘事件 | `actions.rs::Type`；对照 pressSequentially/type 与 fill 的边界 |
| B05 | 代码确认 | check 点击后不检查最终状态；页面阻止勾选仍可报告成功 | `actions.rs::Check`；对照原生与 ARIA 控件、已经符合状态和取消勾选 |
| B06 | 代码确认 | fill/key/select/hover/scroll 都要求通过可点击检查 | `tab.rs::ready_element`、`actionability.js`；如悬停禁用按钮查看提示被阻止 |
| B07 | 代码确认 | 点击缺少位置稳定检查；坐标检查后目标移动可能点错 | 同上；对照命中检查、重定位与何时允许重试 |
| B08 | 代码确认 | wait visible 主要检查宽度；visibility:hidden 也可能通过 | `actions.rs::Wait`；enabled 只检查 disabled 属性，缺少完整语义 |
| B09 | 代码确认 | select 对稍后加载的选项直接报错；缺失、歧义、禁用共用一句错误 | `select-options.js`；对照等待选项和单选/多选行为 |
| B10 | 代码确认 | label 靠 AX role 白名单；text-match 匹配 accessible name | `observation.rs::resolve`；日期等控件可能漏匹配，文本、标签、名称语义混淆 |
| B11 | 实测、代码确认 | 错误 tab ID 返回 browser_closed，但 tabs 仍有活页 | `resources.rs::execute/invoke` 共用 Closed |
| B12 | 代码确认 | read 零匹配被归为 ambiguous_target；历史边界、文件错误等被合并 | `actions.rs::Read/Back/Forward`、`resources.rs::invoke`；核对全部错误出口 |
| B13 | 实测、代码确认 | timeout 只有 code/message，没有设计要求的动作进展 | `resources.rs::invoke`；不能据此判断点击是否发生 |
| B14 | 代码确认、因果待复现 | 普通动作完成后还查 URL，附带查询失败能使已完成动作返回失败 | `actions.rs::action_result`；不能声称它就是此前每次超时的原因 |
| B15 | 实测、代码确认 | 断网刷新曾返回 exit 0，随后 inspect 是 chrome-error 页面 | 此证据来自排除于 Max 评分的 Cloud 后续轮；导航故障本身仍有效 |
| B16 | 代码确认、待验证 | goto/reload/back/forward 的等待和失败处理不一致 | back/forward 发出导航请求后直接读 metadata；对照事件订阅、重定向、同页导航、HTTP 错误与网络错误 |
| B17 | 代码确认 | 文本 inspect/find 丢掉 JSON 中已有的 states | `output.rs::render`；如 checked、disabled 对模型不可见 |
| B18 | 实测、代码确认 | tab/ref 很长，复制错误造成额外调用 | tab 为 generation + CDP target ID，ref 为 UUID；对照短引用及失效规则 |
| B19 | 实测、代码确认 | inspect limit 上限 1000 未在帮助中清楚说明；模型尝试 1500 | `browser-protocol`、命令帮助；只补接口特有信息，不写常识教程 |
| B20 | 代码确认 | 部分设计示例与当前可调用参数不同 | key 示例参数形式、无目标 type/key、wait --load 等需要逐项核对；设计已声明包含未来扩展，未实现示例本身不等于矛盾，需要区分明确延期与已承诺动作的实现偏差 |
| B21 | 待验证 | 原生日期的 AX 子控件动作反复超时 | 尚不能判断每次是定位、可操作条件、焦点还是控件自身行为；不预判所有此类控件都不支持 |
| B22 | 模型/页面语义 | 模型将每次提交需勾选的确认框当成持久设置，反复保存 | Cloud 经一次语义解释后完成；区分工具缺陷、页面说明不足与模型误解 |
| B23 | 模型表现 | 漏勾确认、重复操作；也能识别同名记录并按库存纠正数量 | 不能将所有额外调用归因于工具，不能用这个小样本推出通用能力结论 |
| B24 | 代码确认、漏事件场景待实测 | --wait-url 用动作前后的 frame 快照和轮询判断变化，未按设计预先观察导航事件 | `actions.rs::action_result/wait_url`；若匹配导航后迅速跳回原页，轮询可能漏掉中间导航；不能用最终 URL 或 loader 差异完全代替事件 |
| B25 | 代码确认 | URL 通配符使用 globset 默认规则，* 可以跨 /，且 ?、方括号和花括号具有额外模式语义 | 与现有设计和 C `yG` 的 * / ** / 其余字面规则不同；未运行新增浏览器实验 |
| H01 | 测试程序问题 | 第二个控制客户端打开同一会话会接管连接 | 使用一个控制客户端，观察走只读接口；不要把接管当 browser 故障 |
| H02 | 测试程序问题 | Cloud 后续轮 thinking=null，不能算 Max 成绩 | 每轮固定完整模型选择，并从实际请求核对 |
| H03 | 测试程序问题 | send 返回或某轮结束不代表后续任务生命周期结束，页面服务提前停掉 | 服务独立存活至确定任务结束；成功、失败、取消统一清理 |
| H04 | 测试程序问题 | 配对设备首轮测试地址错误；后续服务停启污染耗时 | 从目标 Host 预检可达性；环境失效单列，不算模型失败 |
| H05 | 测试设计限制 | 原生日期失败后用了文本日期对照；确认框重开即清空 | 保留原生失败样本；对照组独立命名；确认框语义在页面写清；最终状态查服务端记录 |

## 参考实现核查

核查已完成到下述范围。产品 API、内部实现、测试框架断言和模型指导
分别记录；没有把任何一层的能力当作另一层的保证。

### 来源与版本

| 代号 | 读取的材料 | 用途与限制 |
| --- | --- | --- |
| P | [Playwright 源码快照](https://github.com/microsoft/playwright/tree/500c9c822ce7664539a4c8a88810048dfe090c3b)、官方 API 文档、仓库回归测试 | 可以核对公开实现及测试覆盖；本机实验使用 `playwright-core 1.62.1`，不把源码快照与安装版本声称为同一构建 |
| C | 本机产品内 `@oai/browser-desktop 0.1.1` 的 `browser-client.mjs`、`browser-service.mjs` | 核对当前随产品分发的实际浏览器适配代码；它不是原版 Playwright 服务端，也不能代表所有 Codex 版本或浏览器后端 |
| U | 本机 `@oai/cua 0.2.4` 的 API 包装与随包文档 | 区分 AX 操作和 locator 操作，核对实际提供给调用者的说明 |
| O | [Codex 公开仓库快照](https://github.com/openai/codex/tree/7f01a84effccef40d4726c3ca12e6c839ec98d7a)、[官方 Browser 介绍](https://learn.chatgpt.com/docs/browser) | 用于确认公开范围；没有从公开仓库获得完整桌面浏览器动作实现，不能据此断言产品没有某功能 |

C 的本机源文件：
[客户端](</Applications/ChatGPT.app/Contents/Resources/cua_node/lib/node_modules/@oai/browser-desktop/scripts/browser-client.mjs>)、
[服务端](</Applications/ChatGPT.app/Contents/Resources/cua_node/lib/node_modules/@oai/browser-desktop/scripts/browser-service.mjs>)。
这两个文件的 SHA-256 分别为
`b9b9bc2319d5ee6aa0b1e481d63bb2130d28102fc7c9080803ab5552185d9037`、
`c96dbf28f0854b00b0cf79e936adfb3754ecb6b0714f94c50a594d3b68940e3e`。
本机链接随产品更新可能失效；本文只概括行为，不复制其实现或提示词。

U 的文档入口是
[CUA API 与工作流](</Applications/ChatGPT.app/Contents/Resources/cua_node/lib/node_modules/@oai/cua/docs/tinysky-alt-core-cua-repl.md>)、
[其他浏览器 API](</Applications/ChatGPT.app/Contents/Resources/cua_node/lib/node_modules/@oai/cua/docs/tinysky-alt-other-browser-apis.md>)。
还读取了本机缓存的 browser 插件 `26.903.71938`；缓存只能作补充，
与当前产品文件有差异时，以当前文件和实测为准。

### 填写、键盘与控件状态

| 问题 | Playwright | 当前 Codex 浏览器实现 | 对 Demi 的结论 |
| --- | --- | --- | --- |
| B01、B21：原生日期 | `fill` 按原生 input 类型分支；date 等设置原生 value，检查浏览器是否接受，再发 input/change；普通文本选择内容后走键盘插入 | locator 的特殊类型分支使用相应 injected fill 逻辑；AX `setValue` 也有原生 setter，拒绝非法值并恢复原值；普通文本 locator fill 走虚拟剪贴板路径 | 已有成熟的原生类型处理，不需要另造日期格式猜测器或预测失败控件。AX 日期内部子控件超时的具体原因仍未定位 |
| B02、B03：定向按键与组合键 | locator press 先聚焦目标，再解析组合键、按下和释放；支持 ControlOrMeta。page keyboard 则作用于当前焦点 | locator press 聚焦目标；AX pressKey 没有元素参数，作用于当前焦点，使用 xdotool 风格键名 | 定向按键与当前焦点按键是两种明确语义。Demi 不能接受目标后又忽略它；键名也不能混用两套语法 |
| B04：输入与光标 | type/pressSequentially 聚焦后逐字符输入；已聚焦时保留选择区，原先未聚焦的 input 可重置到开头；键表外字符可退回 insertText | AX typeText 向当前焦点整串 InsertText；locator type 使用粘贴路径；locator pressSequentially 另有逐字符实现，并在过程中检查目标、焦点和 frame | 不能把三个 API 都称为同一种“模拟打字”。撤回“Playwright 总保留原来的光标”和“所有字符都有完整键盘事件”的泛化 |
| B05：保证勾选状态 | 已符合状态就返回；否则点击后读 checked，未改变则报错；radio 不允许通过 uncheck 取消 | locator check 同样前后检查；AX setValue 对原生 checkbox 直接设值，是另一种操作 | check 必须验证它承诺的 checked 状态。不能拿 AX 直接设值成功证明点击路径正确 |
| B09：选择选项 | 选择前检查 visible、enabled；选项未出现、不可用时在期限内重试；支持值、标签、索引及多选语义 | locator selectOption 检查 enabled，使用 injected selectOptions，并由适配层重试；该入口没有 P 相同的 visible 检查 | 优先参考 Codex selectOption 的前置条件、选项匹配和重试；不要自己把缺失、禁用和定位歧义折成同一句错误 |

P 的对应实现：
[fill、selectOptions 与 focusNode](https://github.com/microsoft/playwright/blob/500c9c822ce7664539a4c8a88810048dfe090c3b/packages/injected/src/injectedScript.ts#L816)、
[服务端 fill](https://github.com/microsoft/playwright/blob/500c9c822ce7664539a4c8a88810048dfe090c3b/packages/playwright-core/src/server/dom.ts#L604)、
[type、press 与 check](https://github.com/microsoft/playwright/blob/500c9c822ce7664539a4c8a88810048dfe090c3b/packages/playwright-core/src/server/dom.ts#L780)、
[键盘事件和组合键](https://github.com/microsoft/playwright/blob/500c9c822ce7664539a4c8a88810048dfe090c3b/packages/playwright-core/src/server/input.ts#L111)。
已对照 [type 光标回归测试](https://github.com/microsoft/playwright/blob/500c9c822ce7664539a4c8a88810048dfe090c3b/tests/page/elementhandle-type.spec.ts#L20)、
[fill 测试](https://github.com/microsoft/playwright/blob/500c9c822ce7664539a4c8a88810048dfe090c3b/tests/page/page-fill.spec.ts)、
[check 测试](https://github.com/microsoft/playwright/blob/500c9c822ce7664539a4c8a88810048dfe090c3b/tests/page/page-check.spec.ts)、
[select 测试](https://github.com/microsoft/playwright/blob/500c9c822ce7664539a4c8a88810048dfe090c3b/tests/page/page-select-option.spec.ts)。
C 对应服务端的 `playwright.fill`、`playwright.pressSequentially`、
`playwright.check`、`focusLocator`、`set_value` 与粘贴入口 `uo`。

一个重要边界：**动作自己的承诺不等于页面业务完成**。
check 明确承诺勾选状态，原生日期 fill 检查浏览器是否接受该值；
普通文本 fill 并没有统一附加“最终 value 必须等于输入”的断言。
本轮实验中，页面阻止 beforeinput 时，Playwright fill 返回成功而值仍是 old。
不能把前面提出的“所有 fill 都做最终相等断言”说成 Playwright 的既有方案。
保存订单是否成功仍需读取页面或应用结果。

### 定位、可操作条件与重试

Playwright 先按动作确定条件，再进行输入。例如 hover 不要求 enabled，
fill 要求 visible、enabled、editable，但不要求鼠标能命中；
click 则需要 visible、stable、enabled 和接收鼠标事件。
因此，Demi 的统一“可点击”前置检查会错误拒绝悬停禁用按钮等合法操作。
完整条件见 [官方 actionability 表](https://playwright.dev/docs/actionability)。

P 的指针路径不仅先检查坐标，还安装事件命中拦截器，覆盖检查完成到输入
发出的间隙；条件未满足时在同一截止时间内重新定位或等待。
这不能解释成“点击没有业务结果就再点一次”。Codex 复用了 injected 的
元素状态和定位逻辑，但指针、frame 坐标、截止时间由自己的服务端适配层处理，
不能因两者都有 click 方法就假定实现一致。对应 B06、B07、B08。
源码见 [指针动作与重试](https://github.com/microsoft/playwright/blob/500c9c822ce7664539a4c8a88810048dfe090c3b/packages/playwright-core/src/server/dom.ts#L317)、
[命中拦截](https://github.com/microsoft/playwright/blob/500c9c822ce7664539a4c8a88810048dfe090c3b/packages/injected/src/injectedScript.ts#L1108)；
C 的对应入口是 `clickLocator` 及其指针准备路径。

P 将文本、标签和 accessible name 分别计算：getByText 使用页面文本，
getByLabel 使用控件的标签关系，getByRole 的 name 使用无障碍名称。
Codex 的 locator 层使用相应 selector 引擎，还处理 frameLocator。
这直接否定 B10 中用 AX role 白名单代替标签关系、用 accessible name 代替文本
的做法。应参考对应 Codex 定位 API 的语义修正实现。
依据：[locator API](https://playwright.dev/docs/api/class-locator)、
[injected 选择器注册与实现](https://github.com/microsoft/playwright/blob/500c9c822ce7664539a4c8a88810048dfe090c3b/packages/injected/src/injectedScript.ts)、
[role 与名称计算](https://github.com/microsoft/playwright/blob/500c9c822ce7664539a4c8a88810048dfe090c3b/packages/injected/src/roleUtils.ts)。

### 导航、超时与动作进展

例子：点击一个链接，服务器收到请求后一直不发送响应。鼠标已经点下，
但新文档还未提交。这时“点击没发生”和“点击发生、等待导航超时”会导致
调用者作出完全不同的下一步，不能都只剩一句 timeout。

| 操作 | Playwright | 当前 Codex 浏览器实现 |
| --- | --- | --- |
| 普通 click | 默认等待由动作触发并被跟踪到的顶层导航；可以在点击完成后超时。call log 保留 click action done 和随后等待导航的记录；noWaitAfter 可跳过该等待 | locator click 最终走自己的 clickPoint；先注册页面加载观察，再输入。公共 waitForPageLoadEvent 将等待开始或完成的异常作为尽力等待结束处理，策略阻止错误仍会抛出；不是 P 的严格导航保证 |
| AX click 后观察 | P 没有同名 AX API，不能直接对应 | 动作记录待观察的导航状态；AX/screenshot 捕获再做有上限的导航、首帧和状态稳定等待。等待部分条件结束不等于业务完成 |
| goto | 在发导航前准备观察；跟踪目标文档、重定向与导航错误，默认等 load；HTTP 404/500 可正常返回 Response，连接失败则报错 | 有独立导航处理器，跟踪主 frame、同文档事件、历史与错误。不能用普通 click 的等待规则推导 goto，也未在本轮完成其全部故障实测 |
| back、forward、reload | 走相应导航及生命周期等待 | 在发送历史导航或 reload 前建立加载观察，但使用上面的尽力等待函数。因此仅看到返回成功，不能推出页面完成加载 |
| waitForURL / expectNavigation | waitForURL 在当前 URL 已符合时可直接等待指定 load state；它不是“必须发生一次新导航” | expectNavigation 先建立 URL 或 load-state 等待，再执行回调并合并结果；继承当前条件可能已满足的边界，不自动证明发生了新导航 |

P 的依据：
[动作关联导航等待](https://github.com/microsoft/playwright/blob/500c9c822ce7664539a4c8a88810048dfe090c3b/packages/playwright-core/src/server/frames.ts#L191)、
[SignalBarrier](https://github.com/microsoft/playwright/blob/500c9c822ce7664539a4c8a88810048dfe090c3b/packages/playwright-core/src/server/frames.ts#L1868)、
[goto](https://github.com/microsoft/playwright/blob/500c9c822ce7664539a4c8a88810048dfe090c3b/packages/playwright-core/src/server/frames.ts#L689)、
[waitForURL](https://github.com/microsoft/playwright/blob/500c9c822ce7664539a4c8a88810048dfe090c3b/packages/playwright-core/src/client/frame.ts#L198)、
[官方导航说明](https://playwright.dev/docs/navigations)。
C 的依据：客户端 `expectNavigation`；服务端 `clickPoint`、
`waitForPageLoadEvent`、`navigate_tab_reload/back/forward`、导航处理器 `ws`、
`markActionCompleted`、`capture`。源码中等待的具体毫秒值属于当前实现参数，
不直接变成 Demi 的产品规则。

对应 B13–B16，修正前面的两个提案：

- 不能宣称“成熟实现都只等输入分发，所以不可能事后超时”。Playwright 本身
  就有这种超时；要保留已发生动作的证据，不能直接自动重放。
- 不能仅靠普通等待或 Codex expectNavigation 的名字，认定它完整替代了
  Demi `--wait-url`。现有设计已明确要求动作开始后出现匹配导航，动作前已匹配
  不算成功；单独 wait --url 则检查当前条件。这是已确认的差异，后续优先按
  Codex 对应 API 调整契约；不因旧设计更严格就自行决定保留。

B14 仍是另一件事：Demi 成功动作后读取 metadata 可能失败。
这条附带查询应怎样报告，不能与等待本次动作触发的导航混为一谈。
B15 的断网刷新问题仍成立，参考实现的尽力等待不自动成为 Demi 的设计选择。

### 错误、观察与引用

- **B11、B12、B13：错误需要保留可行动信息。** P 使用异常类型、定位错误和
  带阶段信息的 call log，没有与 Demi 完全相同的三种动作进展协议。
  C 的 `actionError` 在失败时尝试追加匹配元素数量及可见、禁用等诊断；
  诊断本身失败时保留原始错误。U 对不存在的 tab 明确报找不到 tab。
  应参考 Codex 修正 Demi 的错误映射与诊断，并同步核对动作进展契约；
  不能声称现有这套字段是照搬 P。
- **B17：观察必须传递已有状态。** P 的 ARIA snapshot 包含 checked、disabled
  等状态；C 的 AX 输出包含状态、可设置值信息和层级，并支持差量输出。
  Demi JSON 已有而文本丢弃的 states 应由渲染层修正。成功动作不必全部附上
  整页；失败诊断和显式观察可以分别承担责任。
- **B18：短引用依赖作用域和失效检查。** P 的 ARIA ref 使用带前缀的递增编号，
  仅在元素身份及相关语义仍符合时复用。C 的 AX 接口展示数字索引，查找时检查
  当前会话、tab 和文档身份，旧页元素会报 stale/previous page。
  其完整 AX 编号分配在所读 JavaScript 中不可见，不能虚构全部复用规则。
  Demi 可以选择短显示句柄，但仍必须满足现有设计的跨浏览器代际不误指规则。
- **B19、B20：接口说明与能力要一致。** P 有可查询的 API 和错误，C/U 在入口
  提供当前 API 说明。Demi 应从已有 manifest/schema 生成真实能力与限制，
  检查 type/key/wait 的设计示例；不是额外写一套常识教程。

P 的依据：[ARIA 引用与输出](https://github.com/microsoft/playwright/blob/500c9c822ce7664539a4c8a88810048dfe090c3b/packages/injected/src/ariaSnapshot.ts#L220)、
[动作截止时间与清理](https://github.com/microsoft/playwright/blob/500c9c822ce7664539a4c8a88810048dfe090c3b/packages/playwright-core/src/server/progress.ts)。
C 的对应入口是 `actionError`、`targetForElement`、`validateCapture` 与
`waitForEvent`；后者在成功、失败和取消时共享移除监听、清除定时器的清理。
这些只能证明对应路径，不能据此宣称参考实现所有资源路径都已审计。

### Codex 对模型的指导究竟有什么

此前仅从公开仓库或插件入口没有找到说明，不足以得出“Codex 没有这类指导”。
读取当前 U 的工作流文档后，确认它要求操作后依据最新 AX 状态决定下一步，
支持把确定的一组操作和观察放在同次调用，优先差量观察；无变化时不立即重复
同样的读取，目标结果已出现时停止扩展探索。它还说明捕获自带等待，
不要在观察前额外盲等。依据是上面的 U 文档。

这类指导解决的是调用者怎样使用工具，不能代替修复日期 fill、定向按键等
实现错误。所读材料没有支持前面提出的“固定失败次数触发浏览器纠偏机制”
或“把某个业务确认框规则写进浏览器提示”。B22、B23 应先靠明确页面语义、
足够的观察信息和通用操作反馈判断，不能给一个小测试定制模型规则。

### 最小实验

本轮未调用真实模型。实验使用隔离 HTML 页面和临时 HTTP 服务，分别通过
`playwright-core 1.62.1` 操作本机已安装的 Chrome for Testing，以及当前
CUA 的 in-app browser 接口调用 Codex。浏览器内核和适配层不同，结果仅用于
确认以下具体行为，不是性能或通用网站能力比较。

| 页面条件与操作 | 原版 Playwright 实测 | Codex 实测 |
| --- | --- | --- |
| 原生 date 填入 2026-09-28 | fill 成功，value 正确 | AX setValue 成功，显示值正确 |
| date 填入非法月份 2026-13-28 | fill 报 Malformed value | AX setValue 拒绝，保留之前的合法日期 |
| 普通文本阻止 beforeinput，fill 新值 | 返回成功，仍为 old | locator fill 返回成功，成为新值 |
| 当前焦点在别处，定向 checkbox press Space | 聚焦并勾选 | locator press 聚焦并勾选 |
| hello 已聚焦且选择 ll，再输入 XY | type 得到 heXYo，有对应 X/Y 键盘事件 | AX typeText 得到 heXYo；这不证明其事件序列与 P 相同 |
| hello 未聚焦但保留旧选择区，再输入 XY | type 得到 XYhello | 本项未测 |
| 页面阻止 checkbox click 的默认行为，再 check | 报点击没有改变状态 | locator check 同样报错，并附匹配诊断 |
| 同一个阻止 click 的 checkbox，用直接设值 | 本项未测 | AX setValue 成功；与 click/check 是不同语义 |
| 选项延迟加入，再 selectOption | 等待后选中 SG | 本项未测 |
| visibility:hidden 元素等待 visible | 超时 | 本项未测 |
| ControlOrMeta+A 后逐字符 AB | 本项未单独测组合序列 | locator press + pressSequentially 得到 AB |
| 点击永不响应的导航，短截止时间 | click 超时，日志明确点击已完成；noWaitAfter=true 时成功 | locator click 在约 343 ms 返回成功（参数 300 ms）；后续 AX 观察失败 |
| goto 返回 HTTP 500 的页面 | 正常返回 status 500 | 本项未测 |
| goto 拒绝连接的端口 | 报 ERR_CONNECTION_REFUSED | 本项未测 |

P 共执行 13 个探针，结果保存在本机
`.cache/browser-reference/playwright-probes.json`；隔离页面、服务与脚本在
同目录 `fixture.html`、`serve.mjs`、`probe-playwright.mjs`。
C 的结果来自当前工具调用转录，本文保存了条件与结果；没有把未测项记为通过。

Codex 导航停滞实验要分阶段读：click 的成功返回已确认；之后 getAXState
报目标关闭，再尝试 reload/close 遇到控制命令超时，因而未获得完整的后续
页面状态。这不推翻 click 返回的证据，也不能据此概括 Codex 全部导航行为。
临时 HTTP 服务已停止，P 创建的浏览器已在 finally 中关闭；C 的临时 tab
未能通过 close 确认关闭，交由工具的临时 tab 生命周期处理。

H01–H05 属于 Demi 测试控制与实验有效性，浏览器库不能替测试程序解决
客户端接管、模型参数、服务生命周期或目标地址错误。它们继续按问题表处理；
本轮实验没有重新给模型打分。

### 逐个 API 的参考范围

对每个有问题的 API，都要沿对应 Codex 实现检查这些环节，并用 Playwright
的实现与测试补充理解：

```text
调用契约：定位方式、动作含义、错误与显式等待
    ↓
动作协调：重新定位、截止时间、焦点、命中、导航关联、取消
    ↓
页面算法：标签与名称、控件状态、原生输入、选项匹配
    ↓
浏览器协议：输入事件、frame、导航与生命周期事件
```

先确定对应操作，再比较行为。例如 Demi 的 check 对照 Codex locator check，
不能拿 AX setValue 直接修改 checkbox 的结果代替点击路径的参考。
涉及的设计差异按 Codex 优先处理，并在现有原生服务中修复对应实现。

### 与现有设计对照后的修复方向

| 情况 | 现有规则与参考行为的差别 | 判断与建议 |
| --- | --- | --- |
| 可编辑输入被另一元素遮住，但可以聚焦填写 | [可操作条件](demi-next/browser.md#actionability-and-coordinates) 把 fill 定义为点击条件再加 editable；参考实现按动作区分条件 | 按对应 Codex fill、click、hover、press 的条件分别修正设计与实现 |
| 已在 /dashboard，点击后没有导航 | [等待](demi-next/browser.md#waiting) 已规定 --wait-url 不能立即成功；C expectNavigation 的当前条件可能已满足 | 按 Codex 对应等待 API 调整这项差异，同时区分普通动作的内部等待与显式条件等待；B24 的修复按调整后的契约验收 |
| 输入框未聚焦，但保留旧选择区 | [输入](demi-next/browser.md#input-and-forms) 说 type 在光标处输入，未交代聚焦是否改变选择区；P 和 C 的输入路径并不相同 | 对逐字符输入优先参考 C pressSequentially 的聚焦、选择区及中途失焦处理，补齐设计并修复实现 |
| 勾选被页面阻止、定向按键落到别处、文本观察少了 checked | 现有设计已承诺勾选状态、目标语义和观察状态；实现未兑现 | 直接按现有设计修复，不需要重新讨论是否提供这些基本能力 |

另外几项只是有意的接口差别：Demi 导航默认 `domcontentloaded`，P 默认
`load`；短句柄可以与严格的失效检查共存；动作后显式读取观察与不自动附送
整页也可以共存。这些不需要为了表面一致而修改。

本次复核还纠正了本文自身的两点：动作进展在权威设计中是
`not_started | completed | unknown` 三种，之前误写成四种；
`--wait-url` 的新导航要求已经选定，之前把它写成尚待明确不准确。

## 后续核查与修复

- 按对应 Codex API 逐项核对前置条件、输入方式、等待、诊断与清理；
  Playwright 提供补充实现和测试参考。有差异时 Codex 优先。
- 按 Codex 补清普通动作的导航等待、显式 URL 等待及附带 metadata 失败报告，
  同步调整受影响的旧设计。
- 按 Codex 对应操作明确 fill、逐字符输入、定向按键、当前焦点按键的行为；
  原生控件遵循已验证的行为，不自行猜测或修正非法值。
- 设计更新后，按 B01–B20、B24 对照实现和回归用例；B21 保留单独复现，
  B22、B23 与 H01–H05 分别处理，不混入浏览器算法修复。
- 撤回独立格式推断、问题控件预测、浏览器专用模型纠偏规则的提案。
- 不把“搜索没有找到”写成“Codex 没有”；缺乏公开实现证据的行为标为未知。
- 本轮完成调查文档，运行时代码未修改，没有新增真实模型测试。
  后续逐项更新设计、修复实现，并在配对设备和 Cloud 验收。

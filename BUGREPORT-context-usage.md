# Bug 报告:上下文 token 被高估导致误压缩 + 快照文件异常膨胀

> 面向修复者的自包含说明。你不需要看原始对话,读这份就够。
> 涉及仓库:`demi`(本仓库)。发现场景是外部项目 `assetsfactory` 内嵌的 agent(用
> `@demicodes/*`,通过 `provider-claude-code` 驱动 `claude` CLI),但**两个缺陷都在 demi 侧**,与那个项目无关。

## TL;DR

一条长会话的上下文用量指示器显示 **93%(929.6K / 1.0M)**,并在每个新回合前触发压缩,
用户体感"莫名其妙"。排查后发现两个独立缺陷:

1. **【主因·行为 bug】** demi 把 Claude Code `result` 事件里的 **累加 usage** 当成
   "单次请求的上下文大小"来估算,导致上下文被高估 **2–3 倍**,`preflight` 压缩阈值被
   提前打穿。**这条会话真实模型侧内容只有 ~13.5 万 token,却被估成 90 万+。**
2. **【次要·存储浪费】** 每个 shell 工具调用的 `metadata` 里冗余存了 3–4 份 stdout、
   完整 diff(old+new+unified)、以及 `demi read` 图片 base64 的重复副本,使单会话
   snapshot 膨胀到 **47.8MB**(正常会话几百 KB)。不进模型上下文,但拖慢读写。

优先修第 1 个;它直接解决误压缩。

---

## 环境

- Provider:`provider-claude-code`(驱动本机 `claude` CLI,stream-json)。
- 模型:`claude-sonnet-5`,`contextWindow = 1_000_000`。
- 压缩阈值:`DEFAULT_PREFLIGHT_THRESHOLD_RATIO = 0.8`(`packages/agent/src/session.ts:27`),
  即 80 万 token 触发。
- 复现数据来源:一条真实会话的落盘 snapshot(见文末"如何复现")。

---

## Bug 1:累加 usage 被当成单次上下文 → 误压缩(主因)

### 现象
- 上下文用量指示器显示 93%,任何新回合开始前都先压缩。
- 会话其实并没有那么大。

### 根因链
1. **provider 只上报累加 usage。**
   `packages/provider-claude-code/src/output.ts` 只在 `result`(turn 结束)事件上报一次 usage:
   ```ts
   // output.ts:43-50
   if (message.type === 'result') {
     ...
     events.push({ type: 'response', usage: mapUsage(message.usage) })
     return { events, terminal: true }
   }
   ```
   而 Claude Code 的 `result.usage` 是**整个 turn 内部所有子调用的累加值**(一个用工具的
   turn,CLI 内部会"初始调用 + 每次工具结果后再调一次",各自的 input/cache_read 相加)。
   每个 `assistant` 中间消息自带的 **per-step usage 被丢弃**了——`output.ts:30-32` 只取了
   assistant 消息的 `content`,没取它的 `usage`:
   ```ts
   // output.ts:30-32
   if (message.type === 'assistant' && isRecord(message.message) && !options.ignoreAssistantContent) {
     events.push(...mapContentArray((message.message as { content?: unknown }).content, options))
   }
   ```

2. **transcript 拿这个累加值当"单次上下文"锚点。**
   `packages/agent/src/transcript.ts`:
   ```ts
   // usageAnchor(): transcript.ts:478-489  —— 取最后一个 response block 的 usage
   const tokens = usage.inputTokens + usage.outputTokens + usage.cacheReadTokens + usage.cacheWriteTokens
   // estimateContextTokens(): transcript.ts:462-472  —— 以 anchor 为基准估算"下一次请求的上下文"
   ```
   这里的**隐含契约是:provider 上报的 usage = 单次请求的上下文规模**。claude-code 违反了
   这个契约(它报的是整 turn 累加),于是估算被放大。

3. **压缩控制器用被放大的估算和窗口比。**
   `packages/agent/src/compaction-controller.ts`:
   ```ts
   // preflight(): line 62-68
   const threshold = Math.floor(contextWindow * this.host.thresholdRatio) // 1_000_000 * 0.8
   if (this.host.transcript.estimateContextTokens() < threshold) return
   ...compact
   ```
   工具越多的 turn,累加越狠,`estimateContextTokens()` 越虚高,越早触发压缩。

### 硬证据(为什么确定是累加,而非真占用)
从该会话 snapshot 里逐个 `response` block 读**落盘的真实 API usage**,`cacheRead` 列 = 单次
请求实际读入的上下文:

```
block#43   cacheR=  651,181   cacheW= 24,980   out=10,676   TOTAL=  686,881
block#154  cacheR=  645,929   cacheW= 87,982   out= 8,383   TOTAL=  742,312
block#209  cacheR=1,089,277   cacheW= 96,763   out= 4,627   TOTAL=1,190,689   ← 见下
block#329  cacheR=  859,782   cacheW= 64,576   out= 5,200   TOTAL=  929,588   ← UI 的 93% 来自这里
```

**`block#209` 的 `cacheRead = 1,089,277 > contextWindow(1,000,000)`。**
单次请求的 cache-read 不可能超过上下文窗口本身。唯一解释:这是**一个 turn 内多次调用的
cache-read 相加**。这条就足以证明 `result.usage` 是累加值。

对照:把该会话 snapshot 里**模型真正能看到的内容**(所有 341 个 block 的 `output`+`thinking`
+文本+命令+图,CJK 感知按中文≈1 token/字)全加起来,只有 **~13.5 万 token**;最近一次压缩
之后实际重放的更是只有 ~1.1 万 token 文本 + 14 张图。真实上下文远没到窗口。

### 建议修法(二选一或都做)
- **首选:改 provider,上报真实单次 usage。**
  在 `output.ts` 里跟踪**最后一个 `assistant` 消息自带的 usage**(它是最终那次 API 调用的
  真实上下文),用它作为 `response` 事件的 usage,而不是 `result` 的累加值。
  *需先确认 Claude Code 的 stream-json `assistant` 消息里带 `message.usage`(应当带,镜像
  Anthropic SDK 的 assistant.usage)。*
- **兜底(建议一并加,防御其它 provider):** 在 `usageAnchor()` 或 `estimateContextTokens()`
  里对 usage 做 sanity clamp,例如 `min(anchorTokens, contextWindow)`,并/或和 demi 自己
  按 block 估的字符数交叉校验取较小值。`1.09M > 1M` 这种物理上不可能的数本就该被拦下。

### 验收
修完后,长会话(尤其多工具回合)的上下文用量指示器应贴近真实规模,不再在明显没满时触发压缩。
可用文末脚本重新读一条新会话的 usage 轨迹,确认 `cacheRead` 不再超过窗口。

---

## Bug 2:metadata 冗余存储 → snapshot 膨胀到 47.8MB(次要,不占 token)

### 现象
- 单条会话 snapshot = **47,848,065 字节(47.8MB)**;同机其它会话最大才 ~955KB。
- 路径:`~/.local/share/demi/host-local/<hostKey>/agent-sessions/<sessionId>/snapshot.json`

### 根因
每个 `shell_exec` 的 `tool_call` block 里,`metadata` 存了大量重复内容。实测拆解:

- **stdout 存 3–4 份**:同一段命令输出同时出现在
  `metadata.stdout.delta`、`metadata.output.text`、`metadata.output.chunks[0].text`(内容相同),
  外加 `stdout.tail` / `output.tail`。
  例:一次 `cat 布布怕黑.vid`,`output[0].text`=18,241 字符,而 `metadata` 里同内容重复到
  ~63,000 字符。
- **`demi edit` 存三份 diff**:`metadata.commandMetadata[0].metadata.diffs[0]` 同时含
  `unifiedDiff`(~55K)+`newText`(~27K)+`oldText`(~27K),单次编辑 metadata ~111KB。
- **`demi read` 图片 base64 存两份**:一次读图,`output[1].source.data`(~3.5MB)和
  `metadata.assets[0].data`(~3.5MB)是同一张图的重复副本。该会话共 14 张图,`metadata`
  里的重复副本使全量图片字节从 ~16MB 变成 ~32MB。

全会话 metadata 文本冗余合计 ~296 万字符。

### 影响面
- **不进模型上下文**:`collectInferenceItems()` 只取 `block.output`
  (`packages/agent/src/transcript.ts:432-434`),`metadata` 不重放。所以**不占 token**。
- 但每次 `commitTranscript()` 要序列化/落盘整份 snapshot,47MB 会拖慢读写。

### 根因位置(需修复者确认)
`metadata` 内容来自 shell 命令快照的序列化。请从 `@demicodes/shell`
(`ShellCommandSnapshot` / 命令输出 chunk 记录)以及 agent 侧把 shell 结果写入 tool_call
`metadata` 的那段代码入手,目标:
- stdout 不要 `delta` / `output.text` / `chunks[].text` 各存一份,收敛成单一来源;
- `demi read` 的图片 base64 已在 `output` 里,`metadata.assets` 不必再存原始 `data`
  (要留就留引用/路径/哈希);
- `demi edit` 的 diff 三份(old/new/unified)按需保留一份即可,或对大文件截断。

### 验收
新会话跑同样量级操作后,snapshot 体积应从数十 MB 降到与内容相称的量级(百 KB~个位 MB)。

---

## 如何复现 / 重新取证

1. 找到最大的会话 snapshot:
   ```bash
   BASE="$HOME/.local/share/demi/host-local"
   find "$BASE" -type f -name snapshot.json -exec stat -f "%z %N" {} \; | sort -rn | head
   ```
2. 读真实 usage 轨迹(证明 Bug 1),把 `SNAP` 换成上面的路径:
   ```python
   import json
   b = json.load(open(SNAP))["transcript"]["blocks"]
   win = 1_000_000
   for i,x in enumerate(b):
       if x.get("type")=="response":
           u=x["usage"]; tot=u["inputTokens"]+u["outputTokens"]+u["cacheReadTokens"]+u["cacheWriteTokens"]
           flag=" <<< cacheRead>window!" if u["cacheReadTokens"]>win else ""
           print(i, "cacheR",u["cacheReadTokens"], "TOTAL",tot, flag)
   ```
   看到任意 `cacheRead > contextWindow` 即坐实"累加而非单次"。
3. 拆 metadata 冗余(证明 Bug 2):对比每个 `tool_call` 的 `output` 字符数 vs `metadata`
   字符数;`demi read` 看 `output[*].source.data` 与 `metadata.assets[*].data` 是否为同一
   base64;`demi edit` 看 `metadata...diffs[0]` 是否 old/new/unified 三存。

## 判断口径
- Bug 1:**确定的行为 bug**,证据是落盘的真实数字(1.09M > 窗口),会造成功能问题(误压缩)。
- Bug 2:**明确的实现浪费**,不致功能错误,只费磁盘/IO;严格说是效率缺陷。

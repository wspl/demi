# 可读性优化评审：2026-09-08

这份报告记录本轮可读性改动，比较基线是 `0f09a538`。采用的原则是：**任何代码都需要保持极高的可读性水平，一旦写出了低可读性代码则需要进行优化。**

范围限定为前次提交评审发现、当前仍存在的实现。已被替换的 hostless 路径没有重做；已共享的 FileBrowser、HostPicker 没有另建一套。HostPicker 仅改为引用 HostMenu 共用的设备展示类型。AGENTS.md 未改。

## 改动范围

| 分组 | 负责的代码 | 本次组织调整 |
| --- | --- | --- |
| runner / Worker | `runner/src/commands`、`relay`、`machine`、启动和构建入口；`runner-protocol/src/local.ts` | Worker 两端采用同一消息契约；父端保留事件直接完成 Promise 的机制，由 `finish(outcome)` 集中清理并明确返回成功或失败；IO 请求单独处理。relay 把连接、输入等待、取消和完成处理写成可单独追踪的步骤；展开进程、文件、网络及构建流程中压缩的分支。Worker 修复已通过统一验收。 |
| 原生 C 客户端 | `command-client/src/client.c`、`metadata.c`、`build.ts` | 输入请求、输出写入分别组织为结构体；文件和流输入共用结果处理；协议解析与帧处理分开；握手阶段采用有名称的枚举，替代数值标志；元数据拼接和构建步骤展开。 |
| Cloud | `backend/src/managed`、`storage/machine-image-store.ts` | 分开表达普通启动与 reset 内的 runner 启动；明确 transition task、reset task、持久化 reset 结果的用途；巡检保留触发条件，休眠和 checkpoint 各自管理资源；展开镜像发布、VM 启动及错误处理。 |
| UI | `web-ui` 的滚动、拖拽、动画、HostMenu；`web`、`web-gallery` 的使用处 | 滚动存储读写与恢复分开；拖拽期间与结束后的滚动分别表达；产品和 gallery 使用同一个 HostMenu，菜单关闭归共享组件，产品保留权限、cwd 和 store 操作。 |
| installer | `backend/src/http/runner-install.ts`、新增 `backend/src/runner/installer.ts` | HTTP 路由负责请求与响应，安装脚本生成归 runner 模块；展开 shell 中的平台检查、锁、校验、升级和清理步骤。包职责文档同步更新。 |

## 实际前后对比

下面都是基线与工作区的真实摘录；用来比较具体实现，不作为所有场景都必须采用的写法。

### 1. C 输入处理：回调负责结束请求，共用函数负责解释读取结果

文件：[client.c](../../packages/command-client/src/client.c)。原来的文件读取回调同时处理 libuv 请求释放、输入状态及协议响应；流读取回调还重复一份结束判断。

改动前：

```c
static void input_read(uv_fs_t *req) {
    ssize_t n = req->result; uv_fs_req_cleanup(req); input_pending = 0;
    if (n < 0) check((int)n);
    if (!n) { input_ended = 1; send_frame(&data_peer, DEMI_INPUT_END, NULL, 0); }
    else send_frame(&data_peer, DEMI_INPUT, input, (size_t)n);
}
```

改动后：

```c
static void complete_input(ssize_t count) {
    if (count < 0) {
        check((int)count);
    }
    if (count == 0) {
        input_state.phase = INPUT_ENDED;
        send_frame(&data_peer, DEMI_INPUT_END, NULL, 0);
    } else {
        input_state.phase = INPUT_IDLE;
        send_frame(&data_peer, DEMI_INPUT, input_state.bytes, (size_t)count);
    }
}

static void input_read(uv_fs_t *req) {
    ssize_t count = req->result;
    uv_fs_req_cleanup(req);
    complete_input(count);
}
```

文件读取回调和流读取回调现在都交给 `complete_input` 处理结果。读者可以分别核对平台请求如何结束、读取结果如何映射为输入状态和协议帧；不是把原块移到一个笼统的 helper。

### 2. Cloud 休眠：巡检条件与两层占用的释放分开

文件：[lifecycle.ts](../../packages/backend/src/managed/lifecycle.ts)。原来巡检循环中同时写休眠策略、占用检查和两层清理。

改动前（巡检分支）：

```ts
        if (idle || capped) {
          const releaseMachine = machine.activity.tryReserve()
          if (!releaseMachine) continue
          try {
            const releaseTrees = await this.options.reserveIdle(machine.device.userId)
            if (!releaseTrees) continue
            try { await this.hibernate(machine.device.id) } finally { releaseTrees() }
          } finally { releaseMachine() }
        } else if (now - machine.checkpointAt >= this.config.checkpointIntervalMs) {
          this.options.registry.pauseLiveness(machine.device.id)
          try { await this.options.provisioner.checkpoint(machine.device.id); machine.checkpointAt = now }
          finally { this.options.registry.resumeLiveness(machine.device.id) }
        }
```

改动后，巡检命中条件时调用 `hibernateIdleMachine(machine)`；完整的休眠保护流程是：

```ts
  private async hibernateIdleMachine(machine: Machine): Promise<void> {
    const releaseMachine = machine.activity.tryReserve()
    if (!releaseMachine) {
      return
    }

    try {
      const releaseTrees = await this.options.reserveIdle(machine.device.userId)
      if (!releaseTrees) {
        return
      }

      try {
        await this.hibernate(machine.device.id)
      } finally {
        releaseTrees()
      }
    } finally {
      releaseMachine()
    }
  }
```

machine 占用释放和 conversation tree 占用释放各自对应一个 `finally`，获取失败时从当前流程返回。巡检仍负责判断空闲时间和运行上限，helper 负责带占用保护的休眠。没有借重排代码改写现有容量或并发策略。

### 3. UI 拖拽滚动：保留计算，分别写出拖动中和结束后的行为

文件：[AppSidebar.vue](../../packages/web-ui/src/sidebar/AppSidebar.vue)。原来的滚动距离和动画方式藏在多层三元表达式中。

改动前：

```ts
  const delta = source
    ? item.top < bounds.top ? item.top - bounds.top
      : item.bottom > bounds.bottom ? item.bottom - bounds.bottom : 0
    : item.top + item.height / 2 - (bounds.top + bounds.height / 2)
  if (delta) container.scrollBy({
    top: delta,
    behavior: source || window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth',
  })
```

改动后：

```ts
  let distance = 0
  let behavior: ScrollBehavior = 'instant'

  if (source) {
    // Keep the dragged header visible without shifting it unnecessarily.
    if (item.top < bounds.top) {
      distance = item.top - bounds.top
    } else if (item.bottom > bounds.bottom) {
      distance = item.bottom - bounds.bottom
    }
  } else {
    // Once the children unfold, bring their project header back to the center.
    distance = item.top + item.height / 2 - (bounds.top + bounds.height / 2)
    if (!window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
      behavior = 'smooth'
    }
  }

  if (distance) {
    container.scrollBy({ top: distance, behavior })
  }
```

拖动中仅把越界的项目标题移回可见区域；结束后把标题移到中间，并按减少动态效果设置决定是否平滑滚动。计算仍留在同一处，没有为每个算式再引入跳转。

## 伴随的小行为修正

这些变化超出排版和命名，需要连同行为一起评审：

- Cloud：`atomicJson` 在序列化或写入失败后也删除临时文件；checkpoint 在暂停、复制或恢复失败后也清理暂存目录。启动清理失败会记录日志，同时保留原始启动错误。
- Cloud：镜像初始化直接返回刚发布的状态，不再重复读取同一 manifest。
- UI：拖拽放下先解除事件监听并停止动画帧，再调用 reorder handler；handler 抛错时也不会遗留拖拽资源。滚动恢复在 `nextTick` 后检查是否已结束，避免交互取消后重新安装 observer。
- 测试：原生客户端协议测试不再吞掉包含断言失败的异常，测试失败会向外传播；失败时的连接和客户端进程由清理流程回收。

## 验证过程中的回归与修复

首次 Worker 父端改写使用了 async/deferred 与 `finally` 收尾，在实际命令执行中造成回归。最终代码撤回了这次控制流替换，保留原有事件直接完成 Promise 的机制，再用明确的 `WorkerOutcome`、命名清理函数和 IO 请求函数改善表达。修复后 4 项定向测试、23 项断言通过。临时 DEBUG 和 testing override 已清除，vendor 未修改；没有据此推断 GC 或 vendor 存在问题。

## 验证状态

| 检查 | 当前结果 |
| --- | --- |
| runner / native / Cloud / installer 统一验收 | 运行 runner `prepare-tests.ts` 成功后，共 40 通过、0 失败，292 项断言，15 个测试文件。包含 Worker 回归、协议、Firecracker、installer、managed lifecycle 和 reset；此前单独运行的 Worker 4 项与 Cloud 8 项已包含在内，不另行累计。 |
| TypeScript | 最终 `bunx tsgo --noEmit` 通过。 |
| 原生跨平台构建 | `x86_64-windows-gnu`、`aarch64-linux-musl` 编译通过；未在这两个目标系统执行。 |
| UI 现有测试 | 在 `packages/web-ui` 运行 sidebar、overlay、menu-dismiss：13 通过、0 失败，42 项断言。 |
| UI 构建 | `bun run --cwd packages/web build` 通过。 |
| UI 运行验证 | Vue renderer smoke 验证滚动恢复、用户交互停止恢复、保存及卸载清理；Chrome 实际验证 HostMenu 切换、attach、detach 后状态更新且菜单关闭，locked 状态阻止相应操作。临时验证设置已移除。 |
| Vue 类型检查 | 仍有基线已有的两处错误：`CloudSettings.vue:43` 缺 `InlineError.message`；`SessionSection.vue:474` 的 diagnostics 缺 `source`。本次未修改这两处。 |
| 包边界 | `bun test --conditions development ./packages/core/src/__tests__/platform-entrypoints.test.ts`：26 通过、0 失败，126 项断言。 |
| diff 检查 | 最终全仓 `git diff --check` 通过；临时 DEBUG、验证 override 扫描无命中，`runner/src/testing.ts` 无差异。 |

统一验收命令（临时 CMake 路径仅供本机验证）：

```sh
CMAKE=/tmp/demi-txiki-build-tools/cmake/data/bin/cmake \
  bun run --conditions development packages/runner/runtime/prepare-tests.ts
CMAKE=/tmp/demi-txiki-build-tools/cmake/data/bin/cmake \
  bun test --conditions development \
  ./packages/runner/src \
  ./packages/runner-protocol/src \
  ./packages/backend/src/__tests__/firecracker.test.ts \
  ./packages/backend/src/__tests__/runner-install.test.ts \
  ./packages/backend/src/__tests__/scenarios/s10-managed-lifecycle.test.ts \
  ./packages/backend/src/__tests__/scenarios/s11-reset.test.ts
```

这些结果验证具体路径，不代表所有代码已达到同样的可读性，也不代替用户对前后组织方式的判断。

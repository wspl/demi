export type FindingSeverity = 'high' | 'medium' | 'low'

export interface Finding {
  id: string
  title: string
  severity: FindingSeverity
  /** Components or surfaces where the problem shows. */
  where: string
  problem: string
  /** Measured facts behind the call: contrast ratios, token values, file lines. */
  evidence: string[]
  proposal: string[]
  files: string[]
}

export const FINDINGS: readonly Finding[] = [
  {
    id: 'sidebar-scrollbar',
    title: '侧边栏滚动条比其它滚动条亮一个量级',
    severity: 'high',
    where: 'AppSidebar 会话列表',
    problem:
      '侧边栏列表自己覆盖了 scrollbar-color，thumb 用的是文字色 --fg-faint。Ink 暗色下它是 #7a7a7a，压在 #1a1a1a 的 base 面上是一条常亮的灰条；其它所有滚动条用的是 overlay 12%，几乎看不见。亮色模式同样偏重（#8c8c8c 压在 #ececec 上）。',
    evidence: [
      '侧边栏 thumb #7a7a7a vs base #1a1a1a：对比 4.05:1（一条滚动条达到了正文级对比）',
      '全局 thumb overlay 12% vs float #3c3c3c：对比 1.43:1',
      'AppSidebar.vue:384 `.sidebar-scroll { scrollbar-color: var(--fg-faint) transparent }`',
    ],
    proposal: [
      '删掉 AppSidebar 里的 scrollbar-color 覆盖，让侧边栏走全局 thumb（overlay 12%）',
      'scrollbar-gutter: stable 保留，列表宽度不变',
    ],
    files: ['packages/web-ui/src/sidebar/AppSidebar.vue'],
  },
  {
    id: 'scrollbar-reveal',
    title: '滚动条永久可见，不随指针出现',
    severity: 'medium',
    where: '菜单、侧边栏、Markdown / Code 预览、设置面板等所有滚动容器',
    problem:
      '只要设置了 scrollbar-color，Chrome 和 Safari 就不再使用 macOS 的覆盖式滚动条，thumb 会一直画在那里。菜单打开时右侧永远有一条灰杠，Markdown 预览也是。会话记录已经用 .scrollbar-hidden / .scrollbar-active 做了“滚动时才显示”，但只有它一处。',
    evidence: [
      'base.css:448 `* { scrollbar-width: thin; scrollbar-color: overlay 12% transparent }` 对所有元素生效',
      'base.css:453 `.scrollbar-hidden` 只在 AgentMessageList 和 AgentTabBar 使用',
      '截图：Overlays 页三个 tall 菜单在没有指针的情况下都带 thumb',
    ],
    proposal: [
      '全局默认 thumb 透明；指针悬停在滚动容器内（*:hover）或 .scrollbar-active 时显示，250ms 过渡',
      '.scrollbar-hidden 不再需要单独的类，AgentMessageList 只保留 .scrollbar-active',
      '轨道保持透明，宽度仍为 thin',
    ],
    files: ['packages/web-ui/src/styles/base.css', 'packages/web-ui/src/agent/AgentMessageList.vue', 'packages/web-ui/src/agent/AgentTabBar.vue'],
  },
  {
    id: 'accent-fill',
    title: '主按钮 / 发送 / 开关用文字色当填充，白字对比不足',
    severity: 'high',
    where: 'Button primary、IconButton accent（发送）、Dialog 确认键、Switch、Checkbox、Slider 进度',
    problem:
      '--on-accent 是按“压在深色面上的文字”调出来的（暗色 #60a5fa，偏亮），但 .btn-primary、.switch-on、.checkbox-mark 把它当成实心填充，再压白字。暗色模式下白字压浅蓝只有 2.54:1，主按钮反而是整套 UI 里最难读的文字。亮色模式 #2563eb 没有这个问题。',
    evidence: [
      '暗色：白 #ffffff vs #60a5fa 对比 2.54:1（文字要求 4.5，图形 3）',
      '亮色：白 vs #2563eb 对比 5.17:1',
      '候选填充 #2b74dc：白字 4.53:1，对 surface #242424 3.43:1，对 base #1a1a1a 3.84:1',
      'base.css:243 `.btn-primary { background: var(--on-accent) }`，Switch.vue:55、base.css:666 同样',
    ],
    proposal: [
      '新增填充 token --accent-fill：暗色 #2b74dc，亮色 #2563eb（与 --on-accent 相同）',
      '.btn-primary、.switch-on、.checkbox-mark 选中态、Slider 进度改用 --accent-fill',
      '--on-accent 只保留给文字、描边、状态点，不再做大面积填充',
    ],
    files: ['packages/web-ui/src/styles/base.css', 'packages/web-ui/src/styles/product-appearance.css', 'packages/web-ui/src/ui/Switch.vue', 'packages/web-ui/src/ui/Slider.vue'],
  },
  {
    id: 'md-table-border',
    title: 'Markdown 表格边框比整套 hairline 线都粗重',
    severity: 'medium',
    where: '助手回复、MarkdownPreview 里的表格',
    problem:
      '表格 th/td 的边框用的是文字色 --fg-ghost（暗色 #595959），而同一篇 Markdown 里的 hr 是 overlay 10%、blockquote 是 overlay 15%、界面分隔线是 --line（overlay 12%）。表格因此变成一个明显的粗框网格，和 hairline 风格不一致。',
    evidence: [
      '#595959 vs surface #242424 对比 2.22:1；--line（overlay 12%）vs 同一面 1.45:1，亮 1.5 倍',
      'base.css:628 `.markdown-body th, td { border: 1px solid var(--fg-ghost) }`',
    ],
    proposal: [
      '边框改为 color-mix(in srgb, var(--color-overlay) 12%, transparent)，与 hr / blockquote / --line 同一体系',
      '表头底色保持 overlay 4%',
    ],
    files: ['packages/web-ui/src/styles/base.css'],
  },
  {
    id: 'file-link',
    title: '文件链接硬编码浅蓝，亮色模式下读不出来',
    severity: 'medium',
    where: '助手回复里被识别为文件路径的行内代码（.file-link）',
    problem:
      '.file-link 的颜色和背景写死为 rgb(160 200 255)，完全不跟主题走。暗色下尚可，亮色模式下浅蓝字压白底只有 1.72:1，等于看不见。',
    evidence: [
      'rgb(160 200 255) vs #ffffff 对比 1.72:1；vs #242424 对比 9.02:1',
      'base.css:532-550 `.markdown-body a.file-link { color: rgb(160 200 255); background: rgb(160 200 255 / 0.12) }`',
    ],
    proposal: [
      '颜色改 --on-accent，背景改 --tint-accent，hover 时背景换成 --on-accent 22%',
      '与 Checkbox / 状态点 / 侧边栏头像共用同一组 accent token',
    ],
    files: ['packages/web-ui/src/styles/base.css'],
  },
  {
    id: 'menu-shortcut',
    title: '菜单快捷键提示在浮层上几乎看不清',
    severity: 'low',
    where: 'Menu / ContextMenu 里带 shortcut 的项',
    problem:
      '快捷键提示用 --fg-faint（暗色 #7a7a7a），而菜单浮层 --surface-float 是所有暗色面里最亮的（#3c3c3c），两者只差 2.57:1，字号还是 11px。截图里 ⌘D 要凑近才能辨认。侧边栏里同样的 ⌘N 压在 base 上是 4.05:1，所以只有浮层上出问题。',
    evidence: [
      '#7a7a7a vs float #3c3c3c 对比 2.57:1；--fg-subtle #8c8c8c vs float 3.28:1；--fg-muted #b3b3b3 vs float 5.26:1',
      'MenuItem.vue:151 shortcut 用 text-fg-faint',
    ],
    proposal: [
      '快捷键提示改 --fg-subtle；禁用项、子菜单箭头保持 --fg-faint',
    ],
    files: ['packages/web-ui/src/ui/MenuItem.vue'],
  },
]

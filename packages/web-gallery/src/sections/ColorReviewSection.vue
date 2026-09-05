<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { Copy, Pencil, Send, Trash2 } from '@lucide/vue'
import AppSidebar from '@demicodes/web-ui/sidebar/AppSidebar.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Checkbox from '@demicodes/web-ui/ui/Checkbox.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import { md } from '@demicodes/web-ui/markdown/md'
import ColorReviewCase from '../components/ColorReviewCase.vue'
import GallerySection from '../components/GallerySection.vue'
import { FINDINGS } from '../color-review/findings'
import {
  loadDecisions,
  saveDecisions,
  type ReviewChoice,
  type ReviewDecisions,
} from '../color-review/decisions'
import { galleryState } from '../gallery-state'
import { demoAccount, demoConversations, demoPlugins, demoProjects, demoSkills } from '../sidebar/sidebar-data'

const decisions = ref<ReviewDecisions>({})
const saveState = ref<'idle' | 'saving' | 'error'>('idle')

onMounted(async () => {
  try {
    decisions.value = await loadDecisions()
  } catch {
    saveState.value = 'error'
  }
})

async function decide(id: string, choice: ReviewChoice, note: string): Promise<void> {
  decisions.value = { ...decisions.value, [id]: { choice, note, updatedAt: new Date().toISOString() } }
  saveState.value = 'saving'
  try {
    await saveDecisions(decisions.value)
    saveState.value = 'idle'
  } catch {
    saveState.value = 'error'
  }
}

const decided = computed(() => FINDINGS.filter((finding) => decisions.value[finding.id]).length)

// Specimens. Each renders twice: once as it ships, once under the proposed fix.
const projects = demoProjects()
const conversations = demoConversations()
const plugins = demoPlugins()
const skills = demoSkills()
const tallActions = Array.from({ length: 24 }, (_, i) => `Action ${String(i + 1).padStart(2, '0')}`)
const switchOn = ref(true)
const checkboxOn = ref(true)

const tableMarkdown = `| Check | File | Result |
| --- | --- | --- |
| Helper | cookie.ts | writes \`session=\` |
| Test | auth.test.ts | still expects \`sid\` |
| Snapshot | auth.test.ts | stale header string |

---

> Keep the fix in one file.`

const fileLinkMarkdown = 'Open `packages/web/src/auth.test.ts` and update the assertion; `cookie.ts` stays as it is.'
const knownPaths = new Set(['packages/web/src/auth.test.ts', 'cookie.ts'])

const tableHtml = computed(() => {
  void galleryState.mode
  return md.render(tableMarkdown)
})
const fileLinkHtml = computed(() => {
  void galleryState.mode
  return md.render(fileLinkMarkdown, { knownPaths })
})

const notProposed: [string, string][] = [
  ['文字层级', 'Ink 暗色下 --fg-emphasis / --fg / --fg-body 三个值相同（#e6e6e6），标题和正文只靠字重区分。可读性没有问题，属于层级设计选择，不列入本轮。'],
  ['--line-strong', 'Ink 暗色下与 --line 同为 overlay 12%，但目前没有组件用到 --line-strong，不产生可见差异。'],
  ['排队消息、禁用项、Exploratory marks', '这些是刻意压暗的状态，对比低是设计意图，不算问题。'],
]
</script>

<template>
  <div class="flex flex-col gap-8">
    <GallerySection
      title="Color review"
      note="Ink · regular · medium · hairline 下逐页截图（暗色为主，亮色对照）后确认的配色问题。每一项左边是现在的组件，右边是同一个组件套上拟议改动后的样子；组件都是真实渲染，不是示意图。"
    >
      <div class="gallery-frame px-4 py-3 text-[13px] leading-5 text-fg-muted">
        <div class="flex flex-wrap items-center gap-x-6 gap-y-1">
          <span>已决定 <span class="text-fg">{{ decided }}</span> / {{ FINDINGS.length }}</span>
          <span>决定会写入 <code class="rounded bg-overlay/8 px-1 font-mono text-[12px]">packages/web-gallery/.color-review/decisions.json</code></span>
          <span v-if="saveState === 'saving'" class="text-fg-subtle">保存中…</span>
          <span v-else-if="saveState === 'error'" class="text-on-danger">保存失败：需要通过 gallery 的 dev server 打开这个页面</span>
          <span class="ml-auto">当前：{{ galleryState.mode }} · {{ galleryState.tone }}</span>
        </div>
      </div>
    </GallerySection>

    <ColorReviewCase
      v-for="(finding, index) in FINDINGS"
      :key="finding.id"
      :finding="finding"
      :index="index"
      :decision="decisions[finding.id] ?? null"
      @decide="(choice, note) => decide(finding.id, choice, note)"
    >
      <template v-if="finding.id === 'sidebar-scrollbar'">
        <div class="gallery-frame flex h-[20rem] w-full max-w-[22rem] overflow-hidden">
          <AppSidebar
            :account="demoAccount"
            :projects="projects"
            :conversations="conversations"
            active-id="c-login"
            :plugins="plugins"
            :skills="skills"
          />
        </div>
      </template>

      <template v-else-if="finding.id === 'scrollbar-reveal'">
        <div class="flex flex-wrap items-start gap-4">
          <Menu iconless>
            <MenuItem v-for="label in tallActions" :key="label" :label="label" />
          </Menu>
          <div class="gallery-frame flex h-[16rem] w-[18rem] overflow-hidden">
            <AppSidebar
              :account="demoAccount"
              :projects="projects"
              :conversations="conversations"
              active-id="c-login"
              :plugins="plugins"
              :skills="skills"
            />
          </div>
        </div>
      </template>

      <template v-else-if="finding.id === 'accent-fill'">
        <div class="flex flex-wrap items-center gap-4">
          <Button variant="primary">Keep</Button>
          <Button variant="primary" size="sm">Small</Button>
          <Button variant="primary" size="xs">Extra small</Button>
          <IconButton :icon="Send" variant="accent" />
          <IconButton :icon="Send" variant="accent" circle />
          <Switch v-model="switchOn" label="Available" />
          <Checkbox v-model="checkboxOn" label="Show unreads" />
        </div>
      </template>

      <template v-else-if="finding.id === 'md-table-border'">
        <div class="markdown-body w-full rounded-lg bg-surface-editor p-4 text-conversation text-fg-body" v-html="tableHtml" />
      </template>

      <template v-else-if="finding.id === 'file-link'">
        <div class="markdown-body w-full rounded-lg bg-surface-editor p-4 text-conversation text-fg-body" v-html="fileLinkHtml" />
      </template>

      <template v-else-if="finding.id === 'menu-shortcut'">
        <Menu>
          <MenuItem :icon="Pencil" label="Rename" shortcut="↵" />
          <MenuItem :icon="Copy" label="Duplicate" shortcut="⌘D" />
          <MenuItem :icon="Trash2" label="Archive" shortcut="⌘⇧A" />
          <MenuDivider />
          <MenuItem label="Delete" :icon="Trash2" is-danger shortcut="⌫" />
          <MenuItem label="Running" disabled />
        </Menu>
      </template>
    </ColorReviewCase>

    <GallerySection title="看到但不打算改的" note="检视中记下来、但判断不是真实问题的点，避免误报。">
      <div class="gallery-frame divide-y divide-line">
        <div v-for="[name, reason] in notProposed" :key="name" class="grid gap-2 px-4 py-3 md:grid-cols-[220px_1fr]">
          <div class="text-[13px] text-fg">{{ name }}</div>
          <div class="text-[13px] leading-5 text-fg-muted">{{ reason }}</div>
        </div>
      </div>
    </GallerySection>
  </div>
</template>

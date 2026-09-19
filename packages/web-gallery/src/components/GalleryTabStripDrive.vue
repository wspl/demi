<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import WorkPanel from '@demicodes/web-ui/agent/WorkPanel.vue'
import { addBrowserTab, closeBrowserTabs, workPanelTabs, type WorkTab } from '@demicodes/web-ui/agent/work-panel'

/**
 * The work panel's tab strip in a frame of the given width, with every motion
 * the strip has to get right on buttons under it: select (first, middle, last,
 * a cut-off one), open (one, five), close (first, active, middle, last, all
 * but the active), and a scripted run through them. The panel is the
 * product's; this holds its tabs the way the product does and drives them.
 */
const props = withDefaults(defineProps<{ width?: string }>(), { width: '100%' })

const TITLES = [
  '127.0.0.1:5173', 'Pull request #42', 'Build log', 'API reference', 'Staging',
  'Design review', 'Error dashboard', 'Release notes', 'Issue tracker',
]

const tabs = ref<WorkTab[]>([])
const activeId = ref<string | null>(null)
let opened = 0

function openPage(): void {
  const title = TITLES[opened % TITLES.length]!
  opened += 1
  // The body never shows here, so the page is never loaded.
  const next = addBrowserTab(tabs.value, { url: 'about:blank', title, expose: false })
  tabs.value = next.tabs
  activeId.value = next.activeId
}

function closeTabs(ids: string[]): void {
  const next = closeBrowserTabs(tabs.value, activeId.value, ids)
  tabs.value = next.tabs
  activeId.value = next.activeId
}

/** Change and File, then six pages, the first selected. */
function reset(): void {
  tabs.value = workPanelTabs()
  opened = 0
  for (let i = 0; i < 6; i++)
    openPage()
  activeId.value = ids()[0] ?? null
}
reset()

const frame = ref<HTMLElement | null>(null)
const playing = ref(false)
const step = ref('')
let stopped = false

/** The browser tabs in strip order: the only tabs the strip moves. */
function ids(): string[] {
  return tabs.value.filter((tab) => tab.kind === 'browser').map((tab) => tab.id)
}

function selectAt(index: number): void {
  const id = ids().at(index)
  if (id)
    activeId.value = id
}

function selectMiddle(): void {
  selectAt(Math.floor(ids().length / 2))
}

/** The first tab the frame cuts off, else the last one. */
function selectCutOff(): void {
  const strip = frame.value?.querySelector('[role="tablist"]')
  if (!strip)
    return
  const view = strip.getBoundingClientRect()
  const shown = [...strip.querySelectorAll<HTMLElement>('[role="tab"]')]
  const cut = shown.find((tab) => {
    const rect = tab.getBoundingClientRect()
    return rect.left < view.left || rect.right > view.right
  })
  selectAt(cut ? shown.indexOf(cut) : shown.length - 1)
}

function open(count: number): void {
  for (let i = 0; i < count; i++)
    openPage()
}

function closeAt(index: number): void {
  const id = ids().at(index)
  if (id)
    closeTabs([id])
}

function closeActive(): void {
  if (activeId.value)
    closeTabs([activeId.value])
}

function closeOthers(): void {
  closeTabs(ids().filter((id) => id !== activeId.value))
}

const STEPS: [string, () => void][] = [
  ['open one', () => open(1)],
  ['select first', () => selectAt(0)],
  ['select cut-off', selectCutOff],
  ['close active', closeActive],
  ['open five', () => open(5)],
  ['select middle', selectMiddle],
  ['close middle', () => closeAt(Math.floor(ids().length / 2))],
  ['close last', () => closeAt(-1)],
  ['select last', () => selectAt(-1)],
  ['close first', () => closeAt(0)],
  ['close others', closeOthers],
  ['reset', reset],
]

function wait(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

async function play(): Promise<void> {
  if (playing.value)
    return
  playing.value = true
  for (const [name, run] of STEPS) {
    if (stopped)
      break
    step.value = name
    run()
    await wait(900)
  }
  step.value = ''
  playing.value = false
}

onBeforeUnmount(() => {
  stopped = true
})

const count = computed(() => ids().length)
</script>

<template>
  <div class="flex flex-col gap-3">
    <div
      ref="frame"
      class="gallery-frame gallery-frame-base h-11 max-w-full overflow-hidden"
      :style="{ width: props.width }"
    >
      <WorkPanel
        :tabs="tabs"
        :active-id="activeId"
        @select="activeId = $event"
        @add-browser="openPage"
        @close-tabs="closeTabs"
        @close="reset"
      >
        <!-- Only the header takes part; the body stays empty. -->
        <template #default><span /></template>
      </WorkPanel>
    </div>
    <div class="flex flex-wrap items-center gap-x-4 gap-y-2 text-[12px] text-fg-muted">
      <span class="flex flex-wrap items-center gap-1">
        <span class="mr-1 select-none">Select</span>
        <Button size="sm" :disabled="playing" @click="selectAt(0)">First</Button>
        <Button size="sm" :disabled="playing" @click="selectMiddle">Middle</Button>
        <Button size="sm" :disabled="playing" @click="selectAt(-1)">Last</Button>
        <Button size="sm" :disabled="playing" @click="selectCutOff">Cut-off</Button>
      </span>
      <span class="flex flex-wrap items-center gap-1">
        <span class="mr-1 select-none">Open</span>
        <Button size="sm" :disabled="playing" @click="open(1)">One</Button>
        <Button size="sm" :disabled="playing" @click="open(5)">Five</Button>
      </span>
      <span class="flex flex-wrap items-center gap-1">
        <span class="mr-1 select-none">Close</span>
        <Button size="sm" :disabled="playing" @click="closeAt(0)">First</Button>
        <Button size="sm" :disabled="playing" @click="closeActive">Active</Button>
        <Button size="sm" :disabled="playing" @click="closeAt(Math.floor(count / 2))">Middle</Button>
        <Button size="sm" :disabled="playing" @click="closeAt(-1)">Last</Button>
        <Button size="sm" :disabled="playing" @click="closeOthers">Others</Button>
      </span>
      <span class="flex flex-wrap items-center gap-1">
        <Button size="sm" :disabled="playing" @click="play">Play all</Button>
        <Button size="sm" :disabled="playing" @click="reset">Reset</Button>
        <span v-if="step" class="ml-1 font-mono text-[11px] text-fg-faint">{{ step }}</span>
      </span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import WorkPanel from '@demicodes/web-ui/agent/WorkPanel.vue'
import { browserPage } from '@demicodes/plugin-browser'
import { browserTabKind } from '@demicodes/plugin-browser/live/kind'
import { changesPage } from '@demicodes/plugin-changes'
import { fileBrowserPage } from '@demicodes/plugin-file-browser'
import { galleryBrowser } from '../fixtures/live-browser'
import { galleryFiles, useGalleryWork } from '../fixtures/work-panel'
import { createGalleryWorkspace } from '../fixtures/workspace'

/**
 * The work panel's tab strip in a frame of the given width, with every motion
 * the strip has to get right on buttons under it: select (first, middle, last,
 * a cut-off one), open (one, five), close (first, active, middle, last, all
 * but the active), and a scripted run through them. The panel is the
 * product's; this holds its tabs the way the product does and drives them.
 */
const props = withDefaults(
  defineProps<{
    width?: string
    /** How many pages Reset opens. */
    pages?: number
    /** The page Reset selects. */
    select?: 'first' | 'last'
  }>(),
  { width: '100%', pages: 6, select: 'first' },
)

const TITLES = [
  '127.0.0.1:5173', 'Pull request #42: Keep the session cookie on sign-out', 'Build log', 'API reference', 'Staging',
  'Design review', 'Error dashboard', 'Release notes', 'Issue tracker',
]

const workspace = createGalleryWorkspace()
// Change and File pinned before the pages, as the product's panel has them; the
// pages are the conversation browser's, which has none open before Reset opens them.
const work = useGalleryWork(null, {
  files: galleryFiles(workspace),
  browser: galleryBrowser([]),
  pages: [changesPage, fileBrowserPage, browserPage()],
})
const { panel, pinned, kinds } = work
let opened = 0

function openPage(): void {
  const title = TITLES[opened % TITLES.length]!
  opened += 1
  // The body never shows here, so no picture is shown; the title names the tab.
  work.add(browserTabKind.kind, { url: 'https://example.test/', title })
}

function closeTabs(ids: string[]): void {
  work.closeTabs(ids)
}

/** The pages beside Change and File, the first or the last selected. */
function reset(): void {
  work.reset()
  opened = 0
  for (let i = 0; i < props.pages; i++)
    openPage()
  selectAt(props.select === 'first' ? 0 : -1)
}
reset()

const frame = ref<HTMLElement | null>(null)
const playing = ref(false)
const step = ref('')
let stopped = false

/** The tabs in strip order. */
function ids(): string[] {
  return panel.value.tabs.map((tab) => tab.id)
}

function selectAt(index: number): void {
  const id = ids().at(index)
  if (id)
    work.select(id)
}

function selectMiddle(): void {
  selectAt(Math.floor(ids().length / 2))
}

/** The first page the strip cuts off, else the last one. */
function selectCutOff(): void {
  // The pages follow Change and File in the strip's tab list, in the scrolled part of it.
  const shown = [...frame.value?.querySelectorAll<HTMLElement>('[role="tablist"] [role="tab"]') ?? []].slice(-ids().length)
  const strip = shown[0]?.parentElement
  if (!strip)
    return
  const view = strip.getBoundingClientRect()
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
  const selection = work.selected.value
  if (selection !== null) {
    closeTabs([selection])
  }
}

function closeOthers(): void {
  closeTabs(ids().filter((id) => id !== work.selected.value))
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
        :panel="panel"
        :pinned="pinned"
        :kinds="kinds"
        @select="work.select($event)"
        @add-tab="(kind, data, index) => work.add(kind, data, { select: true, index })"
        @move-tab="work.move"
        @update-tab="work.update"
        @update-pinned="work.updatePinned"
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
        <Button size="sm" :disabled="playing" @click="play">Play All</Button>
        <Button size="sm" :disabled="playing" @click="reset">Reset</Button>
        <span v-if="step" class="ml-1 font-mono text-[11px] text-fg-faint">{{ step }}</span>
      </span>
    </div>
  </div>
</template>

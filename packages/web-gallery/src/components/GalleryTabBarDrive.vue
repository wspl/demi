<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import GalleryTabBar from './GalleryTabBar.vue'

/**
 * A tab bar in a frame of the given width, with every motion the strip has
 * to get right on buttons under it: select (first, middle, last, a cut-off
 * one), open (one, five), close (first, active, middle, last, all but the
 * active), and a scripted run through them. The bar is the same
 * `GalleryTabBar`; this only drives it.
 */
const props = withDefaults(defineProps<{ width?: string }>(), { width: '100%' })

interface Bar {
  selectTab: (id: string) => void
  addTab: () => void
  closeTab: (id: string) => void
  tabIds: () => string[]
  activeId: () => string
  reset: () => void
}
const bar = ref<Bar | null>(null)
const frame = ref<HTMLElement | null>(null)
const playing = ref(false)
const step = ref('')
let stopped = false

function ids(): string[] {
  return bar.value?.tabIds() ?? []
}

function selectAt(index: number): void {
  const id = ids().at(index)
  if (id) {
    bar.value?.selectTab(id)
  }
}

function selectMiddle(): void {
  selectAt(Math.floor(ids().length / 2))
}

/** The first tab the frame cuts off, else the last one. */
function selectCutOff(): void {
  const strip = frame.value?.querySelector('[role="tablist"]')
  if (!strip) {
    return
  }
  const view = strip.getBoundingClientRect()
  const tabs = [...strip.querySelectorAll<HTMLElement>('[role="tab"]')]
  const cut = tabs.find((tab) => {
    const rect = tab.getBoundingClientRect()
    return rect.left < view.left || rect.right > view.right
  })
  const index = cut ? tabs.indexOf(cut) : tabs.length - 1
  selectAt(index)
}

function open(count: number): void {
  for (let i = 0; i < count; i++) {
    bar.value?.addTab()
  }
}

function closeAt(index: number): void {
  const id = ids().at(index)
  if (id) {
    bar.value?.closeTab(id)
  }
}

function closeActive(): void {
  const id = bar.value?.activeId()
  if (id) {
    bar.value?.closeTab(id)
  }
}

function closeOthers(): void {
  const active = bar.value?.activeId()
  for (const id of ids()) {
    if (id !== active) {
      bar.value?.closeTab(id)
    }
  }
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
  ['reset', () => bar.value?.reset()],
]

function wait(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

async function play(): Promise<void> {
  if (playing.value) {
    return
  }
  playing.value = true
  for (const [name, run] of STEPS) {
    if (stopped) {
      break
    }
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
      class="gallery-frame max-w-full overflow-hidden bg-surface-base"
      :style="{ width: props.width }"
    >
      <GalleryTabBar ref="bar" />
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
        <Button size="sm" :disabled="playing" @click="bar?.reset()">Reset</Button>
        <span v-if="step" class="ml-1 font-mono text-[11px] text-fg-faint">{{ step }}</span>
      </span>
    </div>
  </div>
</template>

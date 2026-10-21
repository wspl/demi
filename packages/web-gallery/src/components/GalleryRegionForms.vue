<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Segmented, { type SegmentedOption } from '@demicodes/web-ui/ui/Segmented.vue'
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import FileIcon from '@demicodes/web-ui/files/FileIcon.vue'
import { reportSyntaxColorsUnavailable } from '@demicodes/web-ui/editor/load'
import { provideMediaViewer } from '@demicodes/web-ui/files/media-viewer'
import { pinRegionForms, type RegionForm } from '@demicodes/web-ui/infra/region-wait'
import { renderAgain } from '@demicodes/web-ui/markdown/render-version'
import GallerySpecimen from './GallerySpecimen.vue'

/**
 * The forms a region draws while what it needs loads (code, a formula, a file
 * icon): nothing yet at its size, for 300 ms at most; then its plain form,
 * which stays; then the final one. The control pins every region of the page to
 * one of them; Loaded leaves each in its own, as the product draws it. A
 * message that streams shows that a formula waiting never hides the words that
 * already show.
 */
type Loading = 'first' | 'late' | 'loaded' | 'failed'
const options: readonly SegmentedOption<Loading>[] = [
  { value: 'first', label: 'First Load' },
  { value: 'late', label: 'Past 300 ms' },
  { value: 'loaded', label: 'Loaded' },
  { value: 'failed', label: 'Failed' },
]
const FORMS: Record<Loading, RegionForm | null> = { first: 'held', late: 'plain', loaded: null, failed: 'plain' }
const loading = ref<Loading>('first')

provideMediaViewer()

const REPLY = [
  'Added the helper, and the total follows from it:',
  '',
  '```ts',
  'export function total(prices: number[]): number {',
  '  return prices.reduce((sum, price) => sum + price, 0)',
  '}',
  '```',
  '',
  '$$',
  '\\text{total} = \\sum_{i=1}^{n} p_i',
  '$$',
  '',
  'Each price $p$ counts once.',
].join('\n')
const FILES = ['index.ts', 'package.json', 'App.vue', 'README.md', 'notes.txt']

const STREAMED = 'The total is $\\sum_{i=1}^{n} p_i$ for the prices on the order, and the words after the formula arrive as they are written.'
const streamed = ref('')
const streaming = ref(false)
let stream: ReturnType<typeof setInterval> | null = null
function replay(): void {
  if (stream)
    clearInterval(stream)
  streamed.value = ''
  streaming.value = true
  stream = setInterval(() => {
    streamed.value = STREAMED.slice(0, streamed.value.length + 4)
    if (streamed.value.length >= STREAMED.length) {
      streaming.value = false
      if (stream)
        clearInterval(stream)
      stream = null
    }
  }, 90)
}
replay()

watch(loading, (now) => {
  pinRegionForms(FORMS[now])
  // What draws without reading a form inside a computed draws again.
  renderAgain()
  if (now === 'failed')
    reportSyntaxColorsUnavailable(new Error('Failed to fetch dynamically imported module'))
}, { immediate: true })
onBeforeUnmount(() => {
  pinRegionForms(null)
  renderAgain()
  if (stream)
    clearInterval(stream)
})
</script>

<template>
  <div class="mt-8 flex flex-col gap-5" data-region-forms>
    <h3 class="text-[15px] font-medium text-fg-emphasis">
      Regions While Their Renderers Load
    </h3>
    <div class="flex flex-wrap items-center gap-3">
      <Segmented v-model="loading" :options="options" />
      <Button @click="replay">Replay Streaming</Button>
    </div>
    <div class="flex flex-wrap gap-5">
      <GallerySpecimen variant="Reply · code, a display formula and an inline formula">
        <div class="w-[34rem] max-w-full">
          <StreamedMarkdown :content="REPLY" />
        </div>
      </GallerySpecimen>
      <GallerySpecimen variant="Streaming · a formula waiting hides itself, never the words shown">
        <div class="w-[34rem] max-w-full">
          <StreamedMarkdown :content="streamed" :streaming="streaming" />
        </div>
      </GallerySpecimen>
      <GallerySpecimen variant="File icons · the box keeps its size">
        <div class="flex flex-col gap-1.5 text-chrome text-fg-body">
          <div v-for="name in FILES" :key="name" class="flex items-center gap-2">
            <FileIcon :name="name" :is-directory="false" />
            {{ name }}
          </div>
          <div class="flex items-center gap-2">
            <FileIcon name="src" :is-directory="true" />
            src
          </div>
        </div>
      </GallerySpecimen>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import type { HeldBlock } from '@demicodes/web-ui/transport/protocol'
import AgentMessageList from '@demicodes/web-ui/agent/AgentMessageList.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { provideBlockReader } from '@demicodes/web-ui/agent/whole-blocks'
import { provideHistoryReading } from '@demicodes/web-ui/agent/history-reading'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'
import { commandLookBlocks, requestPairsBlocks, thoughtAnswerBlocks } from '../fixtures/blocks'

/**
 * How a long conversation reads: a window that does not reach the start shows a
 * loading row at its top while the page before it is read, and earlier requests
 * arrive above it without moving the rows in view; a row whose block came in its light form reads it whole
 * when it is opened. The stand-in backend answers each read after a moment.
 */
const READ_MS = 700
const timers = new Set<ReturnType<typeof setTimeout>>()
function later(work: () => void): void {
  const timer = setTimeout(() => {
    timers.delete(timer)
    work()
  }, READ_MS)
  timers.add(timer)
}
onBeforeUnmount(() => {
  for (const timer of timers) {
    clearTimeout(timer)
  }
})

// The latest page holds the last requests, enough of them to fill many
// screens, so the list stands at its bottom; the page before it holds the
// earlier requests.
const whole = commandLookBlocks()
const latestStart = whole.findLastIndex((block) => block.type === 'user')
const latestPage = [...requestPairsBlocks(40), ...whole.slice(latestStart)]
const earlierPage = whole.slice(0, latestStart)
const earlierRead = ref(false)
const reading = ref(false)
const windowBlocks = computed(() => (earlierRead.value ? [...earlierPage, ...latestPage] : latestPage))
// The loading row stands only while the page before is being read.
provideHistoryReading(() => ({ before: reading.value, after: false }))
function readBefore(): void {
  if (earlierRead.value || reading.value) {
    return
  }
  reading.value = true
  later(() => {
    earlierRead.value = true
    reading.value = false
  })
}
/** The list is drawn anew each time the window is shown again, so it stands at the bottom, as a conversation opens. */
const windowKey = ref(0)
function resetWindow(): void {
  earlierRead.value = false
  windowKey.value += 1
}
// The block the reader stopped at, which bounds what of the transcript the
// product holds; null where the window holds the whole transcript and the
// reader is at its latest block.
const shownAt = ref<string | null>(null)
const shownNote = computed(() => {
  if (shownAt.value === null) {
    return 'The window holds the whole transcript and the reader is at its latest block.'
  }
  const place = windowBlocks.value.findIndex((block) => block.id === shownAt.value) + 1
  return `The reader is at block ${place} of the ${windowBlocks.value.length} the window holds; the product holds the blocks around it.`
})

// The same requests read from a page, and a question answered after a
// thinking: their calls and thinking in their light form, without their
// script, output and text, until a row is opened.
function lightened(block: HeldBlock): HeldBlock {
  if (block.type === 'tool_call') {
    // A short input stays whole, as the backend keeps it.
    const description = (JSON.parse(block.input) as { description?: string }).description
    return {
      ...block,
      input: block.input.length > 512 ? JSON.stringify(description === undefined ? {} : { description }) : block.input,
      output: block.output.filter((part) => part.type !== 'text'),
      view: block.view?.kind === 'shell' ? { ...block.view, chunks: [] } : block.view,
      light: true,
    }
  }
  if (block.type === 'thinking') {
    return { ...block, text: '', signature: null, textLength: [...block.text.trim()].length, light: true }
  }
  return block
}
const wholeRead = [...whole, ...thoughtAnswerBlocks()]
const lightBlocks = ref<HeldBlock[]>(wholeRead.map(lightened))
provideBlockReader(() => async (_node, id) => {
  await new Promise<void>((resolve) => later(resolve))
  lightBlocks.value = lightBlocks.value.map((block) => (block.id === id ? wholeRead.find((item) => item.id === id) ?? block : block))
})
function resetLight(): void {
  lightBlocks.value = wholeRead.map(lightened)
}
</script>

<template>
  <GallerySection
    title="A Long Conversation"
    note="The transcript holds the parts the reader has been to. The list stands at the bottom of the latest page. Scrolling up to within three screens of the top of a window that does not reach the start reads the page before it; a loading row stands at the top only while it is read, and its requests appear above without moving the rows in view. A step that came in its light form opens to read its script and output, and a thinking its text, its row shimmering until they arrive."
  >
    <GallerySpecimen variant="earlier requests read as the reader nears the top" wide>
      <div class="flex flex-col gap-2">
        <div class="gallery-frame h-[22rem] bg-surface">
          <AgentMessageList
            :key="windowKey"
            class="h-full"
            conversation-id="gallery-history-window"
            :blocks="windowBlocks"
            :at-start="earlierRead"
            :pending-steers="[]"
            :queue="[]"
            phase="idle"
            :bottom-offset="0"
            :persisted-scroll-state="undefined"
            read-only
            @read-before="readBefore"
            @shown="(block) => (shownAt = block)"
          />
        </div>
        <p class="text-xs text-fg-faint">{{ shownNote }}</p>
        <div>
          <Button size="sm" variant="default" :disabled="!earlierRead" @click="resetWindow">Show Only the Latest Page</Button>
        </div>
      </div>
    </GallerySpecimen>
    <GallerySpecimen variant="a step read whole when it is opened" wide>
      <div class="flex flex-col gap-2">
        <div class="gallery-frame h-[22rem] bg-surface">
          <AgentMessageList
            class="h-full"
            conversation-id="gallery-history-light"
            :blocks="lightBlocks"
            :pending-steers="[]"
            :queue="[]"
            phase="idle"
            :bottom-offset="0"
            :persisted-scroll-state="undefined"
            read-only
          />
        </div>
        <div>
          <Button size="sm" variant="default" @click="resetLight">Fold Back to the Light Form</Button>
        </div>
      </div>
    </GallerySpecimen>
  </GallerySection>
</template>

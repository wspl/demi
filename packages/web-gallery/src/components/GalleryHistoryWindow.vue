<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import type { HeldBlock } from '@demicodes/web-ui/transport/protocol'
import AgentMessageList from '@demicodes/web-ui/agent/AgentMessageList.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { provideBlockReader } from '@demicodes/web-ui/agent/whole-blocks'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'
import { commandLookBlocks } from '../fixtures/blocks'

/**
 * How a long conversation reads (`web-application.md` § Transcript
 * windows): a window that does not reach the start shows a loading row at
 * its top, and earlier requests arrive above it without moving the rows in
 * view; a row whose block came in its light form reads it whole when it is
 * opened. The stand-in backend answers each read after a moment.
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

// The latest page holds the last request; the page before it, the rest.
const whole = commandLookBlocks()
const latestStart = whole.findLastIndex((block) => block.type === 'user')
const earlierRead = ref(false)
const reading = ref(false)
const windowBlocks = computed(() => (earlierRead.value ? whole : whole.slice(latestStart)))
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
function resetWindow(): void {
  earlierRead.value = false
}

// The same request read from a page: its calls and thinking in their light
// form, without their script, output and text, until a row is opened.
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
    return { ...block, text: '', signature: null, light: true }
  }
  return block
}
const lightBlocks = ref<HeldBlock[]>(whole.map(lightened))
provideBlockReader(() => async (_node, id) => {
  await new Promise<void>((resolve) => later(resolve))
  lightBlocks.value = lightBlocks.value.map((block) => (block.id === id ? whole.find((item) => item.id === id) ?? block : block))
})
function resetLight(): void {
  lightBlocks.value = whole.map(lightened)
}
</script>

<template>
  <GallerySection
    title="A Long Conversation"
    note="The transcript holds the parts the reader has been to. Near the top of a window that does not reach the start, a loading row stands while the page before it is read, and its requests appear above without moving the rows in view. A step that came in its light form opens to read its script and output, its row shimmering until they arrive."
  >
    <GallerySpecimen variant="earlier requests read as the reader nears the top" wide>
      <div class="flex flex-col gap-2">
        <div class="gallery-frame h-[22rem] bg-surface">
          <AgentMessageList
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
          />
        </div>
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

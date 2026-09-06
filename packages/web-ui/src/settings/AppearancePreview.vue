<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import { segmentStreamUnits } from '@demicodes/web-ui/ui/stream-reveal'

/**
 * What the appearance rows change, at real size: a reply rendered by the transcript's
 * own streaming markdown at the chosen text size, the tone's stack of surfaces, and a
 * switch that carries the accent and can be flipped. Everything reads the live tokens.
 */
defineProps<{
  /** Transcript text size in px, applied as the transcript would. */
  fontSize?: number
}>()

const PASSAGE = 'The change is in. Surfaces follow the **tone**, the switch below takes the *accent*, and this text sits at the size you chose. It streams again in a moment.'
const FEED_CHARS = 6
const FEED_MS = 90
const REST_MS = 2400

const content = ref('')
const streaming = ref(true)
const on = ref(true)
let timer = 0

// Fed in small chunks the way a model would; StreamedMarkdown paces the reveal.
const units = segmentStreamUnits(PASSAGE)

function feed(index: number, acc: string, chunk: string) {
  if (index >= units.length) {
    content.value = PASSAGE
    streaming.value = false
    timer = window.setTimeout(() => {
      content.value = ''
      streaming.value = true
      feed(0, '', '')
    }, REST_MS)
    return
  }
  const unit = units[index]!
  const nextAcc = acc + unit
  const nextChunk = chunk + unit
  if (nextChunk.length >= FEED_CHARS) {
    content.value = nextAcc
    timer = window.setTimeout(() => feed(index + 1, nextAcc, ''), FEED_MS)
  } else {
    feed(index + 1, nextAcc, nextChunk)
  }
}

onMounted(() => feed(0, '', ''))
onBeforeUnmount(() => window.clearTimeout(timer))
</script>

<template>
  <!-- The same card as the rows beside it and exactly as tall; a fixed width keeps the
       height from feeding back into the width through an aspect ratio. -->
  <div class="flex h-full min-h-56 w-56 select-none flex-col gap-4 rounded-xl border border-line bg-surface-float p-4 text-fg" aria-hidden="true">
    <!-- The reply, as the transcript renders it, at the chosen size -->
    <div class="min-h-0 flex-1 overflow-hidden" :style="{ '--agent-text': `${fontSize ?? 15}px` }">
      <StreamedMarkdown :content="content" :streaming="streaming" class="text-conversation text-fg-body" />
    </div>
    <!-- Tone: the surfaces stacked -->
    <div class="flex gap-1.5">
      <div class="h-10 flex-1 rounded-md bg-surface-base ring-1 ring-line-subtle" />
      <div class="h-10 flex-1 rounded-md bg-surface ring-1 ring-line-subtle" />
      <div class="h-10 flex-1 rounded-md bg-surface-raised ring-1 ring-line-subtle" />
      <div class="h-10 flex-1 rounded-md bg-surface-float ring-1 ring-line-subtle" />
    </div>
    <!-- A plain button and a switch you can flip: the controls as they are -->
    <div class="flex items-center justify-between">
      <Button size="sm">Button</Button>
      <Switch v-model="on" />
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'

/**
 * What the appearance rows change, at real size: two lines of a reply and a code
 * block rendered by the transcript's own markdown at the chosen text size, the tone's
 * stack of surfaces, and the accent on a primary button and a switch that can be
 * flipped. Everything reads the live tokens.
 */
defineProps<{
  /** Transcript text size in px, applied as the transcript would. */
  fontSize?: number
}>()

// Short lines, so the sample stays two lines of prose and three of code at any size.
const SAMPLE = [
  'Text at this size,\\',
  'line after line.',
  '',
  '```ts',
  'const on = true',
  'if (on) run()',
  'return done',
  '```',
].join('\n')

const on = ref(true)
</script>

<template>
  <!-- The same card as the rows beside it and exactly as tall. The card is absolutely
       placed in a fixed-size frame, so its content never sets the row's height; the
       frame's minimum is what the sample needs at the largest text size. -->
  <div class="relative min-h-78 w-56 self-stretch" aria-hidden="true">
    <div
      class="absolute inset-0 flex select-none flex-col gap-3 overflow-hidden rounded-xl border border-line bg-surface-float p-4 text-fg"
    >
      <div
        class="text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle"
      >Preview</div>
    <!-- Prose and code as the transcript renders them. The size is set directly: the
         text-conversation token resolves at the root, so overriding the variable here
         would not reach it. -->
      <div class="-mt-1 min-h-0 flex-1 overflow-hidden">
        <StreamedMarkdown
          :content="SAMPLE"
          class="text-fg-body"
          :style="{ fontSize: `${fontSize ?? 15}px`, lineHeight: 'var(--agent-leading)' }"
        />
      </div>
    <!-- Tone: the surfaces stacked -->
      <div class="flex gap-1.5">
        <div
          class="h-5 flex-1 rounded-md bg-surface-base ring-1 ring-line-subtle"
        />
        <div class="h-5 flex-1 rounded-md bg-surface ring-1 ring-line-subtle" />
        <div
          class="h-5 flex-1 rounded-md bg-surface-raised ring-1 ring-line-subtle"
        />
        <div
          class="h-5 flex-1 rounded-md bg-surface-float ring-1 ring-line-subtle"
        />
      </div>
    <!-- Accent: a primary button and a switch you can flip -->
      <div class="flex items-center justify-between">
        <Button size="sm" variant="primary">Button</Button>
        <Switch v-model="on" />
      </div>
    </div>
  </div>
</template>

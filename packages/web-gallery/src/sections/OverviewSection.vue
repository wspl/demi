<script setup lang="ts">
import AgentMessageVirtualBlock from '@demicodes/web-ui/agent/blocks/AgentMessageVirtualBlock.vue'
import { PARADIGMS, applyParadigm, galleryState } from '../gallery-state'
import { transcriptDemoBlocks } from '../fixtures/blocks'
import { computed } from 'vue'

const sampleBlocks = computed(() => transcriptDemoBlocks().filter((block) => (
  block.id === 'user-1'
  || block.id === 'thinking-done'
  || block.id === 'tool-shell'
  || block.id === 'assistant-1'
)))

function sampleThinkingEndedAt(index: number): string | null {
  const block = sampleBlocks.value[index]
  const next = sampleBlocks.value[index + 1]
  if (block?.type !== 'thinking' || !next || !('createdAt' in next)) return null
  return next.createdAt
}
</script>

<template>
  <div class="space-y-8">
    <div class="max-w-3xl space-y-3 text-[13px] leading-6 text-fg-body">
      <p>
        Live catalog of <code class="rounded bg-overlay/8 px-1">@demicodes/web-ui</code>.
        Every specimen is the same component the product mounts.
      </p>
      <p>
        The header remaps token values so the same session can be judged under each paradigm.
        Product still ships one light/dark pair.
      </p>
      <p class="text-fg-muted">
        Current: <span class="text-fg">{{ galleryState.paradigm }}</span>
        · {{ galleryState.mode }}
        · tone {{ galleryState.tone }}
        · accent {{ galleryState.accent }}
        · {{ galleryState.density }} / {{ galleryState.radius }} / {{ galleryState.shadow }}
      </p>
    </div>

    <div class="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
      <button
        v-for="item in PARADIGMS"
        :key="item.id"
        type="button"
        class="rounded-xl border p-3 text-left transition-colors duration-200 ease-out"
        :class="galleryState.paradigm === item.id
          ? 'border-line-focus bg-surface-raised'
          : 'border-line bg-surface hover:bg-hover'"
        @click="applyParadigm(item.id)"
      >
        <div class="text-[13px] font-medium text-fg-emphasis">{{ item.name }}</div>
        <p class="mt-1 text-[12px] leading-5 text-fg-muted">{{ item.summary }}</p>
      </button>
    </div>

    <div class="gallery-frame overflow-hidden">
      <div class="border-b border-line px-4 py-2 text-[12px] text-fg-subtle">Same login-test transcript under the current axes.</div>
      <div class="bg-surface py-4">
        <AgentMessageVirtualBlock
          v-for="(block, index) in sampleBlocks"
          :key="block.id"
          :block="block"
          conversation-id="demo"
          :is-thinking-streaming="false"
          :thinking-ended-at="sampleThinkingEndedAt(index)"
        />
      </div>
    </div>
  </div>
</template>

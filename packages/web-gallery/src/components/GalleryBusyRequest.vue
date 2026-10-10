<script setup lang="ts">
import { computed, ref } from 'vue'
import AgentMessageList from '@demicodes/web-ui/agent/AgentMessageList.vue'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'
import { busyRequestBlocks } from '../fixtures/blocks'

/**
 * A request of real work, where rows and prose take turns all the way down:
 * the rows a model's steps leave stay a step behind its words, so the
 * answer reads first. Working stops it at its last command, still running.
 */
const state = ref<'done' | 'working'>('done')
const states = [{ value: 'done', label: 'Done' }, { value: 'working', label: 'Working' }] as const
const blocks = computed(() => busyRequestBlocks(state.value === 'working'))
</script>

<template>
  <GallerySection
    title="A Busy Request"
    note="Short sentences between single commands, thinking before most of them, reads, a failing test, an edit and a summary. The rows stand a step under the prose, 13px in a lighter grey beside its 15px, as ChatGPT’s Thought for 7s stands under its answer, so the model’s words read first and its steps stay findable; a row brightens under the pointer. Working shows the last command still running, its row shimmering."
  >
    <GallerySpecimen variant="rows and prose taking turns" wide>
      <div class="flex flex-col gap-2">
        <Segmented v-model="state" :options="states" size="sm" class="self-start" />
        <div class="gallery-frame h-[44rem] bg-surface">
          <AgentMessageList
            :key="state"
            class="h-full"
            conversation-id="gallery-busy-request"
            :blocks="blocks"
            :pending-steers="[]"
            :queue="[]"
            :phase="state === 'working' ? 'running' : 'idle'"
            :bottom-offset="0"
            :persisted-scroll-state="undefined"
            read-only
          />
        </div>
      </div>
    </GallerySpecimen>
  </GallerySection>
</template>

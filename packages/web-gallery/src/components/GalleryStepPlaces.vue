<script setup lang="ts">
import { computed, ref } from 'vue'
import WorkGroupBlock from '@demicodes/web-ui/agent/blocks/WorkGroupBlock.vue'
import type { WorkGroupBlock as Group, WorkStep } from '@demicodes/web-ui/agent/work-groups'

/**
 * One step in each place the transcript shows it (`runtime.md` § Work
 * groups): alone, as the row of a run of that step only, which is also the
 * row a step that runs has under its run; and inside a run the reader
 * opened, after the step `before`. Each row opens and folds as the
 * product's does.
 */
const props = defineProps<{ step: WorkStep; before: WorkStep }>()

// An ended run ended as the block after it started, now; a running one has not.
const endedAt = new Date().toISOString()
const group = (id: string, steps: WorkStep[]): Group => ({ type: 'work_group', id, steps, ends: steps.map(() => endedAt) })
const alone = computed(() => group(`alone:${props.step.id}`, [props.step]))
const inside = computed(() => group(`inside:${props.step.id}`, [props.before, props.step]))
const aloneOpen = ref(false)
const insideOpen = ref(true)
</script>

<template>
  <div class="grid grid-cols-[9rem_1fr] items-start gap-x-3 gap-y-1">
    <span class="pt-1 text-xs text-fg-faint">Alone</span>
    <div class="overflow-hidden"><WorkGroupBlock v-model:open="aloneOpen" :group="alone" /></div>
    <span class="pt-1 text-xs text-fg-faint">In an opened run</span>
    <div class="overflow-hidden"><WorkGroupBlock v-model:open="insideOpen" :group="inside" /></div>
  </div>
</template>

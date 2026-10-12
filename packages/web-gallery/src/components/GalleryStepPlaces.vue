<script setup lang="ts">
import { computed, ref } from 'vue'
import WorkGroupBlock from '@demicodes/web-ui/agent/blocks/WorkGroupBlock.vue'
import type { WorkGroupBlock as Group, WorkStep } from '@demicodes/web-ui/agent/work-groups'

/**
 * One step in each place the transcript shows it (`runtime.md` § Work
 * groups): alone, as the row of a run of that step only; inside a run the
 * reader opened, after the step `before`; and as the folded face of a run
 * that still runs, whose newest step it is. Each row opens and folds as the
 * product's does.
 */
const props = defineProps<{ step: WorkStep; before: WorkStep }>()

// An ended run ended as the block after it started, now; a running one has not.
const endedAt = new Date().toISOString()
const group = (id: string, steps: WorkStep[], live: boolean): Group => ({ type: 'work_group', id, steps, live, endedAt: live ? null : endedAt })
// A call being written stands only at a running turn's end.
const alone = computed(() => group(`alone:${props.step.id}`, [props.step], props.step.type === 'pending_call'))
const inside = computed(() => group(`inside:${props.step.id}`, [props.before, props.step], false))
const face = computed(() => group(`face:${props.step.id}`, [props.before, props.step], true))
const aloneOpen = ref(false)
const insideOpen = ref(true)
const faceOpen = ref(false)
</script>

<template>
  <div class="grid grid-cols-[9rem_1fr] items-start gap-x-3 gap-y-1">
    <span class="pt-1 text-xs text-fg-faint">Alone</span>
    <div class="overflow-hidden"><WorkGroupBlock v-model:open="aloneOpen" :group="alone" /></div>
    <span class="pt-1 text-xs text-fg-faint">In an opened run</span>
    <div class="overflow-hidden"><WorkGroupBlock v-model:open="insideOpen" :group="inside" /></div>
    <span class="pt-1 text-xs text-fg-faint">A running run’s face</span>
    <div class="overflow-hidden"><WorkGroupBlock v-model:open="faceOpen" :group="face" /></div>
  </div>
</template>

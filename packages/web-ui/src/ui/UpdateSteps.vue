<script setup lang="ts">
import { Check, Circle, Minus, X } from '@lucide/vue'
import IndeterminateSpinner from './IndeterminateSpinner.vue'
import type { StepLine } from './update-steps'

/**
 * A server update's steps, one line each, as GitHub Actions lists a job's: a
 * mark (done, running, waiting, skipped, failed), the step, and on the right
 * what it did or how far it is; on a phone that goes under the step, in a
 * line of its own. Every step has one height, its detail's line kept when it
 * has none, and the list keeps every step from the start, so nothing moves as
 * the update goes on; a detail too long for its line is cut, whole in its
 * tooltip.
 */
defineProps<{
  lines: readonly StepLine[]
}>()
</script>

<template>
  <ol class="flex flex-col">
    <li
      v-for="line in lines"
      :key="line.key"
      class="grid h-7 min-w-0 grid-cols-[1rem_auto_1fr] items-center gap-x-2.5 text-chrome phone:h-12 phone:grid-cols-[1rem_1fr] phone:grid-rows-2 phone:gap-y-0"
      :aria-current="line.mark === 'running' ? 'step' : undefined"
    >
      <span class="flex size-4 shrink-0 items-center justify-center phone:row-span-1" :class="{
        'text-on-success': line.mark === 'done',
        'text-fg-muted': line.mark === 'running' || line.mark === 'waiting' || line.mark === 'skipped',
        'text-on-danger': line.mark === 'failed',
      }">
        <Check v-if="line.mark === 'done'" :size="14" :stroke-width="2.25" />
        <IndeterminateSpinner v-else-if="line.mark === 'running'" :size="13" />
        <Minus v-else-if="line.mark === 'skipped'" :size="14" />
        <X v-else-if="line.mark === 'failed'" :size="14" :stroke-width="2.25" />
        <Circle v-else :size="10" class="opacity-60" />
      </span>
      <span
        class="min-w-0 shrink-0 truncate"
        :class="line.mark === 'waiting' || line.mark === 'skipped' ? 'text-fg-muted' : 'text-fg'"
      >{{ line.title }}</span>
      <span
        class="min-w-0 truncate pl-3 text-right tabular-nums phone:col-start-2 phone:pl-0 phone:text-left"
        :class="line.mark === 'failed' ? 'text-on-danger' : 'text-fg-muted'"
        :title="line.detail || undefined"
      >{{ line.detail }}</span>
    </li>
  </ol>
</template>

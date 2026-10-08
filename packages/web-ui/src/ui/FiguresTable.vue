<script setup lang="ts">
import StatusDot from './StatusDot.vue'
import type { SentenceText, TitleText } from './ui-text'

/**
 * Figures that are measured, not set: a small table under the group they
 * inform, as macOS's Network settings show a connection's statistics, never
 * as settings rows. A caption names where they come from and an action at
 * its end refreshes them; each row names what was measured, the row in use
 * is marked with a dot, and a figure not known yet is a dash.
 */
defineProps<{
  /** Where the figures come from: "Measured from this browser". */
  caption: SentenceText
  columns: readonly { key: string; label: TitleText }[]
  rows: readonly {
    key: string
    label: TitleText
    /** Each column's figure, already formatted; a missing one shows a dash. */
    values: Readonly<Record<string, string | null>>
    /** The row in use, marked with a dot. */
    current?: boolean
  }[]
  /** What the dot of the row in use means: "In use". */
  currentLabel?: SentenceText
}>()
</script>

<template>
  <div class="flex flex-col gap-2 px-4">
    <div class="flex min-h-6 items-center justify-between gap-3">
      <span class="select-none text-[12px] leading-4 text-fg-subtle">{{ caption }}</span>
      <slot name="action" />
    </div>
    <table class="w-full table-fixed text-[13px] leading-5">
      <thead>
        <tr class="text-left text-[12px] text-fg-subtle">
          <th class="w-28 pb-1 font-normal" scope="col"><span class="sr-only">Path</span></th>
          <th v-for="column in columns" :key="column.key" class="pb-1 font-normal" scope="col">
            {{ column.label }}
          </th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="row in rows" :key="row.key" class="tabular-nums">
          <th scope="row" class="py-0.5 text-left font-normal text-fg">
            <span class="flex items-center gap-1.5">
              <StatusDot :tone="row.current ? 'success' : null" :label="row.current ? currentLabel : undefined" />
              {{ row.label }}
            </span>
          </th>
          <td v-for="column in columns" :key="column.key" class="py-0.5 text-fg-muted">
            {{ row.values[column.key] ?? '—' }}
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>

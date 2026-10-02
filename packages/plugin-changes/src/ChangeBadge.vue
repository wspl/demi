<script setup lang="ts">
import { computed } from 'vue'
import { usePage } from '@demicodes/plugin-sdk'
import type { ChangeData } from './data'

/**
 * The strip's counts beside Change: the lines the working tree adds and
 * removes. The badge shows while the panel is open, so the working tree
 * stays fresh as long.
 */
const props = defineProps<{ conversation: string; data: ChangeData }>()

const files = usePage().files(props.conversation)
files.showChanges()
const totals = computed(() => files.changes.files.reduce(
  (sum, file) => ({ added: sum.added + file.added, removed: sum.removed + file.removed }),
  { added: 0, removed: 0 },
))
</script>

<template>
  <span class="flex items-center gap-0.5 text-[11px] tabular-nums">
    <span class="text-on-success">+{{ totals.added }}</span>
    <span class="text-on-danger">−{{ totals.removed }}</span>
  </span>
</template>

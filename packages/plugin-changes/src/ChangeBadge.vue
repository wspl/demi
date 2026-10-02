<script setup lang="ts">
import { computed } from 'vue'
import type { ChangeSetSource } from '@demicodes/plugin-sdk'

/** The strip's counts beside Change: the lines the working tree adds and removes. */
const props = defineProps<{ changes: ChangeSetSource }>()

const totals = computed(() => props.changes.files.reduce(
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

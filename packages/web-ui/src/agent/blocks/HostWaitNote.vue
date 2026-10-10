<script setup lang="ts">
/**
 * What a running call's row says after its title while it waits for its
 * Host's runner: *Waiting for Old Laptop* (`sessions-and-targets.md` § Host
 * operations). Set apart from the title, as a report row's outcome is, it
 * stays whole while a long title truncates.
 */
defineProps<{ host: string }>()
</script>

<template>
  <span class="-ml-1 shrink-0 text-fg-subtle">· Waiting for {{ host }}</span>
</template>

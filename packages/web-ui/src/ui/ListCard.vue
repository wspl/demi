<script setup lang="ts">
/**
 * A short list of things inside the transcript or a pane, on one framed
 * surface, the rows divided by a hairline: the pages a command presented,
 * for one. Its rows are `ListCardRow`s; a single thing is a list of one.
 * The frame is the transcript's well (the framed output of a shell call), so
 * the list sits among the rows around it without outweighing them.
 */
</script>

<template>
  <div
    class="flex min-w-0 flex-col divide-y divide-line-subtle overflow-hidden rounded-md border border-line-subtle bg-surface-base"
    role="list"
  >
    <slot />
  </div>
</template>

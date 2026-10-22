<script setup lang="ts">
import { providePane } from './pane-ready'

/**
 * The pane of one tab's content in the work panel (`pane-ready.ts`): its
 * parts say whether they are read, and the areas that wait ask. The element
 * is the content's own box.
 */
const props = defineProps<{
  /** What the tab is called. */
  name: string
  /** Whether the tab is the selected one. */
  shown: boolean
  /** Whether it holds the first view the panel shows, which says what it reads from the start. */
  first?: boolean
}>()

providePane({ name: () => props.name, shown: () => props.shown, first: props.first })
</script>

<template>
  <div>
    <slot />
  </div>
</template>

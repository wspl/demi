<script setup lang="ts">
import { provideBlockScope, type ShownBlock } from '../whole-blocks'

/** Names the block its content shows, for a row in it that opens to read the block whole. */
const props = defineProps<{ block: ShownBlock }>()
provideBlockScope(() => props.block)
</script>

<template>
  <slot />
</template>

<script setup lang="ts">
import type { HeldBlock } from '@demicodes/conversation-client'
import { provideBlockScope } from '../whole-blocks'

/** Names the block its content shows, for a row in it that opens to read the block whole. */
const props = defineProps<{ block: HeldBlock }>()
provideBlockScope(() => props.block)
</script>

<template>
  <slot />
</template>

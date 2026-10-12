<script setup lang="ts">
import CommandEndTag from './CommandEndTag.vue'
import CommandTimeTag from './CommandTimeTag.vue'
import type { ToolCallBlock } from '../block-types'
import { useShellRow } from '../shell-row'

/**
 * The tag after a `shell` row's title (`runtime.md` § Rendering boundary):
 * how its command went wrong once it ended, or, while it runs past 3
 * seconds, how long it has run, counting up; nothing for a quick or a
 * succeeded command. Its own row wears it, and so does a running run's
 * folded face while that row is its newest step (§ Work groups).
 */
const props = defineProps<{ block: ToolCallBlock }>()

const { endMark } = useShellRow(() => props.block)
</script>

<template>
  <CommandEndTag v-if="endMark" :mark="endMark" />
  <CommandTimeTag v-else :calls="[block]" />
</template>

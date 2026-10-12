<script setup lang="ts">
import CommandEndTag from './CommandEndTag.vue'
import CommandTimeTag from './CommandTimeTag.vue'
import RunEndTag from './RunEndTag.vue'
import type { ToolCallBlock } from '../block-types'
import { useShellRow } from '../shell-row'

/**
 * The tag after a `shell` row's title (`runtime.md` § Rendering boundary):
 * how its command went wrong once it ended; Failed for a call that ran
 * nothing, as one the input check refused; or, while its command runs past
 * 3 seconds, how long it has run, counting up; nothing for a quick or a
 * succeeded command.
 */
const props = defineProps<{ block: ToolCallBlock }>()

const { endMark } = useShellRow(() => props.block)
</script>

<template>
  <CommandEndTag v-if="endMark" :mark="endMark" />
  <RunEndTag v-else-if="block.status === 'error'" kind="failed" />
  <CommandTimeTag v-else :calls="[block]" />
</template>

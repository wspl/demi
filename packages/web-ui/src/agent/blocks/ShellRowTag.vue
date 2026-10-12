<script setup lang="ts">
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import CommandEndTag from './CommandEndTag.vue'
import type { ToolCallBlock } from '../block-types'
import { useShellRow } from '../shell-row'

/**
 * The tag after a `shell` row's title (`runtime.md` § Rendering boundary):
 * how its command went wrong once it ended, or, while it runs past 3
 * seconds, how long it has run, counting up; nothing for a quick or a
 * succeeded command. Its own row wears it, and so does a folded run whose
 * face is that row (§ Work groups).
 */
const props = defineProps<{ block: ToolCallBlock }>()

const { endMark, runningTag } = useShellRow(() => props.block)
</script>

<template>
  <CommandEndTag v-if="endMark" :mark="endMark" />
  <Tag v-else-if="runningTag" class="shrink-0 tabular-nums">{{ runningTag }}</Tag>
</template>

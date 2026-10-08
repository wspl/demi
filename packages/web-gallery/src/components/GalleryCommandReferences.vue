<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import type { Block } from '@demicodes/protocol'
import ToolCallBlock from '@demicodes/web-ui/agent/blocks/ToolCallBlock.vue'
import { provideBlockJump } from '@demicodes/web-ui/agent/block-jump'
import { commandCalls, provideCommandReferences } from '@demicodes/web-ui/agent/command-references'
import { toolCallTitle } from '@demicodes/web-ui/agent/block-helpers'
import { highlightFound } from '@demicodes/web-ui/ui/found-highlight'

/**
 * A short transcript whose looks and waits name its commands: a reference
 * jumps to its command's call and marks it, as the message list does; a
 * command of another agent's transcript, `others`, is named by its title
 * alone.
 */
const props = defineProps<{
  blocks: Block[]
  others: Record<string, string>
}>()

const root = ref<HTMLElement>()
const calls = computed(() => commandCalls(props.blocks))
provideCommandReferences((commandId) => {
  const call = calls.value.get(commandId)
  if (call)
    return { title: toolCallTitle(call), blockId: call.id }
  const other = props.others[commandId]
  return other === undefined ? undefined : { title: other }
})
let marking: AbortController | null = null
provideBlockJump((id) => {
  if (!root.value)
    return
  marking?.abort()
  marking = new AbortController()
  highlightFound(root.value, `[data-block-id="${CSS.escape(id)}"]`, marking.signal)
})
onBeforeUnmount(() => marking?.abort())
</script>

<template>
  <div ref="root" class="flex flex-col">
    <div v-for="block in blocks" :key="block.id" :data-block-id="block.id" class="rounded-md">
      <ToolCallBlock v-if="block.type === 'tool_call'" :block="block" :is-streaming="false" />
    </div>
  </div>
</template>

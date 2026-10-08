<script setup lang="ts">
import { computed } from 'vue'
import { toolRowIcon } from './tool-row-icon'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import Reference from '@demicodes/web-ui/ui/Reference.vue'
import PresentedPages from './PresentedPages.vue'
import ShellEditPills from './ShellEditPills.vue'
import FunctionalBlock from './FunctionalBlock.vue'
import ToolMedia from './ToolMedia.vue'
import CommandEndTag from './CommandEndTag.vue'
import { commandEndMark } from '../command-end'
import type { ToolCallBlock } from '../block-types'
import { getToolErrorText, storedShellView } from '../block-helpers'
import { useBlockJump } from '../block-jump'
import { useCommandReferences } from '../command-references'
import {
  standardToolTitle,
  standardToolTitleParts,
  trimToolSummary,
  unknownCommand,
  type ControlToolName,
} from '../tool-rendering'

const props = defineProps<{
  block: ToolCallBlock
  input: Record<string, unknown>
  toolName: ControlToolName
}>()

const references = useCommandReferences()
const jump = useBlockJump()
const parts = computed(() => standardToolTitleParts(props.toolName, props.input))
/** The command the title names, and how a click on it reaches that command's call. */
const reference = computed(() => {
  const title = parts.value
  if (title.kind !== 'reference')
    return null
  const found = references(title.commandId)
  const blockId = found?.blockId
  return {
    text: found?.title ?? unknownCommand(title.commandId),
    follow: blockId && jump ? () => jump(blockId) : undefined,
  }
})
const title = computed(() =>
  standardToolTitle(props.toolName, props.input, (commandId) => references(commandId)?.title),
)
/** What went wrong with the command, as the look found it (`runtime.md` § Rendering boundary). */
const endMark = computed(() =>
  props.toolName === 'shell_status' ? commandEndMark(storedShellView(props.block)) : null,
)
const errorText = computed(() => getToolErrorText(props.block))
const errorSummary = computed(() => {
  const text = errorText.value
  return text ? trimToolSummary(text, 160) : ''
})
const iconComponent = computed(() => toolRowIcon(props.toolName))
</script>

<template>
  <FunctionalBlock
    :loading="block.status === 'executing'"
    :tone="block.status === 'error' ? 'danger' : undefined"
  >
    <template #icon>
      <component :is="iconComponent" :size="ICON_PX.in28" />
    </template>

    <template #default="{ loading }">
      <!-- The words around a reference keep their place; the reference
        gives up its end to an ellipsis when the row has no room. -->
      <span
        v-if="parts.kind === 'reference' && reference"
        class="flex min-w-0 items-center gap-x-[0.3em]"
        :class="loading ? 'thinking-shimmer' : ''"
      >
        <span class="shrink-0">{{ parts.lead }}</span>
        <Reference :text="reference.text" :follow="reference.follow" />
        <span v-if="parts.trail" class="shrink-0">{{ parts.trail }}</span>
      </span>
      <span v-else class="min-w-0 truncate" :class="loading ? 'thinking-shimmer' : ''">{{ title }}</span>
      <CommandEndTag v-if="endMark" :mark="endMark" />
      <span
        v-if="errorSummary"
        class="min-w-0 truncate font-mono text-fg-subtle"
      >{{ errorSummary }}</span>
    </template>
  </FunctionalBlock>
  <ToolMedia :output="block.output" :title="title" />
  <ShellEditPills :block="block" />
  <PresentedPages :block="block" />
</template>

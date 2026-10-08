<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useResizeObserver } from '@vueuse/core'
import { SquareTerminal } from '@lucide/vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import AnsiText from './AnsiText.vue'
import ShellEditPills from './ShellEditPills.vue'
import FunctionalBlock from './FunctionalBlock.vue'
import ToolMedia from './ToolMedia.vue'
import type { ToolCallBlock } from '../block-types'
import { getToolErrorText, shellTerminalOutputChunks } from '../block-helpers'
import { commandEndMark, commandEndWords } from '../command-end'
import CommandEndTag from './CommandEndTag.vue'
import { useLiveCalls } from '../live-calls'
import { shellRowEnd, shellRowRunning } from '../terminals'
import { standardToolTitle } from '../tool-rendering'

const props = defineProps<{
  block: ToolCallBlock
  input: Record<string, unknown>
}>()

const command = computed(() => (props.input['script'] as string) ?? '')
const title = computed(() => standardToolTitle('shell_exec', props.input))
const errorText = computed(() => getToolErrorText(props.block))
const liveCalls = useLiveCalls()
/** The command the call started, as its live frames show it. */
const started = computed(() => liveCalls(props.block.toolUseId))
// How the command ended, also when it ran on after the call returned; none while it runs.
const end = computed(() => shellRowEnd(props.block, started.value))
const endMark = computed(() => commandEndMark(end.value))
const endWords = computed(() => commandEndWords(end.value))
/** While the call runs, its command's output as it comes (`runtime.md` § Rendering boundary). */
const liveOutput = computed(() =>
  props.block.status === 'executing' ? started.value?.output ?? '' : '',
)
/** The row shimmers while its call runs and, after, while its command still does. */
const running = computed(() => shellRowRunning(props.block.status, started.value))
// Once the call returned, the view its result stored.
const terminalOutputText = computed(
  () => liveOutput.value
    || shellTerminalOutputChunks(props.block).map((chunk) => chunk.text).join('')
)
// Only the user opens the row, while the call runs as after it returned.
const isOpen = defineModel<boolean>('open', { default: false })

/** How many lines the command shows until a click shows it whole: the template's `line-clamp-2`. */
const COMMAND_LINES = 2

const commandRef = ref<HTMLElement>()
// Whether the command takes more lines than it shows clamped; only then is it a toggle.
const commandOverflows = ref(false)
const commandWhole = ref(false)

// The element's scroll height is the command's full height, clamped or not.
function measureCommand(): void {
  const element = commandRef.value
  if (!element) {
    commandOverflows.value = false
    return
  }
  const line = Number.parseFloat(getComputedStyle(element).lineHeight) || 0
  commandOverflows.value = element.scrollHeight > COMMAND_LINES * line + 1
}

// Wrapping follows the width, and a clamped box keeps its size while the
// command streams in, so the text is watched as well.
useResizeObserver(commandRef, measureCommand)
watch(command, measureCommand, { flush: 'post' })

function toggleCommand(): void {
  if (!commandOverflows.value) {
    return
  }
  // A drag that selected part of the command is a selection, not a click.
  const selection = window.getSelection()
  if (selection && !selection.isCollapsed && commandRef.value?.contains(selection.anchorNode)) {
    return
  }
  commandWhole.value = !commandWhole.value
}
</script>

<template>
  <FunctionalBlock
    v-model:open="isOpen"
    :loading="running"
    :tone="block.status === 'error' && !endMark ? 'danger' : undefined"
    :error-text="errorText"
    :stick-bottom="block.status === 'executing'"
    framed
  >
    <template #icon>
      <SquareTerminal :size="ICON_PX.in28" />
    </template>

    <template #default="{ loading }">
      <span class="min-w-0 truncate" :class="loading ? 'thinking-shimmer' : ''">{{ title }}</span>
      <CommandEndTag v-if="endMark" :mark="endMark" />
    </template>

    <template #pinned>
      <div class="flex px-3 font-mono text-xs leading-5 text-fg-subtle">
        <span class="mr-1 shrink-0 select-none text-fg-faint">$</span><!--
          A command longer than two lines shows two, the second ending in an
          ellipsis; a click shows it whole and another clamps it again. One
          that fits is no control: a click selects it whole for copying.
        --><span
          ref="commandRef"
          class="min-w-0 terminal-wrap"
          :class="[
            commandOverflows ? 'select-text transition-colors duration-200 ease-out hover:text-fg-body' : 'select-all',
            commandOverflows && commandWhole ? '' : 'line-clamp-2',
          ]"
          :role="commandOverflows ? 'button' : undefined"
          :tabindex="commandOverflows ? 0 : undefined"
          :aria-expanded="commandOverflows ? commandWhole : undefined"
          @click="toggleCommand"
          @keydown.enter.self.prevent="toggleCommand"
          @keydown.space.self.prevent="toggleCommand"
        >{{ command }}</span>
      </div>
    </template>

    <template v-if="terminalOutputText || endWords" #body>
      <div v-if="endWords" class="px-3 text-xs leading-5 text-fg-subtle">{{ endWords }}</div>
      <AnsiText v-if="terminalOutputText" class="px-3" :content="terminalOutputText" />
    </template>
  </FunctionalBlock>
  <ToolMedia :output="block.output" :title="title" />
  <ShellEditPills :block="block" />
</template>

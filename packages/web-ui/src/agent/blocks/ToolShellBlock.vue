<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useResizeObserver } from '@vueuse/core'
import { SquareTerminal } from '@lucide/vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import AnsiText from './AnsiText.vue'
import PresentedPages from './PresentedPages.vue'
import ShellEditPills from './ShellEditPills.vue'
import FunctionalBlock from './FunctionalBlock.vue'
import ToolMedia from './ToolMedia.vue'
import type { ToolCallBlock } from '../block-types'
import { getToolErrorText, shellTerminalOutputChunks } from '../block-helpers'
import { useLiveCalls } from '../live-calls'
import { standardToolTitle } from '../tool-rendering'

const props = defineProps<{
  block: ToolCallBlock
  input: Record<string, unknown>
  isStreaming: boolean
}>()

const command = computed(() => (props.input['script'] as string) ?? '')
const title = computed(() => standardToolTitle('shell_exec', props.input))
const errorText = computed(() => getToolErrorText(props.block))
const liveCalls = useLiveCalls()
/** While the call runs, its command's output as it comes (`runtime.md` § Rendering boundary). */
const liveOutput = computed(() =>
  props.block.status === 'executing' ? liveCalls(props.block.toolUseId)?.output ?? '' : '',
)
// Once the call returned, the view its result stored.
const terminalOutputText = computed(
  () => liveOutput.value
    || shellTerminalOutputChunks(props.block).map((chunk) => chunk.text).join('')
)
const isOpen = defineModel<boolean>('open', { default: false })
// A running call opens once its command's output starts to come, and also
// when its row mounts with the output already coming: the row takes over from
// the activity slot only after the slot's roll, by which time the first lines
// have usually arrived, and a page opened while the call runs shows what the
// command prints now. A fold by the user holds until the call returns.
watch(
  () => liveOutput.value !== '',
  (coming) => {
    if (coming) {
      isOpen.value = true
    }
  },
  { immediate: true },
)

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
    :open-while="block.status === 'executing' || isStreaming"
    :loading="block.status === 'executing'"
    :tone="block.status === 'error' ? 'danger' : undefined"
    :error-text="errorText"
    :stick-bottom="block.status === 'executing' || isStreaming"
    framed
  >
    <template #icon>
      <SquareTerminal :size="ICON_PX.in28" />
    </template>

    <template #default="{ loading }">
      <span class="min-w-0 truncate" :class="loading ? 'thinking-shimmer' : ''">{{ title }}</span>
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

    <template v-if="terminalOutputText" #body>
      <AnsiText class="px-3" :content="terminalOutputText" />
    </template>
  </FunctionalBlock>
  <ToolMedia :output="block.output" :title="title" />
  <ShellEditPills :block="block" />
  <PresentedPages :block="block" />
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch, type Component } from 'vue'
import { useElementVisibility } from '@vueuse/core'
import { Brain, Layers } from '@lucide/vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import Fold from '@demicodes/web-ui/ui/Fold.vue'
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import { CHROME_ENTER_MS, chromeEntrance } from '@demicodes/web-ui/ui/chrome-enter'
import FunctionalBlock from './FunctionalBlock.vue'
import ThinkingBlock from './ThinkingBlock.vue'
import ToolCallBlock from './ToolCallBlock.vue'
import PendingCallBlock from './PendingCallBlock.vue'
import FileChangePills from './FileChangePills.vue'
import { toolRowIcon } from './tool-row-icon'
import { storedShellView, toolCallTitle } from '../block-helpers'
import { pendingCallTitle, toolRenderKind } from '../tool-rendering'
import { thinkingFaceLabel } from '../thinking-label'
import { stepKey, workRunning, workSummary, type WorkGroupBlock, type WorkStep } from '../work-groups'
import { useEditSelection, useTranscript } from '../edit-selection'
import { useCommandReferences } from '../command-references'
import { callFiles, pillSelection } from '../../files/request-changes'
import type { ChangeFile } from '../../files/changes'
import { useFileLineCounts } from '../useFileLineCounts'
import { useElapsedTime } from '../../composables/useElapsedTime'

/**
 * Consecutive steps as one row. While they run, the folded row's face is the
 * newest step, which rolls over the one before, and the open row's a stack
 * and what runs; once they end, a stack and what they did. A lone thinking is
 * the thinking's own row, so the first call rolls over it in place. Opening
 * the row shows every step as its own row; a step that joins while it is open
 * enters as a row joining the transcript does. Folded, the files the steps
 * changed show under it; open, each under its own call.
 */
const props = defineProps<{ group: WorkGroupBlock }>()
const isOpen = defineModel<boolean>('open', { default: false })

const newest = computed(() => props.group.steps.at(-1)!)
const lone = computed(() => {
  const steps = props.group.steps
  return steps.length === 1 && steps[0]!.type === 'thinking' ? steps[0]! : null
})
/** The newest step is thinking that still streams. */
const thinkingLive = computed(() => props.group.live && newest.value.type === 'thinking')

/** When the step at `index` ended: the next step's start, or the group's end. */
function endedAt(index: number): string | null {
  const next = props.group.steps[index + 1]
  return next && next.type !== 'pending_call' ? next.createdAt : props.group.endedAt
}

const loneElapsed = useElapsedTime(
  () => (lone.value ? Date.parse(lone.value.createdAt) : Date.now()),
  () => thinkingLive.value,
  () => {
    const end = endedAt(0)
    return end ? Date.parse(end) : null
  },
)
const loneText = computed(() => lone.value?.text.trim() ?? '')
const references = useCommandReferences()

function stepTitle(step: WorkStep): string {
  switch (step.type) {
    case 'thinking':
      return 'Thinking'
    case 'tool_call':
      // As the step's own row names it, a referred command by its title.
      return toolCallTitle(step, (commandId) => references(commandId)?.title)
    case 'pending_call':
      return pendingCallTitle(step.call)
  }
}

interface Face {
  icon: Component
  iconKey: string
  label: string
  loading: boolean
  rollKey: string
}

const face = computed<Face>(() => {
  if (lone.value) {
    return {
      icon: Brain,
      iconKey: 'thinking',
      label: thinkingFaceLabel(thinkingLive.value, loneElapsed.value),
      loading: thinkingLive.value,
      rollKey: thinkingLive.value ? `${lone.value.id}:live` : `${lone.value.id}:done`,
    }
  }
  if (!props.group.live) {
    return { icon: Layers, iconKey: 'stack', label: workSummary(props.group.steps), loading: false, rollKey: 'done' }
  }
  // Open, the steps show under the row, so it stands still as the stack of
  // what runs; folded again, it shows the newest step. Opening and folding
  // change what the row shows, not how far the work got, so the face cuts
  // over at once rather than rolling.
  if (isOpen.value) {
    return { icon: Layers, iconKey: 'stack', label: workRunning(props.group.steps), loading: true, rollKey: 'running' }
  }
  const step = newest.value
  return {
    icon: step.type === 'thinking' ? Brain : toolRowIcon(step.type === 'tool_call' ? step.toolName : step.call.toolName),
    iconKey: step.type === 'thinking' ? 'thinking' : toolRenderKind(step.type === 'tool_call' ? step.toolName : step.call.toolName),
    label: stepTitle(step),
    loading: step.type !== 'tool_call' || step.status === 'executing',
    // A call's placeholder title rolls over to its description once written;
    // the call's block keeps that face, so it does not roll again.
    rollKey: step.type === 'pending_call' && step.call.description === null ? `${stepKey(step)}:writing` : stepKey(step),
  }
})
const calls = computed(() => props.group.steps.filter((step) => step.type === 'tool_call'))
const changed = computed(() => callFiles(calls.value))
// Folded, each file the steps changed shows once, with the lines of its
// changes across them counted from its two ends, as the Change view's All
// Changes counts them; until they are counted, or when its ends were not kept, its name alone.
const pills = ref<HTMLElement | null>(null)
const pillsShown = useElementVisibility(pills)
const counts = useFileLineCounts(() => changed.value, () => pillsShown.value && !isOpen.value)
const files = computed<ChangeFile[]>(() => changed.value.map((file) => {
  const count = counts.value.get(file.path)
  return { path: file.path, kind: file.kind, added: count?.added ?? 0, removed: count?.removed ?? 0 }
}))

// A step enters once, as it first appears, the way a row joining the
// transcript does: one that joins while the row is open slides in; opening
// the row shows the steps that were there, still.
const entering = ref(new Set<string>())
const timers = new Set<ReturnType<typeof setTimeout>>()
const keys = computed(() => props.group.steps.map(stepKey))
watch(keys, (now, before) => {
  const joined = now.filter((key) => !before.includes(key))
  if (joined.length === 0)
    return
  entering.value = new Set([...entering.value, ...joined])
  const timer = setTimeout(() => {
    timers.delete(timer)
    entering.value = new Set([...entering.value].filter((key) => !joined.includes(key)))
  }, CHROME_ENTER_MS)
  timers.add(timer)
})
onBeforeUnmount(() => {
  for (const timer of timers)
    clearTimeout(timer)
  timers.clear()
})

const select = useEditSelection()
const transcript = useTranscript()
/** A summed pill opens its file at the first of the group's calls that changed it. */
function selection(path: string) {
  if (!transcript)
    return null
  const call = calls.value.find((step) => storedShellView(step)?.files?.some((file) => file.path === path))
  return call ? pillSelection(transcript.node, transcript.requests(), call, path) : null
}
const selectable = computed(() => {
  const first = files.value[0]
  return select() !== undefined && first !== undefined && selection(first.path) !== null
})
function pick(path: string): void {
  const picked = selection(path)
  if (picked)
    select()?.(picked)
}
</script>

<template>
  <FunctionalBlock
    v-model:open="isOpen"
    :expandable="!lone || loneText !== ''"
    :flow="!lone"
    :open-while="lone ? thinkingLive && loneText !== '' : undefined"
    :stick-bottom="lone ? thinkingLive : false"
    :roll-key="face.rollKey"
    :icon-key="face.iconKey"
    :cut-key="isOpen && !lone ? 'open' : 'folded'"
  >
    <template #icon>
      <component :is="face.icon" :size="ICON_PX.in28" />
    </template>
    <span class="min-w-0 truncate" :class="face.loading ? 'thinking-shimmer' : ''">{{ face.label }}</span>
    <!-- A thinking with no text has nothing to open: no body, so no chevron. -->
    <template v-if="!lone || loneText !== ''" #body>
      <StreamedMarkdown
        v-if="lone"
        :content="lone.text"
        :streaming="thinkingLive"
        class="px-3 py-1 text-[13px] leading-4.5 text-fg-subtle [--markdown-block-gap:--spacing(1)]"
      />
      <div v-else class="[--agent-pad-x:0px]">
        <div
          v-for="(step, index) in group.steps"
          :key="stepKey(step)"
          v-bind="chromeEntrance(entering.has(stepKey(step)))"
        >
          <ThinkingBlock
            v-if="step.type === 'thinking'"
            :thinking="step.text"
            :is-streaming="thinkingLive && index === group.steps.length - 1"
            :created-at="step.createdAt"
            :ended-at="endedAt(index)"
          />
          <ToolCallBlock v-else-if="step.type === 'tool_call'" :block="step" />
          <PendingCallBlock v-else :call="step.call" />
        </div>
      </div>
    </template>
  </FunctionalBlock>
  <!-- The summed pills fold in as the first file changes, and away as the group opens to show each call's own. -->
  <Fold :open="!isOpen && files.length > 0">
    <div ref="pills">
      <FileChangePills
        v-if="files.length > 0"
        class="py-1"
        :style="{ paddingLeft: `${ICON_PX.in28 + 8}px` }"
        :files="files"
        :selectable="selectable"
        @select="pick"
      />
    </div>
  </Fold>
</template>

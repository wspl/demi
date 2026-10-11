<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch, type Component } from 'vue'
import { Brain, Layers } from '@lucide/vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import Fold from '@demicodes/web-ui/ui/Fold.vue'
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import { CHROME_ENTER_MS, chromeEntrance } from '@demicodes/web-ui/ui/chrome-enter'
import FunctionalBlock from './FunctionalBlock.vue'
import ThinkingBlock from './ThinkingBlock.vue'
import ToolCallBlock from './ToolCallBlock.vue'
import PendingCallBlock from './PendingCallBlock.vue'
import CommandReportBlock from './CommandReportBlock.vue'
import AgentReceiptBlock from './AgentReceiptBlock.vue'
import NoticeStepFace from './NoticeStepFace.vue'
import FileChangePills from './FileChangePills.vue'
import ToolMedia from './ToolMedia.vue'
import { stepIcon } from './tool-row-icon'
import { toolCallTitle } from '../block-helpers'
import { pendingCallTitle } from '../tool-rendering'
import { thinkingFaceLabel } from '../thinking-label'
import { isNoticeStep, stepKey, workRunning, workSummary, type NoticeStep, type WorkGroupBlock, type WorkStep } from '../work-groups'
import { useEditSelection, useTranscript } from '../edit-selection'
import { changeFile, pillSelection, runFiles } from '../../files/request-changes'
import { foldedMedia } from '../tool-media'
import { useElapsedTime } from '../../composables/useElapsedTime'
import BlockScope from './BlockScope.vue'

/**
 * Consecutive steps as one row (`runtime.md` § Work groups). While they are
 * the end of a running turn, the row is the turn's activity and shimmers
 * until the turn moves past it, also while the model is waited for after its
 * newest step ended; the folded face is the newest step, which rolls over the
 * one before, and the open row's a stack and what runs. Once they end, a
 * stack and what they did. A lone thinking is the thinking's own row, so the
 * first call rolls over it in place; a lone call is the call's own row, which
 * opens to the call, and a step that joins it turns the same row into the
 * stack, open if the reader opened the call; a lone notice or receipt is its
 * own notice row likewise. Opening the row shows every step as its own row,
 * a command's notice and an agent's receipt among them; a step that joins
 * while it is open enters as a row joining the transcript does. Folded, the
 * media the calls and notices returned show under it, and below them the
 * files the steps changed; open, each under its own row.
 */
const props = defineProps<{ group: WorkGroupBlock }>()
const isOpen = defineModel<boolean>('open', { default: false })

const newest = computed(() => props.group.steps.at(-1)!)
const lone = computed(() => {
  const steps = props.group.steps
  return steps.length === 1 && steps[0]!.type === 'thinking' ? steps[0]! : null
})
const loneCall = computed(() => {
  const steps = props.group.steps
  return steps.length === 1 && steps[0]!.type === 'tool_call' ? steps[0]! : null
})
const loneNotice = computed(() => {
  const steps = props.group.steps
  return steps.length === 1 && isNoticeStep(steps[0]!) ? steps[0]! : null
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

function stepTitle(step: Exclude<WorkStep, NoticeStep>): string {
  switch (step.type) {
    case 'thinking':
      return 'Thinking'
    case 'tool_call':
      return toolCallTitle(step)
    case 'pending_call':
      return pendingCallTitle(step.call)
  }
}

interface Face {
  icon: Component
  iconKey: string
  label: string
  /** The newest step is a notice or a receipt: the face says it as its own row does. */
  notice: NoticeStep | null
  rollKey: string
}

const face = computed<Face>(() => {
  if (lone.value) {
    return {
      icon: Brain,
      iconKey: 'thinking',
      label: thinkingFaceLabel(thinkingLive.value, loneElapsed.value),
      notice: null,
      rollKey: thinkingLive.value ? `${lone.value.id}:live` : `${lone.value.id}:done`,
    }
  }
  if (!props.group.live) {
    return { icon: Layers, iconKey: 'stack', label: workSummary(props.group.steps), notice: null, rollKey: 'done' }
  }
  // Open, the steps show under the row, so it stands still as the stack of
  // what runs; folded again, it shows the newest step. Opening and folding
  // change what the row shows, not how far the work got, so the face cuts
  // over at once rather than rolling.
  if (isOpen.value) {
    return { icon: Layers, iconKey: 'stack', label: workRunning(props.group.steps), notice: null, rollKey: 'running' }
  }
  const step = newest.value
  const { icon, key } = stepIcon(step)
  if (isNoticeStep(step)) {
    return { icon, iconKey: key, label: '', notice: step, rollKey: stepKey(step) }
  }
  return {
    icon,
    iconKey: key,
    label: stepTitle(step),
    notice: null,
    // A call's placeholder title rolls over to its description once written;
    // the call's block keeps that face, so it does not roll again.
    rollKey: step.type === 'pending_call' && step.call.description === null ? `${stepKey(step)}:writing` : stepKey(step),
  }
})
const calls = computed(() => props.group.steps.filter((step) => step.type === 'tool_call'))
const select = useEditSelection()
const transcript = useTranscript()
// Folded, each file the run changed shows once, with its lines from before
// the run to after its latest call, as that call stored them; a file whose
// ends were not kept is named alone.
const changed = computed(() => runFiles(calls.value))
const files = computed(() => changed.value.map(changeFile))
const media = computed(() => foldedMedia(props.group.steps))

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

/**
 * A summed pill opens its file at the first of the run's calls that
 * changed it, under the name the file has in the request now.
 */
function selection(path: string) {
  const first = changed.value.find((file) => file.path === path)?.edits[0]
  const call = calls.value.at(-1)
  return transcript && first && call ? pillSelection(transcript.node, transcript.requests(), call, first.call, first.path) : null
}
function selectable(path: string): boolean {
  return select() !== undefined && selection(path) !== null
}
function pick(path: string, clicks: number): void {
  const picked = selection(path)
  if (picked)
    select()?.(picked, clicks)
}
</script>

<template>
  <BlockScope v-if="loneCall" :block="loneCall">
    <ToolCallBlock v-model:open="isOpen" :block="loneCall" :working="group.live" />
  </BlockScope>
  <div v-else-if="loneNotice" class="[--agent-pad-x:0px]">
    <CommandReportBlock v-if="loneNotice.type === 'wakeup'" :reports="loneNotice.reports" :working="group.live" />
    <BlockScope v-else :block="loneNotice">
      <AgentReceiptBlock v-model:open="isOpen" :message="loneNotice.message" :working="group.live" />
    </BlockScope>
  </div>
  <template v-else>
    <FunctionalBlock
      v-model:open="isOpen"
      :expandable="!lone || loneText !== ''"
      :flow="!lone"
      :stick-bottom="lone ? thinkingLive : false"
      :roll-key="face.rollKey"
      :icon-key="face.iconKey"
      :cut-key="isOpen && !lone ? 'open' : 'folded'"
      :loading="group.live"
    >
      <template #icon>
        <component :is="face.icon" :size="ICON_PX.in28" />
      </template>
      <NoticeStepFace v-if="face.notice" :step="face.notice" />
      <span v-else class="min-w-0 truncate">{{ face.label }}</span>
      <!-- A thinking with no text has nothing to open: no body, so no chevron. -->
      <template v-if="!lone || loneText !== ''" #body>
        <StreamedMarkdown
          v-if="lone"
          :content="lone.text"
          :streaming="thinkingLive"
          class="px-3 py-1 text-[13px] leading-4.5 text-fg-subtle [--markdown-block-gap:--spacing(1)]"
        />
        <div v-else class="[--agent-pad-x:0px]">
          <!-- Open, each step's row is named, so a block brought into view inside the run marks its own row. -->
          <div
            v-for="(step, index) in group.steps"
            :key="stepKey(step)"
            :data-step-id="isOpen ? step.id : undefined"
            v-bind="chromeEntrance(entering.has(stepKey(step)))"
          >
            <BlockScope v-if="step.type === 'thinking'" :block="step">
              <ThinkingBlock
                :thinking="step.text"
                :is-streaming="thinkingLive && index === group.steps.length - 1"
                :created-at="step.createdAt"
                :ended-at="endedAt(index)"
              />
            </BlockScope>
            <BlockScope v-else-if="step.type === 'tool_call'" :block="step">
              <ToolCallBlock :block="step" />
            </BlockScope>
            <PendingCallBlock v-else-if="step.type === 'pending_call'" :call="step.call" />
            <CommandReportBlock v-else-if="step.type === 'wakeup'" :reports="step.reports" />
            <BlockScope v-else :block="step">
              <AgentReceiptBlock :message="step.message" />
            </BlockScope>
          </div>
        </div>
      </template>
    </FunctionalBlock>
    <!-- The calls' media fold away as the group opens to show each under its own call, so none shows twice. -->
    <Fold :open="!isOpen && media.length > 0">
      <ToolMedia :media="media" />
    </Fold>
    <!-- The summed pills fold in as the first file changes, and away as the group opens to show each call's own. -->
    <Fold :open="!isOpen && files.length > 0">
      <FileChangePills
        v-if="files.length > 0"
        class="py-1"
        :style="{ paddingLeft: `${ICON_PX.in28 + 8}px` }"
        :files="files"
        :selectable="selectable"
        @select="pick"
      />
    </Fold>
  </template>
</template>

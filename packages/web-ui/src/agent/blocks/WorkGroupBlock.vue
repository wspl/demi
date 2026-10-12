<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch, type Component } from 'vue'
import { Brain, Layers } from '@lucide/vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import Fold from '@demicodes/web-ui/ui/Fold.vue'
import { CHROME_ENTER_MS, chromeEntrance } from '@demicodes/web-ui/ui/chrome-enter'
import FunctionalBlock from './FunctionalBlock.vue'
import ThinkingBlock from './ThinkingBlock.vue'
import ToolCallBlock from './ToolCallBlock.vue'
import PendingCallBlock from './PendingCallBlock.vue'
import ThinkingText from './ThinkingText.vue'
import CommandReportBlock from './CommandReportBlock.vue'
import AgentReceiptBlock from './AgentReceiptBlock.vue'
import NoticeStepFace from './NoticeStepFace.vue'
import ShellRowTag from './ShellRowTag.vue'
import CommandTimeTag from './CommandTimeTag.vue'
import FileChangePills from './FileChangePills.vue'
import GoneMediaLines from './GoneMediaLines.vue'
import { stepIcon } from './tool-row-icon'
import { toolCallTitle } from '../block-helpers'
import { thinkingFaceLabel } from '../thinking-label'
import { isNoticeStep, runFace, stepKey, workRunning, workSummary, type NoticeStep, type WorkGroupBlock, type WorkStep } from '../work-groups'
import { useEditSelection, useTranscript } from '../edit-selection'
import { changeFile, pillSelection, runFiles } from '../../files/request-changes'
import { foldedMedia, splitMedia } from '../tool-media'
import { useElapsedTime } from '../../composables/useElapsedTime'
import BlockScope from './BlockScope.vue'
import { provideBlockScope } from '../whole-blocks'

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
 * pictures the calls and notices returned stand in its line, and the files
 * the steps changed under it; open, each with its own row. Open while it
 * runs, its title says for how long once that is past 3 seconds; folded,
 * the newest step's face wears that step's tag, as a running command's time.
 */
const props = defineProps<{ group: WorkGroupBlock }>()
const isOpen = defineModel<boolean>('open', { default: false })

const newest = computed(() => props.group.steps.at(-1)!)
const shape = computed(() => runFace(props.group, isOpen.value))
const lone = computed(() => (shape.value.kind === 'thinking' ? shape.value.thinking : null))
// A lone thinking's row is the thinking's own: held light, it opens to read it whole.
provideBlockScope(() => lone.value ?? undefined)
const loneCall = computed(() => (shape.value.kind === 'call' ? shape.value.call : null))
const loneNotice = computed(() => (shape.value.kind === 'notice' ? shape.value.notice : null))
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
/** The run is open while it runs: its row stands as a stack and says what runs, and for how long. */
const stackRunning = computed(() => shape.value.kind === 'stack')
/** How long the run has lasted, from its first step's start. */
const runElapsed = useElapsedTime(
  () => {
    const first = props.group.steps.find((step) => step.type !== 'pending_call')
    return first ? Date.parse(first.createdAt) : Number.NaN
  },
  () => stackRunning.value,
)

function stepTitle(step: Exclude<WorkStep, NoticeStep>): string {
  switch (step.type) {
    case 'thinking':
      return 'Thinking'
    case 'tool_call':
      return toolCallTitle(step)
    case 'pending_call':
      return step.call.description
  }
}

interface Face {
  icon: Component
  iconKey: string
  label: string
  /** The newest step is a notice or a receipt: the face says it as its own row does. */
  notice: NoticeStep | null
  /** The newest step is a `shell` call: the face wears its row's tag. */
  command: ToolCallStep | null
  rollKey: string
}
type ToolCallStep = Extract<WorkStep, { type: 'tool_call' }>

const face = computed<Face>(() => {
  const shown = shape.value
  if (lone.value) {
    return {
      icon: Brain,
      iconKey: 'thinking',
      label: thinkingFaceLabel(thinkingLive.value, loneElapsed.value),
      notice: null,
      command: null,
      rollKey: thinkingLive.value ? `${lone.value.id}:live` : `${lone.value.id}:done`,
    }
  }
  // Open, the steps show under the row, so it stands still as the stack of
  // what runs; folded again, it shows the newest step. Opening and folding
  // change what the row shows, not how far the work got, so the face cuts
  // over at once rather than rolling.
  if (shown.kind !== 'step') {
    return shown.kind === 'stack'
      ? { icon: Layers, iconKey: 'stack', label: workRunning(props.group.steps, runElapsed.value), notice: null, command: null, rollKey: 'running' }
      : { icon: Layers, iconKey: 'stack', label: workSummary(props.group.steps), notice: null, command: null, rollKey: 'done' }
  }
  const step = shown.step
  const { icon, key } = stepIcon(step)
  if (isNoticeStep(step)) {
    return { icon, iconKey: key, label: '', notice: step, command: null, rollKey: stepKey(step) }
  }
  return {
    icon,
    iconKey: key,
    label: stepTitle(step),
    notice: null,
    command: shown.command,
    // A call being written and its block are one step by its tool-use id:
    // the block keeps the face the call showed, so it does not roll again.
    rollKey: stepKey(step),
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
const media = computed(() => splitMedia(foldedMedia(props.group.steps)))

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
function pick(path: string): void {
  const picked = selection(path)
  if (picked)
    select()?.(picked)
}
</script>

<template>
  <BlockScope v-if="loneCall" :block="loneCall">
    <ToolCallBlock v-model:open="isOpen" :block="loneCall" :working="group.live" />
  </BlockScope>
  <div v-else-if="loneNotice" class="[--agent-pad-x:0px]">
    <CommandReportBlock v-if="loneNotice.type === 'wakeup'" :reports="loneNotice.reports" :working="group.live" />
    <AgentReceiptBlock v-else :message="loneNotice.message" :working="group.live" />
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
      :pictures="isOpen ? undefined : media.pictures"
    >
      <template #icon>
        <component :is="face.icon" :size="ICON_PX.in28" />
      </template>
      <NoticeStepFace v-if="face.notice" :step="face.notice" />
      <template v-else>
        <span class="min-w-0 truncate">{{ face.label }}</span>
        <ShellRowTag v-if="face.command" :block="face.command" />
        <!-- An ended run wears the time of a command still running in it, until it ends. -->
        <CommandTimeTag v-else-if="shape.kind === 'done' && shape.running.length > 0" :calls="shape.running" />
      </template>
      <!-- A thinking with no text has nothing to open: no body, so no chevron. -->
      <template v-if="!lone || loneText !== ''" #body>
        <ThinkingText v-if="lone" :content="lone.text" :streaming="thinkingLive" />
        <div v-else class="[--agent-pad-x:0px]">
          <!-- Open, each step's row is named, so a block brought into view inside the run marks its own row;
            the row stands in the column already, and its icon gives the mark room at its start. -->
          <div
            v-for="(step, index) in group.steps"
            :key="stepKey(step)"
            class="[--found-inset-x:0px]"
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
            <AgentReceiptBlock v-else :message="step.message" />
          </div>
        </div>
      </template>
    </FunctionalBlock>
    <!-- The lines of gone media fold away as the group opens to show each under its own call, so none shows twice. -->
    <Fold :open="!isOpen && media.gone.length > 0">
      <GoneMediaLines :lines="media.gone" />
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

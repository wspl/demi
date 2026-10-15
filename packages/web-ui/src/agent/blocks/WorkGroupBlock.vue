<script setup lang="ts">
import { computed, onBeforeUnmount, ref, useId, watch, type Component } from 'vue'
import { Layers } from '@lucide/vue'
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
import StepFace from './StepFace.vue'
import CommandTimeTag from './CommandTimeTag.vue'
import FileChangePills from './FileChangePills.vue'
import GoneMediaLines from './GoneMediaLines.vue'
import { stepIcon } from './tool-row-icon'
import { runFace, stepKey, workRunning, workSummary, type WorkGroupBlock, type WorkStep } from '../work-groups'
import { useEditSelection, useFilePills, useTranscript } from '../edit-selection'
import { changeFile, pillSelection, requestFileSelection, runFiles } from '../../files/request-changes'
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
 * stack, open if the reader opened the call; a lone call being written, a
 * lone notice or a lone receipt is its own row likewise. Wherever a step
 * shows, its line is the same face (`StepFace`), the run's folded face
 * included. Opening the row shows every step as its own row,
 * a command's notice and an agent's receipt among them; a step that joins
 * while it is open enters as a row joining the transcript does. Folded, the
 * pictures the calls and notices returned stand in its line, and the files
 * the steps changed under it; open, each with its own row. Open while it
 * runs, its title says for how long once that is past 3 seconds; folded,
 * the newest step's face wears that step's tag, as a running command's time.
 * Folded while it runs, a line over that face says what the steps before the
 * newest did, as an ended run reads them, once they say something; only the
 * face under it shimmers, and a click on either line opens the run.
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
const loneWriting = computed(() => (shape.value.kind === 'writing' ? shape.value.call : null))
/** The newest step is thinking that still streams. */
const thinkingLive = computed(() => props.group.live && newest.value.type === 'thinking')

/** When the step at `index` ended: the next step's start, or the group's end. */
function endedAt(index: number): string | null {
  const next = props.group.steps[index + 1]
  return next && next.type !== 'pending_call' ? next.createdAt : props.group.endedAt
}

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

interface Face {
  icon: Component
  iconKey: string
  /** What the row says while it stands as the run: its stack and what runs or ran. */
  label: string
  /** The step whose own face the row shows: the lone thinking, or the newest step of a folded running run. */
  step: WorkStep | null
  rollKey: string
}

const face = computed<Face>(() => {
  const shown = shape.value
  if (lone.value) {
    return {
      icon: stepIcon(lone.value).icon,
      iconKey: 'thinking',
      label: '',
      step: lone.value,
      rollKey: thinkingLive.value ? `${lone.value.id}:live` : `${lone.value.id}:done`,
    }
  }
  // Open, the steps show under the row, so it stands still as the stack of
  // what runs; folded again, it shows the newest step by the step's own
  // face. Opening and folding change what the row shows, not how far the
  // work got, so the face cuts over at once rather than rolling.
  if (shown.kind !== 'step') {
    return shown.kind === 'stack'
      ? { icon: Layers, iconKey: 'stack', label: workRunning(props.group.steps, runElapsed.value), step: null, rollKey: 'running' }
      : { icon: Layers, iconKey: 'stack', label: workSummary(props.group.steps), step: null, rollKey: 'done' }
  }
  const { icon, key } = stepIcon(shown.step)
  // A call being written and its block are one step by its tool-use id:
  // the block keeps the face the call showed, so it does not roll again.
  return { icon, iconKey: key, label: '', step: shown.step, rollKey: stepKey(shown.step) }
})

/** What the steps before the newest did, while the folded run holds a step that says it; empty otherwise. */
const earlier = computed(() => (shape.value.kind === 'step' ? shape.value.earlier : ''))
// The line folds away as the run ends or opens; its words stay while it does.
const earlierShown = ref(earlier.value)
watch(earlier, (now) => {
  if (now !== '')
    earlierShown.value = now
})
const earlierId = useId()
function toggle(): void {
  isOpen.value = !isOpen.value
}
const calls = computed(() => props.group.steps.filter((step) => step.type === 'tool_call'))
const select = useEditSelection()
const transcript = useTranscript()
const pillsShown = useFilePills()
// Folded, each file the run changed shows once, with its lines from before
// the run to after its latest call, as that call stored them; a file whose
// ends were not kept is named alone. A conversation outside a project shows
// none.
const changed = computed(() => runFiles(calls.value))
const files = computed(() => pillsShown() ? changed.value.map(changeFile) : [])
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
  return transcript && first && call
    ? pillSelection(transcript.node, transcript.requests(), call, first.call, first.path, transcript.directory(call.id))
    : null
}
/** What the "+N Files" pill opens: the first of its files the request lists. */
function restSelection(paths: readonly string[]) {
  const call = calls.value.at(-1)
  if (!transcript || !call)
    return null
  const directory = transcript.directory(call.id)
  for (const path of paths) {
    const found = selection(path)
      ?? requestFileSelection(transcript.node, transcript.requests(), call, path, directory)
    if (found)
      return found
  }
  return null
}
function selectable(path: string): boolean {
  return select() !== undefined && selection(path) !== null
}
function pick(path: string): void {
  const picked = selection(path)
  if (picked)
    select()?.(picked)
}
function pickRest(paths: readonly string[]): void {
  const picked = restSelection(paths)
  if (picked)
    select()?.(picked)
}
</script>

<template>
  <BlockScope v-if="loneCall" :block="loneCall">
    <ToolCallBlock v-model:open="isOpen" :block="loneCall" :working="group.live" />
  </BlockScope>
  <PendingCallBlock v-else-if="loneWriting" :step="loneWriting" />
  <div v-else-if="loneNotice" class="[--agent-pad-x:0px]">
    <CommandReportBlock v-if="loneNotice.type === 'wakeup'" :reports="loneNotice.reports" :working="group.live" />
    <AgentReceiptBlock v-else :message="loneNotice.message" :working="group.live" />
  </div>
  <template v-else>
    <!-- The steps before the newest, done: no shimmer. Like the face under it, the line is a button's face, so the keyboard opens the run from either. -->
    <Fold :open="earlier !== ''">
      <div
        class="group relative flex h-7 cursor-default select-none items-center gap-2 text-row text-fg-row transition-colors duration-200 ease-out hover:text-fg-muted"
        data-run-earlier
        @click="toggle"
      >
        <button
          type="button"
          class="absolute inset-0 cursor-default rounded-sm focus-visible:pointer-events-none focus-visible:z-10"
          :aria-expanded="isOpen"
          :aria-labelledby="earlierId"
        ></button>
        <div class="relative flex min-w-0 items-center gap-2">
          <div class="flex shrink-0 items-center justify-center" :style="{ width: `${ICON_PX.in28}px`, height: `${ICON_PX.in28}px` }">
            <Layers :size="ICON_PX.in28" />
          </div>
          <span :id="earlierId" class="min-w-0 truncate">{{ earlierShown }}</span>
        </div>
      </div>
    </Fold>
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
      <StepFace
        v-if="face.step"
        :step="face.step"
        :streaming="thinkingLive"
        :ended-at="group.endedAt"
      />
      <template v-else>
        <span class="min-w-0 truncate">{{ face.label }}</span>
        <!-- An ended run wears the time of a command still running in it, until it ends. -->
        <CommandTimeTag v-if="shape.kind === 'done' && shape.running.length > 0" :calls="shape.running" />
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
            <PendingCallBlock v-else-if="step.type === 'pending_call'" :step="step" />
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
        :rest-selectable="(paths) => select() !== undefined && restSelection(paths) !== null"
        @select="pick"
        @open-rest="pickRest"
      />
    </Fold>
  </template>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { Layers } from '@lucide/vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import Fold from '@demicodes/web-ui/ui/Fold.vue'
import { CHROME_ENTER_MS, chromeEntrance } from '@demicodes/web-ui/ui/chrome-enter'
import FunctionalBlock from './FunctionalBlock.vue'
import StepRow from './StepRow.vue'
import CommandTimeTag from './CommandTimeTag.vue'
import FileChangePills from './FileChangePills.vue'
import GoneMediaLines from './GoneMediaLines.vue'
import { runLabel, runningOn, stepKey, type WorkGroupBlock } from '../work-groups'
import { useEditSelection, useFilePills, useTranscript } from '../edit-selection'
import { changeFile, pillSelection, requestFileSelection, runFiles } from '../../files/request-changes'
import { foldedMedia, splitMedia } from '../tool-media'

/**
 * The steps of a run that have ended, as one row; a step that still runs is
 * a row of its own under it. A lone step is that step's own row, which opens
 * to the step; more read what they did, done, no shimmer, and open to every
 * step, each as its own row. A step that joins never changes whether the row
 * is open: one that joins while it is open enters as a row joining the
 * transcript does, and a lone step's row that a step joins turns into the
 * stack, open if the reader opened the step. Folded, the row wears the
 * pictures its steps returned, and the files they changed show under it;
 * open, each step its own.
 */
const props = defineProps<{ group: WorkGroupBlock }>()
/** The row is open: the transcript's (`useOpenRuns`). */
const isOpen = defineModel<boolean>('open', { default: false })

const lone = computed(() => (props.group.steps.length === 1 ? props.group.steps[0]! : null))
/** What the run did and the time its steps took. */
const label = computed(() => runLabel(props.group))
/** The calls whose command ran on after they returned: the row wears their time until they end. */
const running = computed(() => runningOn(props.group.steps))

const calls = computed(() => props.group.steps.filter((step) => step.type === 'tool_call'))
const select = useEditSelection()
const transcript = useTranscript()
const pillsShown = useFilePills()
// Folded, each file the run changed shows once, with its
// lines from before the run to after its latest call, as that call stored
// them; a file whose ends were not kept is named alone. A conversation
// outside a project shows none.
const changed = computed(() => runFiles(calls.value))
const files = computed(() => pillsShown() ? changed.value.map(changeFile) : [])
const media = computed(() => splitMedia(foldedMedia(props.group.steps)))

// A step enters once, as it joins, the way a row joining the transcript
// does: one that joins while the row is open slides in; opening the row
// shows the steps that were there, still.
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
  <!-- A lone step is its own row, in the list's column already. -->
  <div v-if="lone" class="[--agent-pad-x:0px]">
    <StepRow v-model:open="isOpen" :step="lone" :ended-at="group.ends[0]" />
  </div>
  <template v-else>
    <FunctionalBlock
      v-model:open="isOpen"
      expandable
      flow
      :pictures="isOpen ? undefined : media.pictures"
    >
      <template #icon>
        <Layers :size="ICON_PX.in28" />
      </template>
      <span class="min-w-0 truncate">{{ label }}</span>
      <!-- The run wears the time of a command still running in it, until it ends. -->
      <CommandTimeTag v-if="running.length > 0" :calls="running" />
      <template #body>
        <div class="[--agent-pad-x:0px]">
          <!-- Open, each step's row is named, so a block brought into view inside the run marks its own row;
            the row stands in the column already, and its icon gives the mark room at its start. -->
          <div
            v-for="(step, index) in group.steps"
            :key="stepKey(step)"
            class="[--found-inset-x:0px]"
            :data-step-id="isOpen ? step.id : undefined"
            v-bind="chromeEntrance(entering.has(stepKey(step)))"
          >
            <StepRow :step="step" :ended-at="group.ends[index]" />
          </div>
        </div>
      </template>
    </FunctionalBlock>
    <!-- The lines of gone media fold away as the row opens to show each under its own call, so none shows twice. -->
    <Fold :open="!isOpen && media.gone.length > 0">
      <GoneMediaLines :lines="media.gone" />
    </Fold>
    <!-- The summed pills fold in as the first file changes, and away as the row opens to show each call's own. -->
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

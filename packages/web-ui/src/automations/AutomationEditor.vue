<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { BellOff, ChevronDown, ChevronLeft, ChevronRight, Ellipsis, Eye, EyeOff, Play, Plus, TextCursorInput, Trash2, TriangleAlert } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import ModelMenu from '../agent/ModelMenu.vue'
import { applyModelChange, initialModelSettings, type ModelSettingsChange } from '../agent/model-selection'
import ModelChoiceTrigger from '../settings/ModelChoiceTrigger.vue'
import { modelChoiceLabel } from '../settings/model-choice'
import SettingsGroup from '../settings/SettingsGroup.vue'
import SettingsRow from '../settings/SettingsRow.vue'
import Button from '../ui/Button.vue'
import CommitTextInput from '../ui/CommitTextInput.vue'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import CopyCode from '../ui/CopyCode.vue'
import Dropdown from '../ui/Dropdown.vue'
import FilterDialog from '../ui/FilterDialog.vue'
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
import MenuDivider from '../ui/MenuDivider.vue'
import MenuItem from '../ui/MenuItem.vue'
import Segmented from '../ui/Segmented.vue'
import Switch from '../ui/Switch.vue'
import Tag from '../ui/Tag.vue'
import TextArea from '../ui/TextArea.vue'
import TitleInput from '../ui/TitleInput.vue'
import Tooltip from '../ui/Tooltip.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { hostGlyph } from '../devices/device-glyphs'
import { t as shared } from '../locales/text'
import AutomationRepeatDialog from './AutomationRepeatDialog.vue'
import AutomationTaskDialog from './AutomationTaskDialog.vue'
import { t } from './locales/text'
import { hasMessage } from './types'
import type {
  Automation,
  AutomationChange,
  AutomationConversation,
  AutomationHost,
  AutomationRun,
  AutomationTask,
  AutomationTrigger,
  HolidayCountry,
  IfBusy,
  Repeat,
  RunState,
  TimeSchedule,
} from './types'
import {
  REPEAT_PRESETS,
  fieldMoment,
  intervalOf,
  intervalWords,
  minutesOf,
  pausedWords,
  repeatPreset,
  repeatWords,
  runLine,
  shortMoment,
  taskWords,
  zonedMoment,
} from './words'

/**
 * One automation, every part editable in place as a provider's page is, each
 * choice one menu as Calendar's and Reminders' are, with no explanation under
 * a row:
 *
 * - Its name, Run Now and On or Paused, in its header; Run Now runs a
 *   repeat's only task, or offers its tasks when it has several.
 * - When it runs, one menu of the repeats, the single triggers and Custom…,
 *   whose sheet (AutomationRepeatDialog) sets any cycle; then a repeat's
 *   tasks, each opening its sheet (AutomationTaskDialog), or a single
 *   trigger's own fields and its message; and the time zone.
 * - Its conversation, one menu of a new one each run, the recent ones and
 *   Choose…; then the Device and Project of a new one, or what the message
 *   does while an existing one is busy; and the model.
 * - Its runs, newest first, each opening its conversation.
 * - Delete, which asks first.
 *
 * Every edit is one `change` of the parts it changes, which the host saves.
 */
const props = defineProps<{
  automation: Automation
  hosts: readonly AutomationHost[]
  /** The conversations an existing-conversation target can name, with their Hosts, the most recent first. */
  conversations: readonly AutomationConversation[]
  /** The time zones to choose from, the user's own first. */
  timeZones: readonly string[]
  /** The countries whose holidays a task's working days can skip. */
  holidayCountries: readonly HolidayCountry[]
  providers: ProviderInfo[]
  /** The models the menu offers, by provider. */
  models: Record<string, ModelInfo[]>
  overlayStore: OverlayStore
  /** A URL trigger's secret while Show shows it; none while hidden. */
  secret?: string | null
  /** No browser of the user has notifications on: what runs notify reaches nobody. */
  notificationsOff?: boolean
}>()

const emit = defineEmits<{
  change: [change: AutomationChange]
  back: []
  /** Run Now: the task it runs, a repeat's; none for a single trigger. */
  runNow: [taskId: string | null]
  delete: []
  showSecret: []
  hideSecret: []
  regenerateSecret: []
  openRun: [run: AutomationRun]
  turnOnNotifications: []
}>()

const renaming = ref(false)
const deleteAsked = ref(false)
const regenerateAsked = ref(false)
/** The task whose sheet is open. */
const openTaskId = ref<string | null>(null)
const customOpen = ref(false)
const choosing = ref(false)
const query = ref('')
watch(() => props.automation.id, () => {
  renaming.value = false
  openTaskId.value = null
  customOpen.value = false
  choosing.value = false
})

function rename(name: string): void {
  renaming.value = false
  const trimmed = name.trim()
  if (trimmed && trimmed !== props.automation.name) {
    emit('change', { name: trimmed })
  }
}

// When it runs.

/** A repeat's tasks; none for a single trigger. */
const tasks = computed<AutomationTask[] | null>(() => {
  const trigger = props.automation.trigger
  return trigger.kind === 'time' && trigger.schedule.kind === 'repeat' ? trigger.schedule.tasks : null
})
const repeat = computed<Repeat | null>(() => {
  const trigger = props.automation.trigger
  return trigger.kind === 'time' && trigger.schedule.kind === 'repeat' ? trigger.schedule.repeat : null
})
/** A single time trigger's schedule; none for a repeat or another trigger. */
const single = computed(() => {
  const trigger = props.automation.trigger
  return trigger.kind === 'time' && trigger.schedule.kind !== 'repeat' ? trigger.schedule : null
})

/** The message runs send: a single trigger's prompt, or a repeat's first task's. */
function message(): string {
  return tasks.value?.[0]?.prompt ?? props.automation.prompt
}

/** A task at the cycle's first moment, Monday, the 1st, in January, at 09:00. */
function newTask(prompt: string): AutomationTask {
  return { id: crypto.randomUUID(), time: '09:00', days: ['mon'], day: { kind: 'date', date: 1 }, month: 1, holidays: null, prompt }
}

/** Today by the automation's clocks, as 2026-10-20. */
function today(): string {
  return fieldMoment(new Date().toISOString(), props.automation.timeZone).slice(0, 10)
}

/**
 * A change to `trigger`, with the message moved to where the new trigger
 * keeps it: a repeat's one task takes the prompt, and a single trigger takes
 * the first task's message.
 */
function changeTrigger(trigger: AutomationTrigger): void {
  const repeats = trigger.kind === 'time' && trigger.schedule.kind === 'repeat'
  emit('change', { trigger, prompt: repeats ? '' : message() })
}

function setSchedule(schedule: TimeSchedule): void {
  if (props.automation.trigger.kind === 'time') {
    emit('change', { trigger: { kind: 'time', schedule } })
  }
}

/** The repeat `next`, keeping the tasks of a repeat or making one from a single trigger's message. */
function setRepeat(next: Repeat): void {
  if (tasks.value) {
    setSchedule({ kind: 'repeat', repeat: next, tasks: tasks.value })
  } else {
    changeTrigger({ kind: 'time', schedule: { kind: 'repeat', repeat: next, tasks: [newTask(message())] } })
  }
}

type WhenChoice = 'once' | (typeof REPEAT_PRESETS)[number]['key'] | 'every' | 'cron' | 'change' | 'url' | 'custom'

const when = computed<WhenChoice>(() => {
  const trigger = props.automation.trigger
  if (trigger.kind !== 'time') {
    return trigger.kind
  }
  const schedule = trigger.schedule
  return schedule.kind === 'repeat' ? repeatPreset(schedule.repeat) ?? 'custom' : schedule.kind
})
/** The menu's face: its choice, or what a choice that asks for more is set to. */
const whenLabel = computed(() => {
  const choice = when.value
  if (choice === 'custom') {
    return repeat.value ? repeatWords(repeat.value) : ''
  }
  if (choice === 'every') {
    return single.value?.kind === 'every' ? intervalWords(single.value.minutes) : ''
  }
  if (choice === 'once' || choice === 'cron' || choice === 'change' || choice === 'url') {
    return t(`when.${choice}`)
  }
  return t(`when.presets.${choice}`)
})

/** The single time trigger of `kind`, at a default moment. */
function scheduleOf(kind: 'once' | 'every' | 'cron'): TimeSchedule {
  switch (kind) {
    case 'once': {
      // Tomorrow 08:00 by the automation's clocks.
      const [year, month, day] = today().split('-').map(Number) as [number, number, number]
      const tomorrow = new Date(Date.UTC(year, month - 1, day + 1))
      const at = zonedMoment(tomorrow.getUTCFullYear(), tomorrow.getUTCMonth() + 1, tomorrow.getUTCDate(), 8, 0, props.automation.timeZone)
      return { kind: 'once', at: at.toISOString() }
    }
    case 'every':
      return { kind: 'every', minutes: 120 }
    case 'cron':
      return { kind: 'cron', expression: '0 9 * * 1-5' }
  }
}

/** A choice of the When menu; Custom… opens its sheet instead. */
function choose(choice: WhenChoice): void {
  if (choice === when.value && choice !== 'custom') {
    return
  }
  switch (choice) {
    case 'custom':
      customOpen.value = true
      return
    case 'once':
    case 'every':
    case 'cron':
      changeTrigger({ kind: 'time', schedule: scheduleOf(choice) })
      return
    case 'change': {
      const minutes = single.value?.kind === 'every' ? single.value.minutes : 10
      changeTrigger({ kind: 'change', command: 'date +%F', everyMinutes: minutes })
      return
    }
    case 'url':
      changeTrigger({ kind: 'url' })
      return
    default: {
      const preset = REPEAT_PRESETS.find((each) => each.key === choice)!
      const start = preset.every > 1 ? repeat.value?.start ?? today() : null
      setRepeat({ every: preset.every, unit: preset.unit, start })
    }
  }
}

/** The span between checks or runs, in minutes; none for a trigger without one. */
const minutes = computed<number | null>(() => {
  const trigger = props.automation.trigger
  if (trigger.kind === 'change') {
    return trigger.everyMinutes
  }
  return single.value?.kind === 'every' ? single.value.minutes : null
})
const interval = computed(() => (minutes.value === null ? null : intervalOf(minutes.value)))
const units = computed(() => (['minutes', 'hours', 'days'] as const).map((value) => ({ value, label: t(`trigger.units.${value}`) })))

function setMinutes(next: number): void {
  const trigger = props.automation.trigger
  if (trigger.kind === 'change') {
    emit('change', { trigger: { ...trigger, everyMinutes: next } })
  } else if (single.value?.kind === 'every') {
    setSchedule({ kind: 'every', minutes: next })
  }
}

const MOMENT = /^(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2})$/

function validCount(value: string): string | null {
  return /^[1-9]\d*$/.test(value.trim()) ? null : t('trigger.intervalInvalid')
}
function validMoment(value: string): string | null {
  return MOMENT.test(value.trim()) ? null : t('trigger.momentInvalid')
}
function validCron(value: string): string | null {
  return value.trim().split(/\s+/).length === 5 ? null : t('trigger.expressionInvalid')
}
function validLimit(value: string): string | null {
  return value.trim() === '' || /^[1-9]\d*$/.test(value.trim()) ? null : t('trigger.limitInvalid')
}
/** What the field last edited found wrong, under its row until it is right. */
const problem = ref<string | null>(null)

function setOnce(value: string): void {
  const match = MOMENT.exec(value.trim())
  if (match) {
    const [, year, month, day, hour, minute] = match.map(Number) as [number, number, number, number, number, number]
    // Taken as the automation's clocks read it, not the browser's.
    setSchedule({ kind: 'once', at: zonedMoment(year, month, day, hour, minute, props.automation.timeZone).toISOString() })
  }
}

function setCommand(command: string): void {
  const trigger = props.automation.trigger
  if (trigger.kind === 'change' && command.trim()) {
    emit('change', { trigger: { ...trigger, command } })
  }
}

function setLimit(value: string): void {
  emit('change', { maxRunsPerDay: value.trim() === '' ? null : Number(value.trim()) })
}

/** A time trigger's times and a daily limit's midnight are read in the time zone. */
const zoned = computed(() => props.automation.trigger.kind === 'time' || props.automation.maxRunsPerDay !== null)

// The tasks.

const openTask = computed(() => tasks.value?.find((each) => each.id === openTaskId.value) ?? null)

function setTasks(next: AutomationTask[]): void {
  if (repeat.value) {
    setSchedule({ kind: 'repeat', repeat: repeat.value, tasks: next })
  }
}

/** Add Task: one at the cycle's first moment with no message, opened. */
function addTask(): void {
  if (tasks.value) {
    const task = newTask('')
    setTasks([...tasks.value, task])
    openTaskId.value = task.id
  }
}
function changeTask(id: string, change: Partial<Omit<AutomationTask, 'id'>>): void {
  if (tasks.value) {
    setTasks(tasks.value.map((each) => (each.id === id ? { ...each, ...change } : each)))
  }
}
function deleteTask(id: string): void {
  if (tasks.value && tasks.value.length > 1) {
    setTasks(tasks.value.filter((each) => each.id !== id))
  }
}

// A single trigger's message, saved when the field is left.

const prompt = ref(props.automation.prompt)
watch(() => props.automation.prompt, (value) => {
  prompt.value = value
})
function commitPrompt(): void {
  if (prompt.value !== props.automation.prompt) {
    emit('change', { prompt: prompt.value })
  }
}

// The conversation.

/** The recent conversations the menu offers; the rest are found with Choose…. */
const RECENT = 5
const existing = computed(() => {
  const target = props.automation.target
  return target.kind === 'existing' ? target : null
})
const conversationTitle = computed(() => existing.value
  ? props.conversations.find((each) => each.id === existing.value?.conversationId)?.title ?? t('words.existingGone')
  : t('target.newEach'))
const recent = computed(() => {
  const first = props.conversations.slice(0, RECENT)
  const chosen = props.conversations.find((each) => each.id === existing.value?.conversationId)
  return chosen && !first.includes(chosen) ? [...first, chosen] : first
})
const found = computed(() => {
  const words = query.value.trim().toLowerCase()
  return props.conversations.filter((each) => each.title.toLowerCase().includes(words))
})
function hostName(id: string): string {
  return props.hosts.find((each) => each.id === id)?.name ?? ''
}

const targetHostId = computed(() => {
  const target = props.automation.target
  return target.kind === 'new' ? target.hostId : props.conversations.find((each) => each.id === target.conversationId)?.hostId
})
const host = computed(() => props.hosts.find((candidate) => candidate.id === targetHostId.value))
const project = computed(() => {
  const target = props.automation.target
  return target.kind === 'new' ? host.value?.projects.find((candidate) => candidate.id === target.projectId) : undefined
})

/** New Each Run: a new conversation on the Host the conversation was on, in no project. */
function chooseNew(): void {
  if (existing.value) {
    emit('change', { target: { kind: 'new', hostId: targetHostId.value ?? 'cloud', projectId: null } })
  }
}
/** An existing conversation, keeping what the message does while it is busy. */
function chooseConversation(id: string): void {
  choosing.value = false
  if (id !== existing.value?.conversationId) {
    emit('change', { target: { kind: 'existing', conversationId: id, ifBusy: existing.value?.ifBusy ?? 'queue' } })
  }
}

/** Another Host has other projects: the new conversation starts there in none. */
function chooseHost(id: string): void {
  if (id !== targetHostId.value) {
    emit('change', { target: { kind: 'new', hostId: id, projectId: null } })
  }
}

/** What a run's message does while the existing conversation is busy, in the composer's words. */
const busyWays = computed(() => (['queue', 'steer'] as const).map((value) => ({ value, label: shared(`sendWay.${value}`) })))
function setIfBusy(ifBusy: IfBusy): void {
  if (existing.value && existing.value.ifBusy !== ifBusy) {
    emit('change', { target: { ...existing.value, ifBusy } })
  }
}

// The model.

const modelLabel = computed(() => props.automation.model
  ? modelChoiceLabel(props.automation.model, props.providers, props.models)
  : { name: t('model.choose'), effort: '', fast: false })
/** A new conversation starts with the automation's model: without one its runs are skipped, which the row says. */
const modelMissing = computed(() =>
  props.automation.target.kind === 'new' && props.automation.model === null ? t('model.missing') : null)

function changeModel(change: ModelSettingsChange): void {
  emit('change', { model: applyModelChange(props.automation.model ?? initialModelSettings(), change) })
}

// The runs.

const STATE_TONE: Record<RunState, 'neutral' | 'accent' | 'success' | 'warning' | 'danger'> = {
  running: 'accent',
  done: 'neutral',
  notified: 'success',
  skipped: 'warning',
  failed: 'danger',
}

/**
 * A run's line, with where its conversation went once it left the
 * automation; among several tasks, the message of the task that started it
 * comes first.
 */
function runText(run: AutomationRun): string {
  const moved = props.automation.target.kind === 'new' && run.conversationId !== null && !run.filed
  const task = tasks.value && tasks.value.length > 1 ? tasks.value.find((each) => each.id === run.taskId) : undefined
  return [task?.prompt, runLine(run), moved ? t('runs.moved') : ''].filter(Boolean).join(' · ')
}
</script>

<template>
  <div class="automation-editor mx-auto flex w-full max-w-2xl flex-col gap-8">
    <!-- The page names the automation and holds what acts on it as a whole, as a provider's header does. -->
    <header class="select-none">
      <div class="-ml-2 mb-2 phone:hidden">
        <Button variant="ghost" size="sm" @click="emit('back')">
          <ChevronLeft :size="ICON_PX.in24" />
          {{ t('page.title') }}
        </Button>
      </div>
      <div class="flex min-h-8 flex-wrap items-center gap-x-1.5 gap-y-2">
        <TitleInput
          v-if="renaming"
          class="w-80 max-w-full"
          :label="t('editor.name')"
          :title="automation.name"
          @submit="rename"
          @cancel="renaming = false"
        />
        <template v-else>
          <h2
            class="min-w-0 text-[20px] font-medium leading-7 text-fg-emphasis"
            @dblclick="renaming = true"
          >
            <TruncatedText :text="automation.name" />
          </h2>
          <Tooltip :content="t('editor.rename')">
            <IconButton
              size="sm"
              variant="ghost"
              :icon="TextCursorInput"
              :aria-label="t('editor.rename')"
              @click="renaming = true"
            />
          </Tooltip>
        </template>
        <div class="ml-auto flex items-center gap-1.5">
          <!-- Among several tasks, Run Now asks which, as a pull-down button does. -->
          <Dropdown v-if="tasks && tasks.length > 1" :overlay-store="overlayStore">
            <template #trigger="{ isOpen: menuOpen }">
              <Button size="sm" :pressed="menuOpen">
                <Play :size="ICON_PX.in24" /> {{ t('editor.runNow') }} <ChevronDown :size="ICON_PX.in24" />
              </Button>
            </template>
            <template #content="{ close }">
              <Menu>
                <MenuItem
                  v-for="task in tasks"
                  :key="task.id"
                  :label="taskWords(task, repeat?.unit ?? 'days')"
                  :detail="task.prompt || undefined"
                  :disabled="!hasMessage(task)"
                  :disabled-reason="t('tasks.messageMissing')"
                  @select="emit('runNow', task.id); close()"
                />
              </Menu>
            </template>
          </Dropdown>
          <Button
            v-else
            size="sm"
            :disabled="tasks?.[0] ? !hasMessage(tasks[0]) : false"
            :disabled-reason="t('tasks.messageMissing')"
            @click="emit('runNow', tasks?.[0]?.id ?? null)"
          >
            <Play :size="ICON_PX.in24" /> {{ t('editor.runNow') }}
          </Button>
          <Switch
            :model-value="!automation.paused"
            :aria-label="t('editor.on')"
            size="sm"
            class="ml-2"
            @update:model-value="emit('change', { paused: !$event })"
          />
        </div>
      </div>
      <p v-if="automation.paused" class="mt-1 text-[13px] leading-5 text-fg-muted">
        {{ pausedWords(automation.pausedBecause) }}
      </p>
      <p v-else-if="automation.nextAt" class="mt-1 text-[13px] leading-5 text-fg-muted">
        {{ t('editor.next', { when: shortMoment(automation.nextAt, automation.timeZone) }) }}
      </p>
    </header>

    <!-- Runs tell the user what they find by notification: while every browser has them off, the page says so. -->
    <SettingsGroup v-if="notificationsOff">
      <SettingsRow :label="t('notify.off')">
        <template #leading><BellOff :size="ICON_PX.in32" /></template>
        <Button size="sm" variant="primary" @click="emit('turnOnNotifications')">{{ t('notify.turnOn') }}</Button>
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup>
      <SettingsRow :label="t('when.label')" :description="problem ?? undefined">
        <Dropdown :overlay-store="overlayStore" variant="field" size="sm" width="shrink" :trigger-label="t('when.label')">
          <template #trigger>{{ whenLabel }}</template>
          <template #content="{ close }">
            <Menu>
              <MenuItem :label="t('when.once')" choice :is-selected="when === 'once'" @select="choose('once'); close()" />
              <MenuItem
                v-for="preset in REPEAT_PRESETS"
                :key="preset.key"
                :label="t(`when.presets.${preset.key}`)"
                choice
                :is-selected="when === preset.key"
                @select="choose(preset.key); close()"
              />
              <MenuDivider />
              <MenuItem :label="t('when.everyMenu')" choice :is-selected="when === 'every'" @select="choose('every'); close()" />
              <MenuItem :label="t('when.cronMenu')" choice :is-selected="when === 'cron'" @select="choose('cron'); close()" />
              <MenuItem :label="t('when.change')" choice :is-selected="when === 'change'" @select="choose('change'); close()" />
              <MenuItem :label="t('when.url')" choice :is-selected="when === 'url'" @select="choose('url'); close()" />
              <MenuDivider />
              <MenuItem :label="t('when.customMenu')" choice :is-selected="when === 'custom'" @select="choose('custom'); close()" />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>

      <SettingsRow v-if="repeat && tasks" :label="t('tasks.title')">
        <template #below>
          <!-- The rows share one grid, so every message starts where the longest moment ends. -->
          <div class="grid grid-cols-[max-content_minmax(0,1fr)_auto] gap-y-1 phone:grid-cols-[minmax(0,1fr)_auto]">
            <!-- A task's row opens its sheet; its ··· holds Run Now and Delete. -->
            <div
              v-for="task in tasks"
              :key="task.id"
              class="-mx-2 col-span-full grid grid-cols-subgrid items-center rounded-md pr-1 hover:bg-hover"
            >
              <button
                type="button"
                class="col-span-2 grid cursor-default grid-cols-subgrid items-center gap-x-4 px-2 py-1.5 text-left text-setting-small focus-visible:outline-none phone:col-span-1 phone:grid-cols-1"
                :aria-label="taskWords(task, repeat.unit)"
                @click="openTaskId = task.id"
              >
                <span class="whitespace-nowrap tabular-nums text-fg-body">{{ taskWords(task, repeat.unit) }}</span>
                <span v-if="hasMessage(task)" class="min-w-0 truncate text-fg-muted"><TruncatedText :text="task.prompt" /></span>
                <span v-else class="flex items-center gap-1.5 text-fg-muted">
                  <TriangleAlert :size="ICON_PX.in24" class="shrink-0 text-on-warning" />
                  {{ t('tasks.noMessage') }}
                </span>
              </button>
              <Dropdown :overlay-store="overlayStore">
                <template #trigger="{ isOpen: menuOpen }">
                  <Tooltip :content="t('tasks.actions')">
                    <IconButton :icon="Ellipsis" size="sm" variant="ghost" :aria-label="t('tasks.actions')" :pressed="menuOpen" />
                  </Tooltip>
                </template>
                <template #content="{ close }">
                  <Menu>
                    <MenuItem
                      :icon="Play"
                      :label="t('tasks.runNow')"
                      :disabled="!hasMessage(task)"
                      :disabled-reason="t('tasks.messageMissing')"
                      @select="emit('runNow', task.id); close()"
                    />
                    <MenuDivider />
                    <MenuItem
                      :icon="Trash2"
                      :label="t('tasks.delete')"
                      is-danger
                      :disabled="tasks.length === 1"
                      :disabled-reason="t('tasks.lastOne')"
                      @select="deleteTask(task.id); close()"
                    />
                  </Menu>
                </template>
              </Dropdown>
            </div>
            <div class="-ml-2 col-span-full">
              <Button variant="ghost" size="sm" @click="addTask">
                <Plus :size="ICON_PX.in24" /> {{ t('tasks.add') }}
              </Button>
            </div>
          </div>
        </template>
      </SettingsRow>

      <!-- A single trigger's own fields, then its message, in place of the tasks. -->
      <SettingsRow v-if="single?.kind === 'once'" :label="t('trigger.moment')">
        <CommitTextInput
          class="w-44"
          literal
          :model-value="fieldMoment(single.at, automation.timeZone)"
          :validate="validMoment"
          :aria-label="t('trigger.moment')"
          @problem="problem = $event"
          @commit="setOnce"
        />
      </SettingsRow>
      <SettingsRow v-if="single?.kind === 'cron'" :label="t('trigger.expression')">
        <CommitTextInput
          class="w-44"
          literal
          :model-value="single.expression"
          :validate="validCron"
          :aria-label="t('trigger.expression')"
          @problem="problem = $event"
          @commit="setSchedule({ kind: 'cron', expression: $event.trim().split(/\s+/).join(' ') })"
        />
      </SettingsRow>
      <SettingsRow v-if="automation.trigger.kind === 'change'" :label="t('trigger.command')">
        <template #below>
          <CommitTextInput
            literal
            class="w-full font-mono"
            :model-value="automation.trigger.command"
            placeholder="gh pr list --json number,title"
            :aria-label="t('trigger.command')"
            @commit="setCommand"
          />
        </template>
      </SettingsRow>
      <SettingsRow v-if="interval" :label="automation.trigger.kind === 'change' ? t('trigger.checkEvery') : t('trigger.interval')">
        <div class="flex items-center gap-2">
          <CommitTextInput
            class="w-16"
            literal
            :model-value="String(interval.count)"
            :validate="validCount"
            :aria-label="t('trigger.interval')"
            @problem="problem = $event"
            @commit="interval && setMinutes(minutesOf(Number($event), interval.unit))"
          />
          <Segmented
            size="sm"
            :model-value="interval.unit"
            :options="units"
            @update:model-value="interval && setMinutes(minutesOf(interval.count, $event))"
          />
        </div>
      </SettingsRow>
      <template v-if="automation.trigger.kind === 'url'">
        <SettingsRow :label="t('trigger.address')">
          <template #below>
            <CopyCode :code="automation.address ?? ''" />
          </template>
        </SettingsRow>
        <!-- The secret stays readable, hidden until Show, as a credential's value is. -->
        <SettingsRow :label="t('trigger.secret')">
          <template #below>
            <div class="flex items-center gap-1.5">
              <CopyCode v-if="secret" class="min-w-0 flex-1" :code="secret" />
              <div
                v-else
                class="min-w-0 flex-1 select-none rounded-md border border-line bg-surface px-3 py-2 font-mono text-[12px] leading-5 text-fg-subtle"
                :aria-label="t('trigger.secretHidden')"
              >••••••••••••••••••••••••••••••••</div>
              <Tooltip :content="secret ? t('trigger.hide') : t('trigger.show')">
                <IconButton
                  variant="ghost"
                  :icon="secret ? EyeOff : Eye"
                  :aria-label="secret ? t('trigger.hide') : t('trigger.show')"
                  @click="secret ? emit('hideSecret') : emit('showSecret')"
                />
              </Tooltip>
            </div>
          </template>
          <Button size="sm" @click="regenerateAsked = true">{{ t('trigger.regenerate') }}</Button>
        </SettingsRow>
        <SettingsRow :label="t('trigger.stripe')">
          <CommitTextInput
            model-value=""
            masked="secret"
            trim
            :stored="automation.stripeSigning"
            :placeholder="t('trigger.stripePaste')"
            :aria-label="t('trigger.stripe')"
            class="w-60 max-w-full"
            @commit="emit('change', { stripeSecret: $event === '' ? null : $event })"
          />
        </SettingsRow>
      </template>
      <SettingsRow v-if="automation.trigger.kind !== 'time'" :label="t('trigger.limit')">
        <CommitTextInput
          class="w-20"
          literal
          :model-value="automation.maxRunsPerDay === null ? '' : String(automation.maxRunsPerDay)"
          :placeholder="t('trigger.noLimit')"
          :validate="validLimit"
          :aria-label="t('trigger.limit')"
          @problem="problem = $event"
          @commit="setLimit"
        />
      </SettingsRow>
      <SettingsRow v-if="!tasks" :label="t('tasks.message')">
        <template #below>
          <div @focusout="commitPrompt">
            <TextArea v-model="prompt" :rows="4" :aria-label="t('tasks.message')" :placeholder="t('tasks.placeholder')" />
          </div>
        </template>
      </SettingsRow>
      <SettingsRow v-if="zoned" :label="t('trigger.timeZone')">
        <Dropdown :overlay-store="overlayStore" variant="field" size="sm" width="shrink" :trigger-label="t('trigger.timeZone')">
          <template #trigger>{{ automation.timeZone }}</template>
          <template #content="{ close }">
            <Menu>
              <MenuItem
                v-for="zone in timeZones"
                :key="zone"
                :label="zone"
                choice
                :is-selected="automation.timeZone === zone"
                @select="emit('change', { timeZone: zone }); close()"
              />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup>
      <SettingsRow :label="t('target.conversation')">
        <Dropdown :overlay-store="overlayStore" variant="field" size="sm" width="shrink" :trigger-label="t('target.conversation')">
          <template #trigger><span class="min-w-0 truncate">{{ conversationTitle }}</span></template>
          <template #content="{ close }">
            <Menu>
              <MenuItem :label="t('target.newEach')" choice :is-selected="!existing" @select="chooseNew(); close()" />
              <MenuDivider />
              <MenuItem
                v-for="each in recent"
                :key="each.id"
                :label="each.title"
                :value="hostName(each.hostId)"
                choice
                :is-selected="existing?.conversationId === each.id"
                @select="chooseConversation(each.id); close()"
              />
              <MenuDivider />
              <MenuItem :label="t('target.choose')" @select="choosing = true; close()" />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
      <template v-if="automation.target.kind === 'new'">
        <SettingsRow :label="t('target.host')" phone-stacked>
          <Dropdown
            :overlay-store="overlayStore"
            variant="field"
            size="sm"
            width="shrink"
            :trigger-label="t('target.host')"
            :icon="host ? hostGlyph(host) : undefined"
          >
            <template #trigger><span class="min-w-0 truncate">{{ host?.name ?? '' }}</span></template>
            <template #content="{ close }">
              <Menu>
                <MenuItem
                  v-for="each in hosts"
                  :key="each.id"
                  :icon="hostGlyph(each)"
                  :label="each.name"
                  :value="each.state === 'online' ? undefined : t('target.offline')"
                  choice
                  :is-selected="each.id === targetHostId"
                  @select="chooseHost(each.id); close()"
                />
              </Menu>
            </template>
          </Dropdown>
        </SettingsRow>
        <SettingsRow :label="t('target.project')" phone-stacked>
          <Dropdown :overlay-store="overlayStore" variant="field" size="sm" width="shrink" :trigger-label="t('target.project')">
            <template #trigger><span class="min-w-0 truncate">{{ project?.name ?? t('target.noProject') }}</span></template>
            <template #content="{ close }">
              <Menu>
                <MenuItem
                  :label="t('target.noProject')"
                  choice
                  :is-selected="automation.target.projectId === null"
                  @select="automation.target.kind === 'new' && emit('change', { target: { ...automation.target, projectId: null } }); close()"
                />
                <MenuItem
                  v-for="each in host?.projects ?? []"
                  :key="each.id"
                  :label="each.name"
                  choice
                  :is-selected="each.id === automation.target.projectId"
                  @select="automation.target.kind === 'new' && emit('change', { target: { ...automation.target, projectId: each.id } }); close()"
                />
              </Menu>
            </template>
          </Dropdown>
        </SettingsRow>
      </template>
      <SettingsRow v-else :label="t('busy.label')">
        <Dropdown :overlay-store="overlayStore" variant="field" size="sm" width="shrink" :trigger-label="t('busy.label')">
          <template #trigger>{{ busyWays.find((each) => each.value === existing?.ifBusy)?.label }}</template>
          <template #content="{ close }">
            <Menu>
              <MenuItem
                v-for="way in busyWays"
                :key="way.value"
                :label="way.label"
                choice
                :is-selected="existing?.ifBusy === way.value"
                @select="setIfBusy(way.value); close()"
              />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
      <SettingsRow :label="t('model.label')">
        <Dropdown :overlay-store="overlayStore" variant="field" size="sm" width="shrink" :trigger-label="t('model.label')">
          <template #trigger>
            <ModelChoiceTrigger :label="modelLabel" :missing="modelMissing" />
          </template>
          <template #content>
            <ModelMenu
              :providers="providers"
              :models="models"
              :settings="automation.model"
              without-context
              @change="changeModel"
            />
          </template>
        </Dropdown>
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup :title="t('runs.title')">
      <p v-if="automation.runs.length === 0" class="select-none px-4 py-3 text-setting-small text-fg-subtle">{{ t('runs.none') }}</p>
      <!-- Each run is a line of the history; one with a conversation opens it, as a row that goes somewhere does. -->
      <component
        :is="run.conversationId ? 'button' : 'div'"
        v-for="run in automation.runs"
        :key="run.id"
        :type="run.conversationId ? 'button' : undefined"
        class="grid w-full select-none grid-cols-[6.5rem_6rem_minmax(0,1fr)_1.5rem] items-center gap-x-3 px-4 py-2.5 text-left text-setting-small phone:grid-cols-[minmax(0,1fr)_auto_1.5rem] phone:gap-y-1"
        :class="run.conversationId ? 'cursor-default hover:bg-hover focus-visible:bg-hover focus-visible:outline-none' : ''"
        :aria-label="run.conversationId ? `${t('runs.open')}: ${shortMoment(run.at, automation.timeZone)}` : undefined"
        @click="run.conversationId && emit('openRun', run)"
      >
        <span class="whitespace-nowrap tabular-nums text-fg-muted">{{ shortMoment(run.at, automation.timeZone) }}</span>
        <span class="flex items-center gap-1.5 phone:col-start-2 phone:row-start-1">
          <Tag :tone="STATE_TONE[run.state]">{{ t(`runs.status.${run.state}`) }}</Tag>
          <span v-if="run.unread" class="size-1.5 rounded-full bg-accent" :aria-label="t('page.unread')" />
        </span>
        <span class="min-w-0 truncate phone:col-span-2 phone:row-start-2" :class="run.state === 'failed' || run.state === 'skipped' ? 'text-fg-muted' : 'text-fg-body'">
          <TruncatedText :text="runText(run)" />
        </span>
        <ChevronRight v-if="run.conversationId" :size="ICON_PX.in28" class="justify-self-end text-fg-subtle phone:row-span-2 phone:row-start-1 phone:col-start-3 phone:self-center" />
      </component>
    </SettingsGroup>

    <div>
      <Button variant="danger" size="sm" @click="deleteAsked = true">
        <Trash2 :size="ICON_PX.in24" /> {{ t('editor.delete') }}
      </Button>
    </div>

    <AutomationTaskDialog
      v-if="openTask && repeat"
      :is-open="true"
      :task="openTask"
      :unit="repeat.unit"
      :holiday-countries="holidayCountries"
      :overlay-store="overlayStore"
      @change="openTask && changeTask(openTask.id, $event)"
      @close="openTaskId = null"
    />
    <AutomationRepeatDialog
      :is-open="customOpen"
      :repeat="repeat ?? { every: 1, unit: 'weeks', start: null }"
      :today="today()"
      :overlay-store="overlayStore"
      @done="setRepeat($event); customOpen = false"
      @close="customOpen = false"
    />
    <FilterDialog v-model:query="query" :is-open="choosing" :overlay-store="overlayStore" :label="t('target.pick')" @close="choosing = false">
      <div class="settings-card overflow-hidden rounded-xl border border-line bg-surface-float">
        <button
          v-for="each in found"
          :key="each.id"
          type="button"
          class="flex h-10 w-full cursor-default select-none items-center gap-3 px-3 text-left text-setting-small hover:bg-hover phone:h-auto phone:min-h-row phone:py-1.5"
          @click="chooseConversation(each.id)"
        >
          <span class="min-w-0 flex-1 truncate text-fg-body">{{ each.title }}</span>
          <span class="shrink-0 text-fg-muted">{{ hostName(each.hostId) }}</span>
        </button>
      </div>
    </FilterDialog>
    <ConfirmDialog
      :is-open="deleteAsked"
      :overlay-store="overlayStore"
      :title="t('editor.deleteTitle', { name: automation.name })"
      :action="t('editor.deleteAction')"
      :on-confirm="() => { deleteAsked = false; emit('delete') }"
      @close="deleteAsked = false"
    >
      <p>{{ t('editor.deleteBody') }}</p>
    </ConfirmDialog>
    <ConfirmDialog
      :is-open="regenerateAsked"
      :overlay-store="overlayStore"
      :title="t('trigger.regenerateTitle')"
      :action="t('trigger.regenerateAction')"
      :on-confirm="() => { regenerateAsked = false; emit('regenerateSecret') }"
      @close="regenerateAsked = false"
    >
      <p>{{ t('trigger.regenerateBody') }}</p>
    </ConfirmDialog>
  </div>
</template>

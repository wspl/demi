<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { BellOff, ChevronLeft, ChevronRight, Eye, EyeOff, Play, TextCursorInput, Trash2 } from '@lucide/vue'
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
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
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
import { t } from './locales/text'
import {
  WEEKDAYS,
  type Automation,
  type AutomationChange,
  type AutomationConversation,
  type AutomationHost,
  type AutomationRun,
  type AutomationTrigger,
  type RunState,
  type TimeSchedule,
  type Weekday,
} from './types'
import { cronWords, fieldMoment, intervalOf, minutesOf, pausedWords, runLine, shortMoment, weekdayName, zonedMoment } from './words'

/**
 * One automation (`automations.md` § Where the user sees them), every part
 * editable in place as a provider's page is: its name, On or Paused and Run
 * Now in its header; its trigger, target, prompt and model; its runs,
 * newest first, each opening its conversation; and Delete, which asks
 * first. Every edit is one `change` of the parts it changes, which the host
 * saves.
 */
const props = defineProps<{
  automation: Automation
  hosts: readonly AutomationHost[]
  /** The conversations an existing-conversation target can name, with their Hosts. */
  conversations: readonly AutomationConversation[]
  /** The time zones to choose from, the user's own first. */
  timeZones: readonly string[]
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
  runNow: []
  delete: []
  showSecret: []
  hideSecret: []
  regenerateSecret: []
  openRun: [run: AutomationRun]
  turnOnNotifications: []
}>()

const automation = computed(() => props.automation)
const renaming = ref(false)
const deleteAsked = ref(false)
const regenerateAsked = ref(false)
watch(() => props.automation.id, () => {
  renaming.value = false
})

function rename(name: string): void {
  renaming.value = false
  const trimmed = name.trim()
  if (trimmed && trimmed !== props.automation.name) {
    emit('change', { name: trimmed })
  }
}

// The trigger.

const triggerKinds = computed(() => [
  { value: 'time' as const, label: t('trigger.time') },
  { value: 'change' as const, label: t('trigger.change') },
  { value: 'url' as const, label: t('trigger.url') },
])
const scheduleKinds = computed(() => [
  { value: 'once' as const, label: t('trigger.once') },
  { value: 'daily' as const, label: t('trigger.daily') },
  { value: 'every' as const, label: t('trigger.repeating') },
  { value: 'cron' as const, label: t('trigger.cron') },
])
const units = computed(() => (['minutes', 'hours', 'days'] as const).map((value) => ({ value, label: t(`trigger.units.${value}`) })))

/**
 * The trigger of `kind`, keeping what the current one has that the new one
 * uses: a change watch keeps an interval's span.
 */
function triggerOf(kind: AutomationTrigger['kind']): AutomationTrigger {
  const current = props.automation.trigger
  const minutes = current.kind === 'change'
    ? current.everyMinutes
    : current.kind === 'time' && current.schedule.kind === 'every' ? current.schedule.minutes : 10
  switch (kind) {
    case 'time':
      return { kind: 'time', schedule: { kind: 'daily', time: '09:00', days: [...WEEKDAYS] } }
    case 'change':
      return { kind: 'change', command: current.kind === 'change' ? current.command : 'date +%F', everyMinutes: minutes }
    case 'url':
      return { kind: 'url' }
  }
}

function setTriggerKind(kind: AutomationTrigger['kind']): void {
  if (kind !== props.automation.trigger.kind) {
    emit('change', { trigger: triggerOf(kind) })
  }
}


const MOMENT = /^(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2})$/
const CLOCK = /^([01]\d|2[0-3]):[0-5]\d$/

function scheduleOf(kind: TimeSchedule['kind']): TimeSchedule {
  switch (kind) {
    case 'once': {
      // Tomorrow 08:00 by the automation's clocks.
      const today = fieldMoment(new Date().toISOString(), props.automation.timeZone)
      const [year, month, day] = today.slice(0, 10).split('-').map(Number) as [number, number, number]
      const tomorrow = new Date(Date.UTC(year, month - 1, day + 1))
      const at = zonedMoment(tomorrow.getUTCFullYear(), tomorrow.getUTCMonth() + 1, tomorrow.getUTCDate(), 8, 0, props.automation.timeZone)
      return { kind: 'once', at: at.toISOString() }
    }
    case 'daily':
      return { kind: 'daily', time: '09:00', days: [...WEEKDAYS] }
    case 'every':
      return { kind: 'every', minutes: 120 }
    case 'cron':
      return { kind: 'cron', expression: '0 9 * * 1-5' }
  }
}

function setSchedule(schedule: TimeSchedule): void {
  if (props.automation.trigger.kind === 'time') {
    emit('change', { trigger: { kind: 'time', schedule } })
  }
}

/** A daily schedule with `day` chosen or not; the last chosen day stays. */
function toggleDay(day: Weekday): void {
  const trigger = props.automation.trigger
  if (trigger.kind !== 'time' || trigger.schedule.kind !== 'daily') {
    return
  }
  const { time, days } = trigger.schedule
  const next = days.includes(day) ? days.filter((each) => each !== day) : WEEKDAYS.filter((each) => each === day || days.includes(each))
  if (next.length > 0) {
    setSchedule({ kind: 'daily', time, days: next })
  }
}

/** The span between checks or runs, in minutes; none for a trigger without one. */
const minutes = computed<number | null>(() => {
  const trigger = props.automation.trigger
  if (trigger.kind === 'change') {
    return trigger.everyMinutes
  }
  return trigger.kind === 'time' && trigger.schedule.kind === 'every' ? trigger.schedule.minutes : null
})
const interval = computed(() => (minutes.value === null ? null : intervalOf(minutes.value)))

function setMinutes(next: number): void {
  const trigger = props.automation.trigger
  if (trigger.kind === 'change') {
    emit('change', { trigger: { ...trigger, everyMinutes: next } })
  } else if (trigger.kind === 'time' && trigger.schedule.kind === 'every') {
    setSchedule({ kind: 'every', minutes: next })
  }
}

function validCount(value: string): string | null {
  return /^[1-9]\d*$/.test(value.trim()) ? null : t('trigger.intervalInvalid')
}
function validMoment(value: string): string | null {
  return MOMENT.test(value.trim()) ? null : t('trigger.momentInvalid')
}
function validClock(value: string): string | null {
  return CLOCK.test(value.trim()) ? null : t('trigger.clockInvalid')
}
function validCron(value: string): string | null {
  return value.trim().split(/\s+/).length === 5 ? null : t('trigger.expressionInvalid')
}
function validLimit(value: string): string | null {
  return value.trim() === '' || /^[1-9]\d*$/.test(value.trim()) ? null : t('trigger.limitInvalid')
}
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

// The target.

const targetHostId = computed(() => {
  const target = props.automation.target
  return target.kind === 'new' ? target.hostId : props.conversations.find((each) => each.id === target.conversationId)?.hostId
})
const host = computed(() => props.hosts.find((candidate) => candidate.id === targetHostId.value))
const project = computed(() => {
  const target = props.automation.target
  return target.kind === 'new' ? host.value?.projects.find((candidate) => candidate.id === target.projectId) : undefined
})
const hostConversations = computed(() => props.conversations.filter((each) => each.hostId === targetHostId.value))
const existingTitle = computed(() => {
  const target = props.automation.target
  return target.kind === 'existing' ? props.conversations.find((each) => each.id === target.conversationId)?.title ?? t('words.existingGone') : ''
})
/** A change watch on the Cloud wakes it at every check, which the trigger says. */
const watchesCloud = computed(() => props.automation.trigger.kind === 'change' && host.value?.kind === 'cloud')

/** Another Host has other projects and conversations: the target starts over on it, with a new conversation. */
function chooseHost(id: string): void {
  if (id !== targetHostId.value) {
    emit('change', { target: { kind: 'new', hostId: id, projectId: null } })
  }
}

const conversationKinds = computed(() => [
  { value: 'new' as const, label: t('target.new') },
  { value: 'existing' as const, label: t('target.existing') },
])

function setConversationKind(kind: 'new' | 'existing'): void {
  if (kind === props.automation.target.kind) {
    return
  }
  const first = hostConversations.value[0]
  if (kind === 'existing' && first) {
    emit('change', { target: { kind: 'existing', conversationId: first.id } })
  } else if (kind === 'new' && targetHostId.value) {
    emit('change', { target: { kind: 'new', hostId: targetHostId.value, projectId: null } })
  }
}

// The prompt, saved when the field is left.

const prompt = ref(props.automation.prompt)
watch(() => props.automation.prompt, (value) => {
  prompt.value = value
})
function commitPrompt(): void {
  if (prompt.value !== props.automation.prompt) {
    emit('change', { prompt: prompt.value })
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

/** A run's line, with where its conversation went once it left the automation. */
function runText(run: AutomationRun): string {
  const moved = props.automation.target.kind === 'new' && run.conversationId !== null && !run.filed
  const line = runLine(run)
  return moved ? [line, t('runs.moved')].filter(Boolean).join(' · ') : line
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
          <Button size="sm" @click="emit('runNow')">
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

    <SettingsGroup :title="t('trigger.title')">
      <SettingsRow :label="t('trigger.kind')">
        <!-- A pop-up menu, as macOS offers a choice of long names: a segmented control would cut them on a phone. -->
        <Dropdown :overlay-store="overlayStore" variant="field" size="sm" width="shrink" :trigger-label="t('trigger.kind')">
          <template #trigger>{{ triggerKinds.find((kind) => kind.value === automation.trigger.kind)?.label }}</template>
          <template #content="{ close }">
            <Menu>
              <MenuItem
                v-for="kind in triggerKinds"
                :key="kind.value"
                :label="kind.label"
                choice
                :is-selected="automation.trigger.kind === kind.value"
                @select="setTriggerKind(kind.value); close()"
              />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
      <template v-if="automation.trigger.kind === 'time'">
        <SettingsRow :label="t('trigger.repeat')">
          <Segmented
            data-field
            size="sm"
            :model-value="automation.trigger.schedule.kind"
            :options="scheduleKinds"
            @update:model-value="setSchedule(scheduleOf($event))"
          />
        </SettingsRow>
        <SettingsRow
          v-if="automation.trigger.schedule.kind === 'once'"
          :label="t('trigger.moment')"
          :description="problem ?? t('trigger.momentHint')"
        >
          <CommitTextInput
            class="w-44"
            literal
            :model-value="fieldMoment(automation.trigger.schedule.at, automation.timeZone)"
            :validate="validMoment"
            :aria-label="t('trigger.moment')"
            @problem="problem = $event"
            @commit="setOnce"
          />
        </SettingsRow>
        <template v-else-if="automation.trigger.schedule.kind === 'daily'">
          <SettingsRow :label="t('trigger.dailyTime')" :description="problem ?? undefined">
            <CommitTextInput
              class="w-20"
              literal
              :model-value="automation.trigger.schedule.time"
              :validate="validClock"
              :aria-label="t('trigger.dailyTime')"
              @problem="problem = $event"
              @commit="automation.trigger.kind === 'time' && automation.trigger.schedule.kind === 'daily' && setSchedule({ ...automation.trigger.schedule, time: $event.trim() })"
            />
          </SettingsRow>
          <SettingsRow :label="t('trigger.days')">
            <!-- The days are toggles, as Clock's alarms and Reminders' repeat choose them. -->
            <div class="flex flex-wrap gap-1" role="group" :aria-label="t('trigger.days')">
              <Button
                v-for="day in WEEKDAYS"
                :key="day"
                size="sm"
                variant="ghost"
                :pressed="automation.trigger.schedule.days.includes(day)"
                :aria-pressed="automation.trigger.schedule.days.includes(day)"
                :aria-label="weekdayName(day, 'long')"
                class="min-w-10"
                @click="toggleDay(day)"
              >{{ weekdayName(day) }}</Button>
            </div>
          </SettingsRow>
        </template>
        <SettingsRow
          v-else-if="automation.trigger.schedule.kind === 'cron'"
          :label="t('trigger.expression')"
          :description="problem ?? t('trigger.expressionHint', { words: cronWords(automation.trigger.schedule.expression) })"
        >
          <CommitTextInput
            class="w-44"
            literal
            :model-value="automation.trigger.schedule.expression"
            :validate="validCron"
            :aria-label="t('trigger.expression')"
            @problem="problem = $event"
            @commit="setSchedule({ kind: 'cron', expression: $event.trim().split(/\s+/).join(' ') })"
          />
        </SettingsRow>
      </template>
      <SettingsRow v-else-if="automation.trigger.kind === 'change'" :label="t('trigger.command')" :description="t('trigger.commandHint')">
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
      <SettingsRow
        v-if="interval"
        :label="automation.trigger.kind === 'change' ? t('trigger.checkEvery') : t('trigger.interval')"
        :description="problem ?? (watchesCloud ? t('trigger.cloudCost') : undefined)"
      >
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
        <SettingsRow :label="t('trigger.address')" :description="t('trigger.addressHint')">
          <template #below>
            <CopyCode :code="automation.address ?? ''" />
          </template>
        </SettingsRow>
        <!-- The secret stays readable, hidden until Show, as a credential's value is. -->
        <SettingsRow :label="t('trigger.secret')" :description="t('trigger.secretHint')">
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
        <SettingsRow :label="t('trigger.stripe')" :description="t('trigger.stripeHint')">
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
      <SettingsRow
        v-if="automation.trigger.kind !== 'time'"
        :label="t('trigger.limit')"
        :description="problem ?? t('trigger.limitHint')"
      >
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

    <SettingsGroup :title="t('target.title')">
      <SettingsRow :label="t('target.host')">
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
      <!-- An existing conversation has its own place, so only a new one asks for a project. -->
      <SettingsRow v-if="automation.target.kind === 'new'" :label="t('target.project')">
        <Dropdown
          :overlay-store="overlayStore"
          variant="field"
          size="sm"
          width="shrink"
          :trigger-label="t('target.project')"
        >
          <template #trigger>{{ project?.name ?? t('target.noProject') }}</template>
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
      <SettingsRow
        :label="t('target.conversation')"
        :description="automation.target.kind === 'new' ? t('target.newHint') : t('target.existingHint')"
      >
        <template #below>
          <div class="flex flex-wrap items-center gap-2">
            <Segmented
              size="sm"
              :model-value="automation.target.kind"
              :options="conversationKinds"
              :disabled="!hostConversations.length && automation.target.kind === 'new'"
              :disabled-reason="t('target.noConversations', { host: host?.name ?? '' })"
              @update:model-value="setConversationKind"
            />
            <Dropdown
              v-if="automation.target.kind === 'existing'"
              :overlay-store="overlayStore"
              variant="field"
              size="sm"
              width="shrink"
              :trigger-label="t('target.pick')"
            >
              <template #trigger>{{ existingTitle }}</template>
              <template #content="{ close }">
                <Menu>
                  <MenuItem
                    v-for="each in hostConversations"
                    :key="each.id"
                    :label="each.title"
                    choice
                    :is-selected="automation.target.kind === 'existing' && automation.target.conversationId === each.id"
                    @select="emit('change', { target: { kind: 'existing', conversationId: each.id } }); close()"
                  />
                </Menu>
              </template>
            </Dropdown>
          </div>
        </template>
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup :title="t('prompt.title')" :description="t('prompt.hint')">
      <div class="p-3" @focusout="commitPrompt">
        <TextArea v-model="prompt" :rows="4" :placeholder="t('prompt.placeholder')" />
      </div>
    </SettingsGroup>

    <SettingsGroup>
      <SettingsRow :label="t('model.label')" :description="t('model.hint')">
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

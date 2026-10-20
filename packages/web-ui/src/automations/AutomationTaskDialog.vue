<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import SettingsGroup from '../settings/SettingsGroup.vue'
import SettingsRow from '../settings/SettingsRow.vue'
import Button from '../ui/Button.vue'
import CommitTextInput from '../ui/CommitTextInput.vue'
import Dialog from '../ui/Dialog.vue'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuDivider from '../ui/MenuDivider.vue'
import MenuItem from '../ui/MenuItem.vue'
import TextArea from '../ui/TextArea.vue'
import { t } from './locales/text'
import { WEEKDAYS, type AutomationTask, type HolidayCountry, type MonthDay, type RepeatUnit, type Weekday, type WeekOfMonth } from './types'
import { countryName, monthName, taskWords, weekdayName } from './words'

/**
 * A repeat's task, as a sheet over the automation, as Focus's schedule is
 * edited in System Settings: its moment, with what the repeat's unit asks
 * (weekdays for weeks, a day of the month for months, a month and its day
 * for years), the holidays a working day skips, its time and its message,
 * with no explanation under a row. Every edit is one `change` of
 * the parts it changes; the message is saved when its field is left or the
 * sheet closes.
 */
const props = defineProps<{
  isOpen: boolean
  task: AutomationTask
  unit: RepeatUnit
  /** The countries whose holidays a working day can skip. */
  holidayCountries: readonly HolidayCountry[]
  overlayStore: OverlayStore
}>()

const emit = defineEmits<{
  change: [change: Partial<Omit<AutomationTask, 'id'>>]
  close: []
}>()

const CLOCK = /^([01]\d|2[0-3]):[0-5]\d$/
const DATE = /^([1-9]|[12]\d|3[01])$/
const problem = ref<string | null>(null)

function validClock(value: string): string | null {
  return CLOCK.test(value.trim()) ? null : t('trigger.clockInvalid')
}
function validDate(value: string): string | null {
  return DATE.test(value.trim()) ? null : t('tasks.dateInvalid')
}

/** The task with `day` chosen or not; the last chosen day stays. */
function toggleDay(day: Weekday): void {
  const days = props.task.days
  const next = days.includes(day) ? days.filter((each) => each !== day) : WEEKDAYS.filter((each) => each === day || days.includes(each))
  if (next.length > 0) {
    emit('change', { days: next })
  }
}

/** A day of the month's kinds, as the Day menu lists them: a date, the last day, a weekday of the month, a working day. */
type DayChoice = 'date' | 'lastDay' | WeekOfMonth | 'firstWorking' | 'lastWorking'
const WEEKS: readonly WeekOfMonth[] = ['first', 'second', 'third', 'fourth', 'last']

const dayChoice = computed<DayChoice>(() => {
  const day = props.task.day
  switch (day.kind) {
    case 'date':
      return 'date'
    case 'last_day':
      return 'lastDay'
    case 'weekday':
      return day.week
    case 'working_day':
      return day.which === 'first' ? 'firstWorking' : 'lastWorking'
  }
})

/** The day of `choice`, keeping the date or the weekday the current day has. */
function dayOf(choice: DayChoice): MonthDay {
  const current = props.task.day
  switch (choice) {
    case 'date':
      return { kind: 'date', date: current.kind === 'date' ? current.date : 1 }
    case 'lastDay':
      return { kind: 'last_day' }
    case 'firstWorking':
      return { kind: 'working_day', which: 'first' }
    case 'lastWorking':
      return { kind: 'working_day', which: 'last' }
    default:
      return { kind: 'weekday', week: choice, weekday: current.kind === 'weekday' ? current.weekday : 'mon' }
  }
}

const dayLabel = computed(() => t(`tasks.dayKinds.${dayChoice.value}`))
const MONTHS = Array.from({ length: 12 }, (_, index) => index + 1)

// The message, saved when its field is left or the sheet closes.

const prompt = ref(props.task.prompt)
/** The message as last saved, which the field showed or the user started from. */
let saved = props.task.prompt
// Another task shows its message; a new saved message of this one shows
// unless the user is writing another, whose draft stays until it is saved.
watch(() => props.task.id, () => {
  prompt.value = props.task.prompt
  saved = props.task.prompt
})
watch(() => props.task.prompt, (value) => {
  if (prompt.value === saved) {
    prompt.value = value
  }
  saved = value
})
function commitPrompt(): void {
  if (prompt.value !== props.task.prompt) {
    emit('change', { prompt: prompt.value })
  }
}
function close(): void {
  commitPrompt()
  problem.value = null
  emit('close')
}

// The holidays a first or last working day skips.

const countries = computed(() => [...props.holidayCountries]
  .map((each) => ({ ...each, name: countryName(each.code) }))
  .sort((a, b) => a.name.localeCompare(b.name)))
const country = computed(() => countries.value.find((each) => each.code === props.task.holidays?.country) ?? null)
const region = computed(() => country.value?.regions.find((each) => each.code === props.task.holidays?.region) ?? null)
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" size="wide" :label="taskWords(task, unit)" @close="close">
    <div class="flex flex-col gap-5 px-5 pt-5">
      <h3 class="select-none pr-8 text-[15px] font-medium text-fg-emphasis">{{ taskWords(task, unit) }}</h3>
      <SettingsGroup>
        <SettingsRow v-if="unit === 'weeks'" :label="t('tasks.days')">
          <!-- The days are toggles, as Clock's alarms and Reminders' repeat choose them. -->
          <div class="flex flex-wrap gap-1" role="group" :aria-label="t('tasks.days')">
            <Button
              v-for="day in WEEKDAYS"
              :key="day"
              size="sm"
              variant="ghost"
              :pressed="task.days.includes(day)"
              :aria-pressed="task.days.includes(day)"
              :aria-label="weekdayName(day, 'long')"
              class="min-w-10"
              @click="toggleDay(day)"
            >{{ weekdayName(day) }}</Button>
          </div>
        </SettingsRow>
        <SettingsRow v-if="unit === 'years'" :label="t('tasks.month')">
          <Dropdown :overlay-store="overlayStore" variant="field" size="sm" width="shrink" :trigger-label="t('tasks.month')">
            <template #trigger>{{ monthName(task.month, 'long') }}</template>
            <template #content="{ close: closeMenu }">
              <Menu>
                <MenuItem
                  v-for="month in MONTHS"
                  :key="month"
                  :label="monthName(month, 'long')"
                  choice
                  :is-selected="task.month === month"
                  @select="emit('change', { month }); closeMenu()"
                />
              </Menu>
            </template>
          </Dropdown>
        </SettingsRow>
        <SettingsRow
          v-if="unit === 'months' || unit === 'years'"
          :label="t('tasks.day')"
          :description="problem ?? undefined"
        >
          <!-- One menu of every kind of day, as Calendar's custom monthly repeat offers them; a date or a weekday then asks which. -->
          <div class="flex flex-wrap items-center gap-2">
            <Dropdown :overlay-store="overlayStore" variant="field" size="sm" width="shrink" :trigger-label="t('tasks.day')">
              <template #trigger>{{ dayLabel }}</template>
              <template #content="{ close: closeMenu }">
                <Menu>
                  <MenuItem :label="t('tasks.dayKinds.date')" choice :is-selected="dayChoice === 'date'" @select="emit('change', { day: dayOf('date') }); closeMenu()" />
                  <MenuItem :label="t('tasks.dayKinds.lastDay')" choice :is-selected="dayChoice === 'lastDay'" @select="emit('change', { day: dayOf('lastDay') }); closeMenu()" />
                  <MenuDivider />
                  <MenuItem
                    v-for="week in WEEKS"
                    :key="week"
                    :label="t(`tasks.dayKinds.${week}`)"
                    choice
                    :is-selected="dayChoice === week"
                    @select="emit('change', { day: dayOf(week) }); closeMenu()"
                  />
                  <MenuDivider />
                  <MenuItem :label="t('tasks.dayKinds.firstWorking')" choice :is-selected="dayChoice === 'firstWorking'" @select="emit('change', { day: dayOf('firstWorking') }); closeMenu()" />
                  <MenuItem :label="t('tasks.dayKinds.lastWorking')" choice :is-selected="dayChoice === 'lastWorking'" @select="emit('change', { day: dayOf('lastWorking') }); closeMenu()" />
                </Menu>
              </template>
            </Dropdown>
            <CommitTextInput
              v-if="task.day.kind === 'date'"
              class="w-14"
              literal
              :model-value="String(task.day.date)"
              :validate="validDate"
              :aria-label="t('tasks.date')"
              @problem="problem = $event"
              @commit="emit('change', { day: { kind: 'date', date: Number($event.trim()) } })"
            />
            <Dropdown
              v-else-if="task.day.kind === 'weekday'"
              :overlay-store="overlayStore"
              variant="field"
              size="sm"
              width="shrink"
              :trigger-label="t('tasks.weekday')"
            >
              <template #trigger>{{ weekdayName(task.day.weekday, 'long') }}</template>
              <template #content="{ close: closeMenu }">
                <Menu>
                  <MenuItem
                    v-for="weekday in WEEKDAYS"
                    :key="weekday"
                    :label="weekdayName(weekday, 'long')"
                    choice
                    :is-selected="task.day.kind === 'weekday' && task.day.weekday === weekday"
                    @select="task.day.kind === 'weekday' && emit('change', { day: { ...task.day, weekday } }); closeMenu()"
                  />
                </Menu>
              </template>
            </Dropdown>
          </div>
        </SettingsRow>
        <SettingsRow v-if="(unit === 'months' || unit === 'years') && task.day.kind === 'working_day'" :label="t('holidays.label')">
          <div class="flex flex-wrap items-center justify-end gap-2">
            <Dropdown :overlay-store="overlayStore" variant="field" size="sm" width="shrink" :trigger-label="t('holidays.label')">
              <template #trigger>{{ country?.name ?? t('holidays.none') }}</template>
              <template #content="{ close: closeMenu }">
                <Menu>
                  <MenuItem :label="t('holidays.none')" choice :is-selected="task.holidays === null" @select="emit('change', { holidays: null }); closeMenu()" />
                  <MenuDivider />
                  <MenuItem
                    v-for="each in countries"
                    :key="each.code"
                    :label="each.name"
                    choice
                    :is-selected="each.code === task.holidays?.country"
                    @select="each.code !== task.holidays?.country && emit('change', { holidays: { country: each.code, region: null } }); closeMenu()"
                  />
                </Menu>
              </template>
            </Dropdown>
            <Dropdown
              v-if="country && country.regions.length"
              :overlay-store="overlayStore"
              variant="field"
              size="sm"
              width="shrink"
              :trigger-label="t('holidays.region')"
            >
              <template #trigger>{{ region?.name ?? t('holidays.allRegions') }}</template>
              <template #content="{ close: closeMenu }">
                <Menu>
                  <MenuItem
                    :label="t('holidays.allRegions')"
                    choice
                    :is-selected="!task.holidays?.region"
                    @select="emit('change', { holidays: { country: country.code, region: null } }); closeMenu()"
                  />
                  <MenuDivider />
                  <MenuItem
                    v-for="each in country.regions"
                    :key="each.code"
                    :label="each.name"
                    choice
                    :is-selected="each.code === task.holidays?.region"
                    @select="emit('change', { holidays: { country: country.code, region: each.code } }); closeMenu()"
                  />
                </Menu>
              </template>
            </Dropdown>
          </div>
        </SettingsRow>
        <SettingsRow :label="t('tasks.time')" :description="unit === 'months' || unit === 'years' ? undefined : problem ?? undefined">
          <CommitTextInput
            class="w-20"
            literal
            :model-value="task.time"
            :validate="validClock"
            :aria-label="t('tasks.time')"
            @problem="problem = $event"
            @commit="emit('change', { time: $event.trim() })"
          />
        </SettingsRow>
        <SettingsRow :label="t('tasks.message')">
          <template #below>
            <div @focusout="commitPrompt">
              <TextArea v-model="prompt" :rows="5" :aria-label="t('tasks.message')" :placeholder="t('tasks.placeholder')" />
            </div>
          </template>
        </SettingsRow>
      </SettingsGroup>
    </div>
    <template #footer>
      <Button variant="primary" @click="close">{{ t('tasks.done') }}</Button>
    </template>
  </Dialog>
</template>

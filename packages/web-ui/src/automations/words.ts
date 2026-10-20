import { failureHeadline } from '../agent/error-detail'
import { formatPast } from '../ui/dates'
import { t } from './locales/text'
import {
  WEEKDAYS,
  type Automation,
  type AutomationConversation,
  type AutomationHost,
  type AutomationRun,
  type AutomationTask,
  type MonthDay,
  type PauseReason,
  type Repeat,
  type RepeatUnit,
  type RunNote,
  type RunState,
  type TimeSchedule,
  type Weekday,
} from './types'

/** A clock time as the design writes it: 09:00. */
function clock(hour: number, minute: number): string {
  return `${String(hour).padStart(2, '0')}:${String(minute).padStart(2, '0')}`
}

/** The clock reading of `moment` in `zone`, the browser's own without one. */
function wallClock(moment: Date, zone?: string): { year: number; month: number; day: number; hour: number; minute: number } {
  const parts = new Intl.DateTimeFormat('en-US', {
    timeZone: zone,
    hourCycle: 'h23',
    year: 'numeric',
    month: 'numeric',
    day: 'numeric',
    hour: 'numeric',
    minute: 'numeric',
  }).formatToParts(moment)
  const part = (type: Intl.DateTimeFormatPartTypes) => Number(parts.find((each) => each.type === type)?.value)
  return { year: part('year'), month: part('month'), day: part('day'), hour: part('hour'), minute: part('minute') }
}

/**
 * A moment to the minute, short: Oct 17 09:00, as the clocks of `zone`
 * read it, an automation's times in the automation's zone; the browser's zone
 * without one.
 */
export function shortMoment(iso: string, zone?: string): string {
  const moment = new Date(iso)
  const day = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', timeZone: zone }).format(moment)
  const { hour, minute } = wallClock(moment, zone)
  return `${day} ${clock(hour, minute)}`
}

/** A moment as the Once field shows it in `zone`: 2026-10-18 08:00. */
export function fieldMoment(iso: string, zone: string): string {
  const { year, month, day, hour, minute } = wallClock(new Date(iso), zone)
  const pad = (value: number) => String(value).padStart(2, '0')
  return `${year}-${pad(month)}-${pad(day)} ${clock(hour, minute)}`
}

/**
 * The moment `zone`'s clocks show the date and time at; a time the clocks
 * skip is the first minute after the gap, and a repeated one its first
 * occurrence, as the scheduler reads them.
 */
export function zonedMoment(year: number, month: number, day: number, hour: number, minute: number, zone: string): Date {
  const wall = Date.UTC(year, month - 1, day, hour, minute)
  /** How far `zone`'s clocks are ahead of UTC at `at`. */
  const offset = (at: number) => {
    const read = wallClock(new Date(at), zone)
    return Date.UTC(read.year, read.month - 1, read.day, read.hour, read.minute) - Math.floor(at / 60_000) * 60_000
  }
  // The offsets a day before and after hold every reading of the time.
  const candidates = [offset(wall - 86_400_000), offset(wall + 86_400_000)]
    .map((each) => wall - each)
    .filter((at) => offset(at) === wall - at)
    .sort((a, b) => a - b)
  if (candidates.length > 0) {
    return new Date(candidates[0]!)
  }
  // In a gap: read with the later offset, the time falls before the
  // change, and with the earlier one after it; the change, found to the
  // minute between them, is the first minute after the gap.
  const before = offset(wall - 86_400_000)
  let low = wall - offset(wall + 86_400_000)
  let high = wall - before
  while (high - low > 60_000) {
    const middle = low + Math.floor((high - low) / 120_000) * 60_000
    if (offset(middle) === before) {
      low = middle
    } else {
      high = middle
    }
  }
  return new Date(high)
}

/** An interval in minutes as the editor shows it: the largest whole unit. */
export function intervalOf(minutes: number): { count: number; unit: 'minutes' | 'hours' | 'days' } {
  if (minutes % 1440 === 0) {
    return { count: minutes / 1440, unit: 'days' }
  }
  if (minutes % 60 === 0) {
    return { count: minutes / 60, unit: 'hours' }
  }
  return { count: minutes, unit: 'minutes' }
}

/** The minutes of a count of a unit. */
export function minutesOf(count: number, unit: 'minutes' | 'hours' | 'days'): number {
  return count * { minutes: 1, hours: 60, days: 1440 }[unit]
}

/** An interval in words: Every 10 min, Every 2 hours. */
export function intervalWords(minutes: number): string {
  const { count, unit } = intervalOf(minutes)
  return t(`words.interval.${unit}`, { count }, count)
}

/** A weekday's short name in the page's language: Mon. */
export function weekdayName(day: Weekday, width: 'short' | 'long' = 'short'): string {
  // 2023-01-02 was a Monday.
  return new Intl.DateTimeFormat(undefined, { weekday: width }).format(new Date(2023, 0, 2 + WEEKDAYS.indexOf(day)))
}

/** Weekdays and a time in words: Every day 09:00, Weekdays 09:00, Mon 10:00, Mon, Thu 09:00. */
export function weekdaysWords(time: string, days: readonly Weekday[]): string {
  const chosen = WEEKDAYS.filter((day) => days.includes(day))
  if (chosen.length === 7) {
    return t('words.daily', { time })
  }
  if (chosen.length === 5 && !chosen.includes('sat') && !chosen.includes('sun')) {
    return t('words.weekdays', { time })
  }
  return t('words.days', { days: chosen.map((day) => weekdayName(day)).join(', '), time })
}

/** A month's short name in the page's language: Mar. */
export function monthName(month: number, width: 'short' | 'long' = 'short'): string {
  return new Intl.DateTimeFormat(undefined, { month: width }).format(new Date(2023, month - 1, 1))
}

/** A country's name in the page's language, from its ISO 3166 code: China. */
export function countryName(code: string): string {
  return new Intl.DisplayNames(undefined, { type: 'region' }).of(code) ?? code
}

/** The menu's choices of a repeat, each a cycle Calendar offers; any other is Custom. */
export const REPEAT_PRESETS = [
  { key: 'day', every: 1, unit: 'days' },
  { key: 'week', every: 1, unit: 'weeks' },
  { key: 'twoWeeks', every: 2, unit: 'weeks' },
  { key: 'month', every: 1, unit: 'months' },
  { key: 'threeMonths', every: 3, unit: 'months' },
  { key: 'year', every: 1, unit: 'years' },
] as const satisfies readonly { key: string; every: number; unit: RepeatUnit }[]

/** The preset a repeat is, by its key; none for a custom one. */
export function repeatPreset(repeat: Repeat): (typeof REPEAT_PRESETS)[number]['key'] | null {
  return REPEAT_PRESETS.find((preset) => preset.every === repeat.every && preset.unit === repeat.unit)?.key ?? null
}

/** A repeat in words, as a sentence starts: Every week, Every 3 months. */
export function repeatWords(repeat: Repeat): string {
  return t(`words.repeat.${repeat.unit}`, { count: repeat.every }, repeat.every)
}

/** A day of a month in words: Day 15, Last day, Second Tuesday, Last working day. */
export function monthDayWords(day: MonthDay): string {
  switch (day.kind) {
    case 'date':
      return t('words.date', { date: day.date })
    case 'last_day':
      return t('words.lastDay')
    case 'weekday':
      return t(`words.week.${day.week}`, { weekday: weekdayName(day.weekday, 'long') })
    case 'working_day':
      return t(`words.workingDay.${day.which}`)
  }
}

/** A task's moment in words, as the repeat's unit reads it: 09:00, Mon 10:00, Last working day 17:00, Mar 15 09:00. */
export function taskWords(task: AutomationTask, unit: RepeatUnit): string {
  switch (unit) {
    case 'days':
      return task.time
    case 'weeks':
      return weekdaysWords(task.time, task.days)
    case 'months':
      return t('words.moment', { day: monthDayWords(task.day), time: task.time })
    case 'years': {
      const day = task.day.kind === 'date'
        ? t('words.monthDate', { month: monthName(task.month), date: task.day.date })
        : t('words.dayOfMonth', { day: monthDayWords(task.day), month: monthName(task.month) })
      return t('words.moment', { day, time: task.time })
    }
  }
}

/**
 * A repeat in words, as the list shows it: one task every day or every week
 * says its moment alone (Weekdays 09:00); one task in another cycle says
 * both (Every month · Last working day 17:00); several tasks, their count.
 */
function repeatScheduleWords(repeat: Repeat, tasks: readonly AutomationTask[]): string {
  const only = tasks.length === 1 ? tasks[0]! : null
  if (only && repeat.every === 1 && (repeat.unit === 'days' || repeat.unit === 'weeks')) {
    if (repeat.unit === 'days') {
      return t('words.daily', { time: only.time })
    }
    return only.days.length === 1
      ? t('words.weekly', { day: weekdayName(only.days[0]!, 'long'), time: only.time })
      : weekdaysWords(only.time, only.days)
  }
  const what = only ? taskWords(only, repeat.unit) : t('words.tasks', { count: tasks.length }, tasks.length)
  return `${repeatWords(repeat)} · ${what}`
}

/**
 * A cron expression in words where it is one of the common shapes, at a
 * time every day, on weekdays or on one weekday; any other reads as itself.
 */
export function cronWords(expression: string): string {
  const fields = expression.trim().split(/\s+/)
  if (fields.length !== 5) {
    return expression
  }
  const [minute, hour, dayOfMonth, month, weekday] = fields as [string, string, string, string, string]
  if (!/^\d+$/.test(minute) || !/^\d+$/.test(hour) || dayOfMonth !== '*' || month !== '*') {
    return expression
  }
  const time = clock(Number(hour), Number(minute))
  if (weekday === '*') {
    return t('words.daily', { time })
  }
  if (weekday === '1-5') {
    return t('words.weekdays', { time })
  }
  if (/^[0-6]$/.test(weekday)) {
    // Cron's weekday 0 is Sunday, the last of WEEKDAYS.
    return t('words.weekly', { day: weekdayName(WEEKDAYS[(Number(weekday) + 6) % 7]!, 'long'), time })
  }
  return expression
}

/** A time schedule in words, its moment in its zone `zone`. */
export function scheduleWords(schedule: TimeSchedule, zone?: string): string {
  switch (schedule.kind) {
    case 'once':
      return shortMoment(schedule.at, zone)
    case 'repeat':
      return repeatScheduleWords(schedule.repeat, schedule.tasks)
    case 'every':
      return intervalWords(schedule.minutes)
    case 'cron':
      return cronWords(schedule.expression)
  }
}

/** The trigger in words, as the list shows it. */
export function triggerWords(automation: Pick<Automation, 'trigger' | 'timeZone'>): string {
  const trigger = automation.trigger
  switch (trigger.kind) {
    case 'time':
      return scheduleWords(trigger.schedule, automation.timeZone)
    case 'change':
      return t('words.onChange', { interval: intervalWords(trigger.everyMinutes) })
    case 'url':
      return t('words.url')
  }
}

/**
 * The target in words, as the list shows it: a new conversation with its
 * Host and project, or the existing conversation's title alone, which
 * names the place already.
 */
export function targetWords(
  automation: Automation,
  hosts: readonly AutomationHost[],
  conversations: readonly AutomationConversation[],
): string {
  const target = automation.target
  if (target.kind === 'existing') {
    const title = conversations.find((candidate) => candidate.id === target.conversationId)?.title
    return title ? t('words.existing', { title }) : t('words.existingGone')
  }
  const host = hosts.find((candidate) => candidate.id === target.hostId)
  const project = host?.projects.find((candidate) => candidate.id === target.projectId)
  return [t('words.newConversation'), host?.name ?? '', project?.name ?? ''].filter(Boolean).join(' · ')
}

/** A run's state in words. */
export function stateWords(state: RunState): string {
  return t(`runs.status.${state}`)
}

/** Why a run was skipped or failed, in words: MacBook Pro was offline. */
export function noteWords(note: RunNote): string {
  switch (note.kind) {
    case 'offline':
      return t('runs.note.offline', { device: note.device })
    case 'limit':
      return t('runs.note.limit', { limit: note.limit, calls: note.calls }, note.calls)
    case 'target_gone':
      return t('runs.note.targetGone')
    case 'no_model':
      return t('runs.note.noModel')
    case 'command_timed_out':
      return t('runs.note.timedOut')
    case 'command_failed':
      return t('runs.note.commandFailed', { message: note.message })
    // A turn's failure reads as the transcript words it; a run that could not start its turn says why.
    case 'turn_failed':
      return note.code ? failureHeadline(note.code, null, new Date()) : note.message
  }
}

/** A run's line: the start of the agent's last answer, or why it was skipped or failed. */
export function runLine(run: AutomationRun): string {
  return run.note ? noteWords(run.note) : run.line ?? ''
}

/** Why an automation paused itself, in words. */
export function pausedWords(reason: PauseReason | null): string {
  return reason ? t(`editor.pausedBecause.${reason}`) : t('editor.pausedHint')
}

/** The last run in words against `now`: Notified 2 hours ago; Never run. */
export function lastRunWords(run: AutomationRun | undefined, now: Date | number): string {
  if (!run) {
    return t('runs.never')
  }
  return t('runs.last', { status: stateWords(run.state), when: formatPast(run.at, now) })
}
